//! `mach-hypervisor` — Safe Rust bindings to Apple's `Hypervisor.framework`.
//!
//! This crate wraps the low-level CPU virtualization API available on macOS
//! (ARM64 / Apple Silicon and Intel with VT-x). It exposes three main
//! abstractions:
//!
//! * [`Vm`]          — The hypervisor virtual machine context (one per process).
//! * [`VCpu`]        — A virtual CPU that runs inside the VM.
//! * [`GuestMemory`] — Page-aligned host memory mapped into the guest IPA space.
//!
//! # Example
//! ```no_run
//! use mach_hypervisor::{Vm, VCpu, GuestMemory};
//!
//! # fn main() -> Result<(), mach_hypervisor::HvError> {
//! let vm = Vm::new()?;
//!
//! // Allocate 4 MiB of guest RAM starting at IPA 0x0.
//! let mut ram = GuestMemory::allocate(4 * 1024 * 1024, 0x0)?;
//!
//! // Map it into the guest address space (rwx).
//! vm.map_memory(ram.host_ptr(), ram.guest_base(), ram.size(), true, true, true)?;
//!
//! // Create a vCPU and point it at our guest code.
//! let vcpu = VCpu::new()?;
//! vcpu.set_pc(0x0)?;
//!
//! let exit = vcpu.run()?;
//! println!("vCPU exited: {:?}", exit);
//! # Ok(())
//! # }
//! ```
//!
//! # Platform support
//! All types are guarded with `#[cfg(target_os = "macos")]`. On other
//! operating systems, the public API surface is still exported (as stub
//! types / functions that immediately return [`HvError::NotSupported`]) so
//! the crate can be syntax-checked in CI on Linux hosts.

#![deny(missing_docs)]
#![deny(unsafe_op_in_unsafe_fn)]

pub mod ffi;
pub mod memory;
pub mod vcpu;
pub mod vm;

pub use memory::{GuestMemory, HvError};
pub use vcpu::{VCpu, VCpuExit};
pub use vm::Vm;

// Re-export the FFI register types so callers don't need to reach into `ffi`.
#[cfg(any(target_os = "macos", doc))]
pub use ffi::{HvReg, HvSysReg};
