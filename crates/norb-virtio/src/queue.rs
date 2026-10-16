//! Split virtqueue implementation (Virtio 1.2 §2.7).
//!
//! A split virtqueue consists of three regions in guest physical memory:
//!
//! 1. **Descriptor Table** — array of [`VirtqDesc`] entries that describe
//!    scatter-gather buffers (address, length, flags, next pointer).
//! 2. **Available Ring** — written by the driver; lists descriptor chain head
//!    indices that the driver has made available for the device to consume.
//! 3. **Used Ring** — written by the device; lists descriptor chain head
//!    indices (and bytes written) that the device has finished processing.
//!
//! All structures are in guest-native little-endian byte order; we read them
//! directly from the `mem: &[u8]` guest RAM slice using byte offsets.
//!
//! ## Safety
//!
//! The read/write helpers use raw pointer arithmetic to interpret guest RAM as
//! C structs.  The following invariants must hold at call sites:
//! - `mem` covers the entire guest RAM (index 0 == guest physical address 0).
//! - The descriptor/ring physical addresses stored in [`VirtQueue`] were
//!   previously validated by the MMIO transport to lie within `mem`.
//! - The queue size (`num`) is a power of two in the range [1, 32768].
//!
//! We use `get` / `copy_from_slice` rather than raw pointer casts to avoid
//! undefined behaviour; the only `unsafe` block is for reading the C-repr
//! struct from aligned guest memory.

use tracing::{trace, warn};

// ── Descriptor flags ──────────────────────────────────────────────────────────

/// Descriptor continues via `next` field.
pub const VIRTQ_DESC_F_NEXT: u16 = 1;
/// Buffer is device-writable (from guest perspective: write = to guest).
pub const VIRTQ_DESC_F_WRITE: u16 = 2;
/// Buffer contains a list of indirect descriptors.
pub const VIRTQ_DESC_F_INDIRECT: u16 = 4;

// ── C-compatible descriptor / ring structs ────────────────────────────────────

/// A single virtqueue descriptor.
///
/// # Layout
/// `repr(C)` ensures the compiler does not reorder or pad fields, matching the
/// Virtio 1.2 wire format exactly.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct VirtqDesc {
    /// Guest physical address of the buffer.
    pub addr: u64,
    /// Length of the buffer in bytes.
    pub len: u32,
    /// Descriptor flags (`VIRTQ_DESC_F_*`).
    pub flags: u16,
    /// Index of the next descriptor (valid only when `VIRTQ_DESC_F_NEXT` is set).
    pub next: u16,
}

/// Available ring — driver → device.
///
/// The ring array is fixed at 256 entries; the spec allows up to 32768.  We
/// use 256 everywhere to keep the types `Sized` without a heap allocation.
/// The actual used size is determined by [`VirtQueue::num`].
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VirtqAvail {
    /// Flags (bit 0: `VIRTQ_AVAIL_F_NO_INTERRUPT`).
    pub flags: u16,
    /// Index into `ring` where the driver will write the *next* entry.
    pub idx: u16,
    /// Descriptor chain head indices.
    pub ring: [u16; 256],
}

/// One entry in the used ring.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct VirtqUsedElem {
    /// Descriptor chain head index.
    pub id: u32,
    /// Total bytes written into the device-writable buffers.
    pub len: u32,
}

/// Used ring — device → driver.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VirtqUsed {
    /// Flags (bit 0: `VIRTQ_USED_F_NO_NOTIFY`).
    pub flags: u16,
    /// Index into `ring` where the device will write the *next* entry.
    pub idx: u16,
    /// Completed descriptor chains.
    pub ring: [VirtqUsedElem; 256],
}

// ── VirtQueue ─────────────────────────────────────────────────────────────────

/// Live virtqueue state.
///
/// Created by [`VirtioMmio`](crate::mmio::VirtioMmio) when the guest sets
/// `QueueReady = 1` and the physical addresses have been configured.
pub struct VirtQueue {
    /// Guest physical address of the descriptor table.
    pub desc_addr: u64,
    /// Guest physical address of the available ring.
    pub avail_addr: u64,
    /// Guest physical address of the used ring.
    pub used_addr: u64,
    /// Queue size (number of descriptors; must be a power of two).
    pub num: u16,
    /// Index of the next available ring entry to consume.
    pub last_avail_idx: u16,
    /// Whether the queue has been activated by the driver.
    pub ready: bool,
}

impl VirtQueue {
    /// Create a new (unactivated) virtqueue with capacity `num`.
    pub fn new(num: u16) -> Self {
        Self {
            desc_addr: 0,
            avail_addr: 0,
            used_addr: 0,
            num,
            last_avail_idx: 0,
            ready: false,
        }
    }

    /// Returns `true` when the queue is activated and has valid memory pointers.
    #[inline]
    pub fn is_ready(&self) -> bool {
        self.ready && self.desc_addr != 0 && self.avail_addr != 0 && self.used_addr != 0
    }

    // ── Available ring helpers ────────────────────────────────────────────────

    /// Read the available ring's `idx` field from guest memory (little-endian).
    fn avail_idx(&self, mem: &[u8]) -> u16 {
        // VirtqAvail layout: flags(u16) + idx(u16) + ring([u16; N])
        // idx is at byte offset 2 from avail_addr.
        let off = self.avail_addr as usize + 2;
        if off + 2 > mem.len() {
            warn!(avail_addr = self.avail_addr, "avail_idx out of bounds");
            return 0;
        }
        u16::from_le_bytes([mem[off], mem[off + 1]])
    }

    /// Read one entry from the available ring at position `idx % num`.
    fn avail_ring_entry(&self, mem: &[u8], idx: u16) -> u16 {
        // ring starts at avail_addr + 4 (flags u16 + idx u16)
        let slot = (idx % self.num) as usize;
        let off = self.avail_addr as usize + 4 + slot * 2;
        if off + 2 > mem.len() {
            warn!(avail_addr = self.avail_addr, slot, "avail ring entry OOB");
            return 0;
        }
        u16::from_le_bytes([mem[off], mem[off + 1]])
    }

    /// Pop the next available descriptor chain head from the available ring.
    ///
    /// Returns `Some(head_idx)` if there is at least one new entry since the
    /// last call, or `None` if the ring is empty.  Advances `last_avail_idx`.
    pub fn pop_avail(&mut self, mem: &[u8]) -> Option<u16> {
        if !self.is_ready() {
            return None;
        }
        let avail_idx = self.avail_idx(mem);
        if avail_idx == self.last_avail_idx {
            return None; // No new entries.
        }
        let head = self.avail_ring_entry(mem, self.last_avail_idx);
        self.last_avail_idx = self.last_avail_idx.wrapping_add(1);
        trace!(head, last_avail_idx = self.last_avail_idx, "pop_avail");
        Some(head)
    }

    // ── Descriptor table helpers ──────────────────────────────────────────────

    /// Read a [`VirtqDesc`] at `index` from the descriptor table in guest
    /// memory.
    ///
    /// # Safety
    /// We read exactly `size_of::<VirtqDesc>()` bytes from a byte-aligned
    /// guest memory slice via `copy_from_slice`, so there is no alignment or
    /// aliasing UB.
    fn read_desc(&self, mem: &[u8], index: u16) -> Option<VirtqDesc> {
        const DESC_SIZE: usize = std::mem::size_of::<VirtqDesc>(); // 16 bytes
        let off = self.desc_addr as usize + (index as usize) * DESC_SIZE;
        if off + DESC_SIZE > mem.len() {
            warn!(
                desc_addr = self.desc_addr,
                index, "descriptor read out of bounds"
            );
            return None;
        }
        // SAFETY: VirtqDesc is repr(C), no padding, all bit patterns valid.
        // We copy into a zeroed stack value then reinterpret the bytes.
        let mut desc = VirtqDesc::default();
        // SAFETY: the byte slice we copy from is exactly DESC_SIZE bytes and
        // within the bounds of `mem` (checked above).  The destination is a
        // properly-aligned local stack variable.
        unsafe {
            let dst =
                std::slice::from_raw_parts_mut(&mut desc as *mut VirtqDesc as *mut u8, DESC_SIZE);
            dst.copy_from_slice(&mem[off..off + DESC_SIZE]);
        }
        // Convert from guest little-endian.
        Some(VirtqDesc {
            addr: u64::from_le(desc.addr),
            len: u32::from_le(desc.len),
            flags: u16::from_le(desc.flags),
            next: u16::from_le(desc.next),
        })
    }

    /// Walk a descriptor chain starting at `head` and collect all buffers.
    ///
    /// Returns a `Vec` of `(guest_addr, len, is_device_writable)` tuples in
    /// chain order.  Stops at the first missing or invalid descriptor, or after
    /// visiting `num` descriptors (cycle guard).
    pub fn read_chain(&self, head: u16, mem: &[u8]) -> Vec<(u64, u32, bool)> {
        let mut result = Vec::new();
        let mut idx = head;
        let mut hops = 0u16;

        loop {
            if hops >= self.num {
                warn!(head, "descriptor chain cycle detected");
                break;
            }
            let Some(desc) = self.read_desc(mem, idx) else {
                break;
            };
            let is_write = (desc.flags & VIRTQ_DESC_F_WRITE) != 0;
            result.push((desc.addr, desc.len, is_write));
            trace!(idx, addr = desc.addr, len = desc.len, is_write, "desc");

            if (desc.flags & VIRTQ_DESC_F_NEXT) == 0 {
                break; // End of chain.
            }
            idx = desc.next;
            hops += 1;
        }

        result
    }

    // ── Used ring helpers ─────────────────────────────────────────────────────

    /// Read the current used ring `idx` counter from guest memory.
    fn used_idx(&self, mem: &[u8]) -> u16 {
        // VirtqUsed layout: flags(u16) + idx(u16) + ...
        let off = self.used_addr as usize + 2;
        if off + 2 > mem.len() {
            return 0;
        }
        u16::from_le_bytes([mem[off], mem[off + 1]])
    }

    /// Write the used ring `idx` counter to guest memory.
    fn write_used_idx(&self, mem: &mut [u8], idx: u16) {
        let off = self.used_addr as usize + 2;
        if off + 2 > mem.len() {
            warn!("write_used_idx out of bounds");
            return;
        }
        let bytes = u16::to_le_bytes(idx);
        mem[off] = bytes[0];
        mem[off + 1] = bytes[1];
    }

    /// Push a completed descriptor chain entry into the used ring.
    ///
    /// `id` is the descriptor chain head index (from `pop_avail`); `len` is
    /// the total number of bytes written by the device into device-writable
    /// descriptors.
    pub fn push_used(&mut self, mem: &mut [u8], id: u32, len: u32) {
        if !self.is_ready() {
            return;
        }
        let used_idx = self.used_idx(mem);
        let slot = (used_idx % self.num) as usize;

        // VirtqUsed layout:
        //   flags  (2 bytes)
        //   idx    (2 bytes)
        //   ring[] (8 bytes × num) — each VirtqUsedElem is { id: u32, len: u32 }
        const USED_ELEM_SIZE: usize = std::mem::size_of::<VirtqUsedElem>(); // 8 bytes
        let off = self.used_addr as usize + 4 + slot * USED_ELEM_SIZE;

        if off + USED_ELEM_SIZE > mem.len() {
            warn!(used_addr = self.used_addr, slot, "push_used out of bounds");
            return;
        }

        let id_bytes = u32::to_le_bytes(id);
        let len_bytes = u32::to_le_bytes(len);
        mem[off..off + 4].copy_from_slice(&id_bytes);
        mem[off + 4..off + 8].copy_from_slice(&len_bytes);

        // Advance the used index (visible to driver after memory barrier).
        let new_idx = used_idx.wrapping_add(1);
        self.write_used_idx(mem, new_idx);
        trace!(id, len, new_used_idx = new_idx, "push_used");
    }

    // ── Guest memory helpers ──────────────────────────────────────────────────

    /// Read `len` bytes from `guest_addr` in the guest memory slice.
    ///
    /// Returns an empty `Vec` if the range is out of bounds.
    pub fn read_bytes(mem: &[u8], guest_addr: u64, len: usize) -> Vec<u8> {
        let start = guest_addr as usize;
        let end = start.saturating_add(len);
        if end > mem.len() {
            warn!(
                guest_addr,
                len, "read_bytes: range exceeds guest memory size"
            );
            return Vec::new();
        }
        mem[start..end].to_vec()
    }

    /// Write `data` into guest memory at `guest_addr`.
    ///
    /// Silently truncates if the write would exceed the memory slice bounds.
    pub fn write_bytes(mem: &mut [u8], guest_addr: u64, data: &[u8]) {
        let start = guest_addr as usize;
        let end = start.saturating_add(data.len());
        if end > mem.len() {
            warn!(
                guest_addr,
                len = data.len(),
                "write_bytes: range exceeds guest memory size"
            );
            let available = mem.len().saturating_sub(start);
            if available > 0 {
                mem[start..start + available].copy_from_slice(&data[..available]);
            }
            return;
        }
        mem[start..end].copy_from_slice(data);
    }
}
