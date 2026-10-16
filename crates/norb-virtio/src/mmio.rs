//! Virtio MMIO transport layer.
//!
//! Implements the Virtio 1.2 MMIO transport (spec §4.2). A guest driver discovers
//! a virtio device by reading the magic number, version, and device ID from the
//! MMIO region, then negotiates features, sets up virtqueues, and writes to the
//! Status register to signal readiness.
//!
//! ## Register map (relative to `base_addr`)
//!
//! | Offset | Name              | Dir | Notes                         |
//! |--------|-------------------|-----|-------------------------------|
//! | 0x000  | MagicValue        | R   | 0x74726976 ("virt")           |
//! | 0x004  | Version           | R   | 0x2                           |
//! | 0x008  | DeviceID          | R   | device-specific               |
//! | 0x00c  | VendorID          | R   | 0x554d4551 ("QEMU")           |
//! | 0x010  | DeviceFeatures    | R   | selected by DeviceFeaturesSel |
//! | 0x014  | DeviceFeaturesSel | W   |                               |
//! | 0x020  | DriverFeatures    | W   | selected by DriverFeaturesSel |
//! | 0x024  | DriverFeaturesSel | W   |                               |
//! | 0x030  | QueueSel          | W   |                               |
//! | 0x034  | QueueNumMax       | R   | max descriptors in queue      |
//! | 0x038  | QueueNum          | W   | actual queue size chosen      |
//! | 0x044  | QueueReady        | RW  |                               |
//! | 0x050  | QueueNotify       | W   | triggers queue_notify()       |
//! | 0x060  | InterruptStatus   | R   |                               |
//! | 0x064  | InterruptACK      | W   |                               |
//! | 0x070  | Status            | RW  | ACKNOWLEDGE/DRIVER/etc bits   |
//! | 0x080  | QueueDescLow      | W   | lower 32 bits of desc PA      |
//! | 0x084  | QueueDescHigh     | W   | upper 32 bits of desc PA      |
//! | 0x090  | QueueDriverLow    | W   | available ring PA (low)       |
//! | 0x094  | QueueDriverHigh   | W   | available ring PA (high)      |
//! | 0x0a0  | QueueDeviceLow    | W   | used ring PA (low)            |
//! | 0x0a4  | QueueDeviceHigh   | W   | used ring PA (high)           |
//! | 0x0fc  | ConfigGeneration  | R   |                               |
//! | 0x100+ | Config space      | RW  | device-specific               |

use crate::queue::VirtQueue;
use tracing::{debug, trace, warn};

// ── Virtio MMIO magic / version ───────────────────────────────────────────────

/// Magic value that identifies a Virtio MMIO device ("virt" in little-endian).
pub const MMIO_MAGIC: u32 = 0x7472_6976;

/// Virtio MMIO transport version 2 (legacy = 1, modern = 2).
pub const MMIO_VERSION: u32 = 0x2;

/// Fake vendor ID — matches QEMU's value so guest drivers don't complain.
pub const MMIO_VENDOR_ID: u32 = 0x554d_4551;

/// Maximum number of virtqueues supported per device.
pub const MAX_QUEUES: usize = 8;

/// Maximum number of descriptors per queue (must be a power of two, ≤ 32768).
pub const QUEUE_NUM_MAX: u16 = 256;

// ── Virtio Status register bits (spec §2.1) ───────────────────────────────────

/// Guest OS has found the device.
pub const STATUS_ACKNOWLEDGE: u32 = 1;
/// Guest OS knows how to drive the device.
pub const STATUS_DRIVER: u32 = 2;
/// Driver has finished setting up the device.
pub const STATUS_DRIVER_OK: u32 = 4;
/// Driver has acknowledged all features it understands.
pub const STATUS_FEATURES_OK: u32 = 8;
/// Device has experienced an unrecoverable error.
pub const STATUS_DEVICE_NEEDS_RESET: u32 = 64;
/// Something went wrong in the guest.
pub const STATUS_FAILED: u32 = 128;

// ── Interrupt status bits (spec §4.2.3) ──────────────────────────────────────

/// Interrupt was asserted because a used buffer has been placed in a used ring.
pub const INT_USED_RING: u32 = 0x1;
/// Interrupt was asserted because the configuration has changed.
pub const INT_CONFIG_CHANGE: u32 = 0x2;

// ── MmioDevice trait ─────────────────────────────────────────────────────────

/// Trait implemented by every Virtio device that sits behind the MMIO transport.
///
/// The transport ([`VirtioMmio`]) owns the queue state and guest memory; the
/// device only needs to handle feature negotiation, config-space accesses, and
/// `queue_notify` callbacks.
pub trait MmioDevice: Send {
    /// Virtio device ID (e.g. 3 for console, 1 for net, 2 for block).
    fn device_id(&self) -> u32;

    /// Return 32 bits of device feature flags for the given `page` (0 = bits
    /// 0–31, 1 = bits 32–63). Bits 24–31 of page 1 are reserved for transport
    /// features.
    fn device_features(&self, page: u32) -> u32;

    /// Read `size` bytes from the device-specific configuration space at
    /// `offset` (relative to the start of the config region, i.e. MMIO+0x100).
    fn config_read(&self, offset: u64, size: u8) -> u32;

    /// Write `size` bytes to the device-specific configuration space.
    fn config_write(&mut self, offset: u64, value: u32, size: u8);

    /// Called when the guest driver writes to QueueNotify. `queue_idx` is the
    /// queue that has new available buffers. `mem` is the entire guest RAM
    /// slice so the device can read/write descriptor data.
    fn queue_notify(&mut self, queue_idx: u32, mem: &mut [u8], queue: Option<&mut VirtQueue>);

    /// Human-readable device name used in log messages.
    fn device_name(&self) -> &str;
}

// ── Per-queue configuration registers ────────────────────────────────────────

/// Staging area for queue configuration writes.  Once `QueueReady` is set the
/// values are committed into the corresponding [`VirtQueue`].
#[derive(Default, Clone)]
struct QueueConfig {
    /// Selected queue size (written by driver via QueueNum).
    num: u16,
    /// Descriptor table base address (64-bit, written as two 32-bit halves).
    desc_addr: u64,
    /// Available ring base address.
    avail_addr: u64,
    /// Used ring base address.
    used_addr: u64,
    /// Whether the queue is active (QueueReady == 1).
    ready: bool,
}

// ── VirtioMmio ────────────────────────────────────────────────────────────────

/// Virtio MMIO transport.
///
/// Wraps a [`MmioDevice`] and handles all register-level reads/writes from the
/// guest.  The VMM calls [`mmio_read`] / [`mmio_write`] on every MMIO exit.
///
/// [`mmio_read`]: VirtioMmio::mmio_read
/// [`mmio_write`]: VirtioMmio::mmio_write
pub struct VirtioMmio {
    /// Guest physical base address of this MMIO region.
    base_addr: u64,
    /// The underlying virtio device.
    device: Box<dyn MmioDevice>,

    // ── feature negotiation ──────────────────────────────────────────────────
    /// Which page of device features the driver is currently reading (set by
    /// writing DeviceFeaturesSel).
    device_features_sel: u32,
    /// Features the driver has accepted (two pages, indexed by DriverFeaturesSel).
    driver_features: [u32; 2],
    /// Which page of driver features the driver is writing.
    driver_features_sel: u32,

    // ── queue management ─────────────────────────────────────────────────────
    /// Index of the currently-selected queue (written via QueueSel).
    queue_sel: u32,
    /// Per-queue configuration registers (staging).
    queue_configs: [QueueConfig; MAX_QUEUES],
    /// Live virtqueue state (populated once QueueReady is set).
    queues: [Option<VirtQueue>; MAX_QUEUES],

    // ── device status ────────────────────────────────────────────────────────
    /// Device status register bits.
    status: u32,
    /// Pending interrupt status bits (cleared by guest writing InterruptACK).
    interrupt_status: u32,
    /// Config generation counter (incremented when config space changes).
    config_generation: u32,
}

impl VirtioMmio {
    /// Create a new MMIO transport wrapping `device` mapped at `base_addr`.
    pub fn new(base_addr: u64, device: Box<dyn MmioDevice>) -> Self {
        const NO_QUEUE: Option<VirtQueue> = None;
        Self {
            base_addr,
            device,
            device_features_sel: 0,
            driver_features: [0; 2],
            driver_features_sel: 0,
            queue_sel: 0,
            queue_configs: std::array::from_fn(|_| QueueConfig::default()),
            queues: [NO_QUEUE; MAX_QUEUES],
            status: 0,
            interrupt_status: 0,
            config_generation: 0,
        }
    }

    /// Guest physical base address of this MMIO region.
    #[inline]
    pub fn base_addr(&self) -> u64 {
        self.base_addr
    }

    /// Size of the standard Virtio MMIO region (0x200 bytes).
    #[inline]
    pub const fn size() -> u64 {
        0x200
    }

    /// Returns `true` if there is a pending interrupt for the guest.
    #[inline]
    pub fn has_interrupt(&self) -> bool {
        self.interrupt_status != 0
    }

    /// Raw interrupt status bits (OR of [`INT_USED_RING`] / [`INT_CONFIG_CHANGE`]).
    #[inline]
    pub fn interrupt_status(&self) -> u32 {
        self.interrupt_status
    }

    /// Assert a used-ring interrupt (called by the device after pushing to a
    /// used ring so the guest ISR can be notified).
    pub fn assert_interrupt(&mut self) {
        self.interrupt_status |= INT_USED_RING;
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    /// Return the index of the currently selected queue, clamped to
    /// `MAX_QUEUES - 1` to prevent out-of-bounds access.
    #[inline]
    fn sel(&self) -> usize {
        (self.queue_sel as usize).min(MAX_QUEUES - 1)
    }

    /// Activate the currently selected queue using the staged config values.
    fn activate_queue(&mut self, sel: usize) {
        let cfg = &self.queue_configs[sel];
        let num = if cfg.num == 0 { QUEUE_NUM_MAX } else { cfg.num };
        let mut q = VirtQueue::new(num);
        q.desc_addr = cfg.desc_addr;
        q.avail_addr = cfg.avail_addr;
        q.used_addr = cfg.used_addr;
        q.ready = true;
        self.queues[sel] = Some(q);
        debug!(
            device = self.device.device_name(),
            queue = sel,
            num,
            desc_addr = cfg.desc_addr,
            avail_addr = cfg.avail_addr,
            used_addr = cfg.used_addr,
            "virtqueue activated"
        );
    }

    // ── MMIO register reads ───────────────────────────────────────────────────

    /// Handle a guest MMIO read.
    ///
    /// `offset` is relative to [`base_addr`](Self::base_addr). `size` is the
    /// access width in bytes (1, 2, or 4). Returns the register value; invalid
    /// offsets return 0.
    pub fn mmio_read(&self, offset: u64, size: u8) -> u32 {
        trace!(
            device = self.device.device_name(),
            offset = offset,
            size = size,
            "mmio_read"
        );

        // Config space — delegate to the device.
        if offset >= 0x100 {
            return self.device.config_read(offset - 0x100, size);
        }

        match offset {
            0x000 => MMIO_MAGIC,
            0x004 => MMIO_VERSION,
            0x008 => self.device.device_id(),
            0x00c => MMIO_VENDOR_ID,
            0x010 => self.device.device_features(self.device_features_sel),
            0x034 => u32::from(QUEUE_NUM_MAX),
            0x044 => {
                // QueueReady
                u32::from(self.queue_configs[self.sel()].ready)
            }
            0x060 => self.interrupt_status,
            0x070 => self.status,
            0x0fc => self.config_generation,
            other => {
                warn!(
                    device = self.device.device_name(),
                    offset = other,
                    "unhandled mmio_read"
                );
                0
            }
        }
    }

    // ── MMIO register writes ──────────────────────────────────────────────────

    /// Handle a guest MMIO write.
    ///
    /// `offset` is relative to [`base_addr`](Self::base_addr). `size` is the
    /// access width in bytes. `mem` is the guest's full RAM slice, required
    /// when the write triggers a `queue_notify`.
    pub fn mmio_write(&mut self, offset: u64, value: u32, size: u8, mem: &mut [u8]) {
        trace!(
            device = self.device.device_name(),
            offset = offset,
            value = value,
            size = size,
            "mmio_write"
        );

        // Config space — delegate.
        if offset >= 0x100 {
            self.device.config_write(offset - 0x100, value, size);
            return;
        }

        match offset {
            // ── Feature negotiation ──────────────────────────────────────────
            0x014 => {
                self.device_features_sel = value;
            }
            0x020 => {
                let sel = self.driver_features_sel as usize;
                if sel < 2 {
                    self.driver_features[sel] = value;
                }
            }
            0x024 => {
                self.driver_features_sel = value;
            }

            // ── Queue configuration ──────────────────────────────────────────
            0x030 => {
                self.queue_sel = value;
                trace!(queue_sel = value, "QueueSel");
            }
            0x038 => {
                let sel = self.sel();
                self.queue_configs[sel].num = value as u16;
            }
            0x044 => {
                // QueueReady
                let sel = self.sel();
                let ready = value != 0;
                self.queue_configs[sel].ready = ready;
                if ready {
                    self.activate_queue(sel);
                } else {
                    // Deactivate: drop the live queue object.
                    self.queues[sel] = None;
                    debug!(
                        device = self.device.device_name(),
                        queue = sel,
                        "virtqueue deactivated"
                    );
                }
            }
            0x050 => {
                // QueueNotify — value is the queue index to kick.
                let queue_idx = value;
                debug!(
                    device = self.device.device_name(),
                    queue = queue_idx,
                    "QueueNotify"
                );
                let sel = (queue_idx as usize).min(MAX_QUEUES - 1);
                let q = self.queues[sel].as_mut();
                self.device.queue_notify(queue_idx, mem, q);
                // After processing, assert a used-ring interrupt so the guest
                // ISR knows there are completions to collect.
                self.interrupt_status |= INT_USED_RING;
            }
            0x064 => {
                // InterruptACK — guest clears the bits it has handled.
                self.interrupt_status &= !value;
            }
            0x070 => {
                // Status — guest writes new status bits.  A write of 0 means
                // device reset.
                if value == 0 {
                    debug!(
                        device = self.device.device_name(),
                        "device reset requested"
                    );
                    self.reset();
                } else {
                    self.status = value;
                    debug!(
                        device = self.device.device_name(),
                        status = value,
                        "device status updated"
                    );
                }
            }

            // ── Queue descriptor table address ───────────────────────────────
            0x080 => {
                let sel = self.sel();
                self.queue_configs[sel].desc_addr =
                    (self.queue_configs[sel].desc_addr & 0xffff_ffff_0000_0000) | u64::from(value);
            }
            0x084 => {
                let sel = self.sel();
                self.queue_configs[sel].desc_addr =
                    (self.queue_configs[sel].desc_addr & 0x0000_0000_ffff_ffff)
                        | (u64::from(value) << 32);
            }

            // ── Available ring (driver area) address ─────────────────────────
            0x090 => {
                let sel = self.sel();
                self.queue_configs[sel].avail_addr =
                    (self.queue_configs[sel].avail_addr & 0xffff_ffff_0000_0000) | u64::from(value);
            }
            0x094 => {
                let sel = self.sel();
                self.queue_configs[sel].avail_addr =
                    (self.queue_configs[sel].avail_addr & 0x0000_0000_ffff_ffff)
                        | (u64::from(value) << 32);
            }

            // ── Used ring (device area) address ──────────────────────────────
            0x0a0 => {
                let sel = self.sel();
                self.queue_configs[sel].used_addr =
                    (self.queue_configs[sel].used_addr & 0xffff_ffff_0000_0000) | u64::from(value);
            }
            0x0a4 => {
                let sel = self.sel();
                self.queue_configs[sel].used_addr =
                    (self.queue_configs[sel].used_addr & 0x0000_0000_ffff_ffff)
                        | (u64::from(value) << 32);
            }

            other => {
                warn!(
                    device = self.device.device_name(),
                    offset = other,
                    value = value,
                    "unhandled mmio_write"
                );
            }
        }
    }

    /// Reset all device state (triggered by guest writing 0 to Status).
    fn reset(&mut self) {
        self.status = 0;
        self.interrupt_status = 0;
        self.device_features_sel = 0;
        self.driver_features = [0; 2];
        self.driver_features_sel = 0;
        self.queue_sel = 0;
        // Reset all queue configs and live queues.
        for i in 0..MAX_QUEUES {
            self.queue_configs[i] = QueueConfig::default();
            self.queues[i] = None;
        }
    }
}
