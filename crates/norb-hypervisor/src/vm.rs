//! Safe wrapper for the hypervisor VM context.
//!
//! There can be **at most one `Vm` per process** — this is a hard constraint
//! imposed by `Hypervisor.framework`.  Creating a second `Vm` before dropping
//! the first will return [`HvError::Framework`] with code `HV_BUSY`
//! (`0xfae94002`).
//!
//! The VM is automatically destroyed when the [`Vm`] is dropped.

use std::ffi::c_void;

use tracing::{debug, error, instrument};

use crate::ffi::{HV_MEMORY_EXEC, HV_MEMORY_READ, HV_MEMORY_WRITE, HV_SUCCESS};
use crate::memory::HvError;

// ── Vm ────────────────────────────────────────────────────────────────────────

/// The hypervisor virtual machine context.
///
/// Wraps `hv_vm_create` / `hv_vm_destroy`.  Only one `Vm` may exist per
/// process at a time.
///
/// # Drop behaviour
/// Calling `drop` invokes `hv_vm_destroy`.  Any vCPUs must be destroyed
/// **before** the `Vm` is dropped, or `hv_vm_destroy` will return an error
/// (which is logged as a warning but otherwise ignored to avoid panicking in
/// destructors).
pub struct Vm {
    /// Zero-sized private field prevents construction outside this module.
    _private: (),
}

impl Vm {
    /// Create a new hypervisor VM for the current process.
    ///
    /// Equivalent to `hv_vm_create(NULL)`.
    ///
    /// # Errors
    /// * [`HvError::NotSupported`] — not running on macOS.
    /// * [`HvError::Framework`] — framework returned a non-zero code
    ///   (e.g. `HV_BUSY` if a VM already exists, `HV_DENIED` if the
    ///   `com.apple.security.hypervisor` entitlement is missing).
    #[instrument(name = "Vm::new")]
    pub fn new() -> Result<Self, HvError> {
        #[cfg(target_os = "macos")]
        {
            // SAFETY:
            // * NULL config selects framework defaults — always valid.
            // * We check the return value before constructing Self.
            let ret = unsafe { crate::ffi::hv_vm_create(std::ptr::null_mut()) };
            if ret != HV_SUCCESS {
                error!(ret = ret, "hv_vm_create failed");
                return Err(HvError::Framework(ret));
            }
            debug!("VM created");
            Ok(Self { _private: () })
        }

        #[cfg(not(target_os = "macos"))]
        {
            Err(HvError::NotSupported)
        }
    }

    /// Map `size` bytes of host memory at `host_ptr` into the guest at
    /// `guest_addr`.
    ///
    /// The memory at `host_ptr` must:
    /// * Be page-aligned (use [`GuestMemory`](crate::GuestMemory) to guarantee
    ///   this).
    /// * Remain valid (not freed / remapped) for the entire time the mapping
    ///   is active.
    ///
    /// # Errors
    /// [`HvError::Framework`] if the framework rejects the mapping (e.g.
    /// overlap with an existing region, bad alignment, or invalid permissions).
    #[instrument(skip(self, host_ptr), fields(guest_addr, size, readable, writable, executable))]
    pub fn map_memory(
        &self,
        host_ptr: *mut u8,
        guest_addr: u64,
        size: usize,
        readable: bool,
        writable: bool,
        executable: bool,
    ) -> Result<(), HvError> {
        #[cfg(target_os = "macos")]
        {
            let mut flags: u64 = 0;
            if readable {
                flags |= HV_MEMORY_READ;
            }
            if writable {
                flags |= HV_MEMORY_WRITE;
            }
            if executable {
                flags |= HV_MEMORY_EXEC;
            }

            // SAFETY:
            // * host_ptr is caller-guaranteed to be page-aligned and point to
            //   at least `size` valid bytes.
            // * size is caller-guaranteed to be a multiple of the page size.
            // * flags is a valid bitfield (we built it ourselves).
            let ret = unsafe {
                crate::ffi::hv_vm_map(host_ptr.cast::<c_void>(), guest_addr, size, flags)
            };
            if ret != HV_SUCCESS {
                error!(ret, guest_addr, size, flags, "hv_vm_map failed");
                return Err(HvError::Framework(ret));
            }
            debug!(guest_addr, size, flags, "mapped guest memory");
            Ok(())
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (host_ptr, guest_addr, size, readable, writable, executable);
            Err(HvError::NotSupported)
        }
    }

    /// Unmap a previously mapped guest IPA region.
    ///
    /// `guest_addr` and `size` must exactly match values passed to a prior
    /// call to [`map_memory`](Self::map_memory).
    ///
    /// # Errors
    /// [`HvError::Framework`] if the framework rejects the unmap.
    #[instrument(skip(self))]
    pub fn unmap_memory(&self, guest_addr: u64, size: usize) -> Result<(), HvError> {
        #[cfg(target_os = "macos")]
        {
            // SAFETY: guest_addr / size describe a valid, previously mapped
            // region — callers are required to uphold this precondition.
            let ret = unsafe { crate::ffi::hv_vm_unmap(guest_addr, size) };
            if ret != HV_SUCCESS {
                error!(ret, guest_addr, size, "hv_vm_unmap failed");
                return Err(HvError::Framework(ret));
            }
            debug!(guest_addr, size, "unmapped guest memory");
            Ok(())
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (guest_addr, size);
            Err(HvError::NotSupported)
        }
    }
}

impl Drop for Vm {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        {
            // SAFETY: We created the VM with hv_vm_create in Vm::new, so it
            // is valid to destroy it here.  vCPUs should have been destroyed
            // first; if they haven't, the framework will return an error that
            // we log but cannot propagate from a destructor.
            let ret = unsafe { crate::ffi::hv_vm_destroy() };
            if ret != HV_SUCCESS {
                error!(ret, "hv_vm_destroy failed");
            } else {
                debug!("VM destroyed");
            }
        }
    }
}
