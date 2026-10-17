//! `mach-vmm` — Virtual Machine Monitor layer for Mach.
//!
//! Orchestrates the hypervisor, memory mapping, device tree generation,
//! MMIO device dispatching, and guest console I/O.

pub mod boot;
pub mod config;
pub mod fdt;
pub mod mmio_bus;
pub mod vmm;
pub mod vz;

pub use config::VmmConfig;
pub use vmm::Vmm;
pub use vz::run_vz_vm;
