//! `mach-virtio` — Virtio MMIO transport and device implementations for the Mach VMM.
//!
//! This crate provides:
//! - A Virtio MMIO transport layer ([`VirtioMmio`]) that handles guest MMIO reads/writes
//!   and dispatches them to the underlying device.
//! - A split virtqueue implementation ([`VirtQueue`]) compliant with Virtio 1.2 §2.7.
//! - A virtio-console device ([`VirtioConsole`], device ID 3) for terminal I/O with the
//!   Linux guest.
//!
//! # Architecture
//!
//! ```text
//! Guest VM
//!   │  MMIO exits (read/write at base_addr + offset)
//!   ▼
//! VirtioMmio  ──► MmioDevice trait (implemented by VirtioConsole, etc.)
//!   │
//!   ▼
//! VirtQueue(s)   ──► guest physical memory (shared &mut [u8])
//! ```

pub mod console;
pub mod mmio;
pub mod queue;

pub use console::VirtioConsole;
pub use mmio::{MmioDevice, VirtioMmio};

use thiserror::Error;

/// Top-level error type for the mach-virtio crate.
#[derive(Debug, Error)]
pub enum VirtioError {
    /// A guest MMIO access was outside the valid register range or had a bad size.
    #[error("invalid MMIO access at offset {addr:#x} with size {size}")]
    InvalidAccess { addr: u64, size: u8 },

    /// A virtqueue operation failed (e.g. descriptor chain is malformed).
    #[error("virtqueue error: {0}")]
    QueueError(String),

    /// A device-level error occurred.
    #[error("device error: {0}")]
    DeviceError(String),
}
