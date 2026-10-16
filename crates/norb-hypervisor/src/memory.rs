//! Error type and page-aligned guest memory allocation.
//!
//! [`GuestMemory`] wraps a `mmap`-allocated host buffer that is later mapped
//! into the guest IPA space via [`crate::vm::Vm::map_memory`].  It owns the
//! mapping and unmaps it via `munmap` on drop.

use std::ffi::c_void;
use tracing::{debug, warn};

// ── Error type ────────────────────────────────────────────────────────────────

/// All errors that can be returned by the `norb-hypervisor` crate.
#[derive(Debug, thiserror::Error)]
pub enum HvError {
    /// The `Hypervisor.framework` returned a non-zero status code.
    ///
    /// Common codes (from `<Hypervisor/hv_error.h>`):
    /// * `0xfae94001` — `HV_ERROR` (general failure)
    /// * `0xfae94002` — `HV_BUSY` (VM already exists)
    /// * `0xfae94003` — `HV_BAD_ARGUMENT`
    /// * `0xfae94005` — `HV_NO_RESOURCES`
    /// * `0xfae94006` — `HV_NO_DEVICE` (unsupported hardware / entitlement missing)
    /// * `0xfae94007` — `HV_DENIED` (missing entitlement)
    #[error("Hypervisor.framework error: {0:#x}")]
    Framework(u64),

    /// A host memory operation (mmap / munmap / pointer arithmetic) failed.
    #[error("guest memory error: {0}")]
    Memory(String),

    /// A vCPU register access failed.
    #[error("register error: {0}")]
    Register(String),

    /// The operation is not supported on this platform (non-macOS build).
    #[error("not supported on this platform")]
    NotSupported,
}

// ── Page-aligned guest memory ─────────────────────────────────────────────────

/// Page-aligned host memory that can be mapped into the guest IPA space.
///
/// Memory is allocated with `mmap(MAP_ANON | MAP_PRIVATE)` to guarantee
/// 4 KiB page alignment (required by `hv_vm_map`).  On drop the mapping is
/// released with `munmap`.
///
/// # Thread safety
/// The underlying buffer pointer is stable for the lifetime of this struct.
/// Safe access to the contents requires external synchronisation or using the
/// safe helper methods.
pub struct GuestMemory {
    /// Host virtual address of the mapping.
    host_ptr: *mut u8,
    /// Size of the mapping in bytes (multiple of the host page size).
    size: usize,
    /// Guest Intermediate Physical Address this memory will be (or is) mapped to.
    guest_base: u64,
}

// SAFETY: The raw pointer owns the mapping; we are responsible for
// synchronising access, which callers do via &mut references or unsafe slices.
unsafe impl Send for GuestMemory {}
unsafe impl Sync for GuestMemory {}

impl GuestMemory {
    /// Allocate `size` bytes of page-aligned anonymous memory.
    ///
    /// `size` is rounded up to the next 4 KiB page boundary.
    /// `guest_base` is the guest IPA that this buffer will eventually be
    /// mapped to; it is stored for convenience and is not validated here.
    ///
    /// # Errors
    /// Returns [`HvError::Memory`] if `mmap` fails or `size` is zero.
    #[cfg(target_os = "macos")]
    pub fn allocate(size: usize, guest_base: u64) -> Result<Self, HvError> {
        if size == 0 {
            return Err(HvError::Memory("size must be non-zero".into()));
        }

        // Round up to 4 KiB page granule.
        const PAGE_SIZE: usize = 4096;
        let aligned_size = size.checked_add(PAGE_SIZE - 1).ok_or_else(|| {
            HvError::Memory("size overflow when aligning to page boundary".into())
        })? & !(PAGE_SIZE - 1);

        // SAFETY:
        // * NULL hint — kernel chooses the address.
        // * PROT_READ | PROT_WRITE — required for guest payload writes.
        // * MAP_PRIVATE | MAP_ANON — no backing file; private to this process.
        // * fd = -1, offset = 0 — required by MAP_ANON.
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                aligned_size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANON,
                -1,
                0,
            )
        };

        if ptr == libc::MAP_FAILED {
            let err = std::io::Error::last_os_error();
            return Err(HvError::Memory(format!("mmap failed: {err}")));
        }

        debug!(
            host_ptr = ?ptr,
            size = aligned_size,
            guest_base,
            "allocated guest memory"
        );

        Ok(Self {
            host_ptr: ptr.cast::<u8>(),
            size: aligned_size,
            guest_base,
        })
    }

    /// Stub for non-macOS targets so the crate can be syntax-checked.
    #[cfg(not(target_os = "macos"))]
    pub fn allocate(_size: usize, _guest_base: u64) -> Result<Self, HvError> {
        Err(HvError::NotSupported)
    }

    /// Returns the host virtual address of the mapping.
    ///
    /// This pointer is suitable for passing to
    /// [`Vm::map_memory`](crate::vm::Vm::map_memory).
    pub fn host_ptr(&self) -> *mut u8 {
        self.host_ptr
    }

    /// Returns the guest IPA this buffer is intended to back.
    pub fn guest_base(&self) -> u64 {
        self.guest_base
    }

    /// Returns the actual (page-aligned) size of the allocation in bytes.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Write `data` bytes to the guest memory at `guest_addr`.
    ///
    /// `guest_addr` must be within `[guest_base, guest_base + size)`.
    ///
    /// # Errors
    /// Returns [`HvError::Memory`] if the range falls outside the allocation.
    pub fn write_at(&mut self, guest_addr: u64, data: &[u8]) -> Result<(), HvError> {
        let offset = self.offset_of(guest_addr, data.len())?;
        // SAFETY: offset is validated to be within [0, self.size); host_ptr
        // is valid for self.size bytes.
        let dst = unsafe { std::slice::from_raw_parts_mut(self.host_ptr.add(offset), data.len()) };
        dst.copy_from_slice(data);
        Ok(())
    }

    /// Read `len` bytes from the guest memory at `guest_addr`.
    ///
    /// # Errors
    /// Returns [`HvError::Memory`] if the range falls outside the allocation.
    pub fn read_at(&self, guest_addr: u64, len: usize) -> Result<Vec<u8>, HvError> {
        let offset = self.offset_of(guest_addr, len)?;
        // SAFETY: same as write_at.
        let src = unsafe { std::slice::from_raw_parts(self.host_ptr.add(offset), len) };
        Ok(src.to_vec())
    }

    /// Return a mutable byte slice for a guest physical address range.
    ///
    /// # Safety
    /// The caller must ensure:
    /// * No other mutable references to overlapping regions exist.
    /// * The slice is not used after this `GuestMemory` is dropped.
    ///
    /// # Errors
    /// Returns [`HvError::Memory`] if the range falls outside the allocation.
    pub unsafe fn slice_at_mut(
        &mut self,
        guest_addr: u64,
        len: usize,
    ) -> Result<&mut [u8], HvError> {
        let offset = self.offset_of(guest_addr, len)?;
        // SAFETY: Invariant documented on the function; we validate bounds above.
        Ok(unsafe { std::slice::from_raw_parts_mut(self.host_ptr.add(offset), len) })
    }

    /// Return a mutable slice of the entire guest memory allocation.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.host_ptr, self.size) }
    }

    /// Return an immutable slice of the entire guest memory allocation.
    pub fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.host_ptr, self.size) }
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    /// Convert a `guest_addr` into a host buffer offset, checking bounds.
    fn offset_of(&self, guest_addr: u64, len: usize) -> Result<usize, HvError> {
        if guest_addr < self.guest_base {
            return Err(HvError::Memory(format!(
                "guest_addr {guest_addr:#x} is below guest_base {:#x}",
                self.guest_base
            )));
        }
        let offset = (guest_addr - self.guest_base) as usize;
        let end = offset.checked_add(len).ok_or_else(|| {
            HvError::Memory(format!(
                "guest_addr {guest_addr:#x} + len {len} overflows"
            ))
        })?;
        if end > self.size {
            return Err(HvError::Memory(format!(
                "range [{guest_addr:#x}, {guest_addr:#x}+{len}) exceeds allocation \
                 [{:#x}, {:#x})",
                self.guest_base,
                self.guest_base + self.size as u64,
            )));
        }
        Ok(offset)
    }
}

impl Drop for GuestMemory {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        {
            // SAFETY: host_ptr and size were set by mmap and have not been
            // altered since construction.  No other owner of this mapping
            // exists because GuestMemory is the sole owner.
            let ret = unsafe { libc::munmap(self.host_ptr.cast::<c_void>(), self.size) };
            if ret != 0 {
                let err = std::io::Error::last_os_error();
                warn!(%err, host_ptr = ?self.host_ptr, size = self.size, "munmap failed");
            } else {
                debug!(host_ptr = ?self.host_ptr, size = self.size, "released guest memory");
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            // Nothing to do on non-macOS; allocate() always fails there.
            let _ = c_void::default; // suppress unused import lint
        }
    }
}
