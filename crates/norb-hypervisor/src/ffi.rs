//! Raw FFI bindings to Apple's `Hypervisor.framework` (AArch64 / ARM64).
//!
//! Every declaration in this module is `unsafe` to call and deliberately
//! mirrors the C API without interpretation.  Higher-level safe wrappers live
//! in [`crate::vm`], [`crate::vcpu`], and [`crate::memory`].
//!
//! # ABI notes
//! * `hv_return_t` is `uint64_t` in the SDK headers.
//! * `hv_vcpu_t` is `uint64_t`.
//! * All functions use the C calling convention (`extern "C"`).
//! * The `HvVcpuExit` / `HvVcpuExitException` structs use `#[repr(C)]` so
//!   their layout exactly matches what the framework writes through the exit
//!   pointer returned by `hv_vcpu_create`.

// Only compile the real FFI on macOS.  The rest of the crate provides stubs
// for other targets so `cargo check` succeeds everywhere.
#![allow(non_camel_case_types)]

/// Return type of every `Hypervisor.framework` function.
///
/// `hv_return_t` is `typedef uint64_t hv_return_t` in the SDK.
/// A value of [`HV_SUCCESS`] (0) means success; any other value is an error
/// code whose meaning is documented in `<Hypervisor/hv_error.h>`.
pub type HvReturn = u64;

/// The success sentinel returned by `Hypervisor.framework` functions.
pub const HV_SUCCESS: HvReturn = 0;

// ── Memory-mapping permission flags ──────────────────────────────────────────

/// Guest memory is readable.
pub const HV_MEMORY_READ: u64 = 1 << 0;
/// Guest memory is writable.
pub const HV_MEMORY_WRITE: u64 = 1 << 1;
/// Guest memory is executable.
pub const HV_MEMORY_EXEC: u64 = 1 << 2;

// ── AArch64 general-purpose / special register identifiers ───────────────────

/// General-purpose and special register identifiers for `hv_vcpu_get_reg` /
/// `hv_vcpu_set_reg`.
///
/// The numeric values match the `hv_reg_t` enumeration in
/// `<Hypervisor/hv_vcpu.h>`.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum HvReg {
    X0 = 0,
    X1 = 1,
    X2 = 2,
    X3 = 3,
    X4 = 4,
    X5 = 5,
    X6 = 6,
    X7 = 7,
    X8 = 8,
    X9 = 9,
    X10 = 10,
    X11 = 11,
    X12 = 12,
    X13 = 13,
    X14 = 14,
    X15 = 15,
    X16 = 16,
    X17 = 17,
    X18 = 18,
    X19 = 19,
    X20 = 20,
    X21 = 21,
    X22 = 22,
    X23 = 23,
    X24 = 24,
    X25 = 25,
    X26 = 26,
    X27 = 27,
    X28 = 28,
    /// Frame pointer (x29)
    X29 = 29,
    /// Link register (x30)
    X30 = 30,
    /// Program counter
    Pc = 31,
    /// Floating-point control register
    Fpcr = 32,
    /// Floating-point status register
    Fpsr = 33,
    /// Current program status register
    Cpsr = 34,
}

// ── AArch64 system register identifiers ──────────────────────────────────────

/// System register identifiers for `hv_vcpu_get_sys_reg` /
/// `hv_vcpu_set_sys_reg`.
///
/// Values match `hv_sys_reg_t` in `<Hypervisor/hv_vcpu.h>`.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum HvSysReg {
    /// System Control Register (EL1)
    SctlrEl1 = 0xc080,
    /// Translation Table Base Register 0 (EL1)
    Ttbr0El1 = 0xc100,
    /// Translation Table Base Register 1 (EL1)
    Ttbr1El1 = 0xc101,
    /// Translation Control Register (EL1)
    TcrEl1 = 0xc102,
    /// Exception Syndrome Register (EL1)
    EsrEl1 = 0xc290,
    /// Fault Address Register (EL1)
    FarEl1 = 0xc300,
    /// Memory Attribute Indirection Register (EL1)
    MairEl1 = 0xc510,
    /// Vector Base Address Register (EL1)
    VbarEl1 = 0xc600,
    /// Stack Pointer (EL0)
    SpEl0 = 0x4208,
    /// Stack Pointer (EL1)
    SpEl1 = 0xe208,
    /// Exception Link Register (EL1)
    ElrEl1 = 0xc201,
    /// Saved Program Status Register (EL1)
    SpsrEl1 = 0xc200,
}

// ── Exit information structures ───────────────────────────────────────────────

/// Reason the vCPU stopped executing.
///
/// Mirrors `hv_exit_reason_t` in `<Hypervisor/hv_vcpu.h>`.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HvExitReason {
    /// The vCPU was canceled (e.g. via `hv_vcpus_exit`).
    Canceled = 0,
    /// A synchronous or asynchronous exception occurred; inspect
    /// `HvVcpuExit::exception` for details.
    Exception = 1,
    /// The virtual timer fired.
    VTimerActivated = 2,
    /// An exit reason the framework may add in the future.
    Unknown = u32::MAX,
}

/// Per-exit exception information populated by the framework.
///
/// # Safety
/// This struct is written by the hypervisor framework through the raw pointer
/// returned by `hv_vcpu_create`.  The framework guarantees the pointed-to
/// memory is valid for the lifetime of the vCPU.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct HvVcpuExitException {
    /// Value of `ESR_EL2` at the time of the exit (Exception Syndrome).
    pub syndrome: u64,
    /// Value of `FAR_EL2` (Fault Address Register) — the virtual address that
    /// caused the fault, if applicable.
    pub virtual_address: u64,
    /// Intermediate Physical Address (IPA) that caused the fault.
    pub physical_address: u64,
}

/// Exit information structure filled by the framework on every `hv_vcpu_run`
/// return.
///
/// # Safety
/// Layout must exactly match `hv_vcpu_exit_t` from the SDK.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct HvVcpuExit {
    /// Why the vCPU stopped.
    pub reason: HvExitReason,
    /// Exception details (only valid when `reason == Exception`).
    pub exception: HvVcpuExitException,
}

// ── Raw extern "C" declarations ───────────────────────────────────────────────

#[cfg(target_os = "macos")]
extern "C" {
    // ── VM lifecycle ──────────────────────────────────────────────────────────

    /// Create a new hypervisor VM for the current process.
    ///
    /// # Safety
    /// * `config` may be NULL (pass `std::ptr::null_mut()`) to use defaults.
    /// * Only one VM may exist per process at a time; a second call before
    ///   `hv_vm_destroy` returns an error.
    pub fn hv_vm_create(config: *mut std::ffi::c_void) -> HvReturn;

    /// Destroy the hypervisor VM for the current process.
    ///
    /// # Safety
    /// * Must be called after all vCPUs have been destroyed.
    /// * Must not be called if no VM exists.
    pub fn hv_vm_destroy() -> HvReturn;

    /// Map a region of host virtual memory into the guest IPA space.
    ///
    /// # Safety
    /// * `addr` must point to at least `size` bytes of page-aligned memory
    ///   that remains valid for the entire time the mapping is active.
    /// * `size` must be a multiple of the host page size (4 KiB on ARM64).
    /// * `flags` is a bitwise OR of [`HV_MEMORY_READ`], [`HV_MEMORY_WRITE`],
    ///   [`HV_MEMORY_EXEC`].
    pub fn hv_vm_map(addr: *mut std::ffi::c_void, ipa: u64, size: usize, flags: u64)
        -> HvReturn;

    /// Unmap a previously mapped IPA region.
    ///
    /// # Safety
    /// * `ipa` and `size` must exactly match a previous `hv_vm_map` call.
    pub fn hv_vm_unmap(ipa: u64, size: usize) -> HvReturn;

    // ── vCPU lifecycle ────────────────────────────────────────────────────────

    /// Create a new virtual CPU.
    ///
    /// # Safety
    /// * `vcpu` must point to a valid `u64` to receive the vCPU handle.
    /// * `exit` must point to a valid `*mut HvVcpuExit` pointer; the framework
    ///   will set it to a framework-owned `HvVcpuExit` that is valid until
    ///   `hv_vcpu_destroy`.
    /// * `config` may be NULL.
    pub fn hv_vcpu_create(
        vcpu: *mut u64,
        exit: *mut *mut HvVcpuExit,
        config: *mut std::ffi::c_void,
    ) -> HvReturn;

    /// Destroy a virtual CPU.
    ///
    /// # Safety
    /// * Must not be called while the vCPU is running.
    pub fn hv_vcpu_destroy(vcpu: u64) -> HvReturn;

    /// Run a virtual CPU until it exits.
    ///
    /// # Safety
    /// * `vcpu` must be a valid handle from `hv_vcpu_create`.
    /// * The thread calling `hv_vcpu_run` becomes the vCPU thread; only one
    ///   thread may call this at a time for a given vCPU.
    pub fn hv_vcpu_run(vcpu: u64) -> HvReturn;

    // ── Register access ───────────────────────────────────────────────────────

    /// Read a general-purpose or special register.
    ///
    /// # Safety
    /// * `vcpu` must be valid.
    /// * `value` must point to a valid `u64`.
    pub fn hv_vcpu_get_reg(vcpu: u64, reg: HvReg, value: *mut u64) -> HvReturn;

    /// Write a general-purpose or special register.
    ///
    /// # Safety
    /// * `vcpu` must be valid.
    pub fn hv_vcpu_set_reg(vcpu: u64, reg: HvReg, value: u64) -> HvReturn;

    /// Read a system register.
    ///
    /// # Safety
    /// * `vcpu` must be valid.
    /// * `value` must point to a valid `u64`.
    pub fn hv_vcpu_get_sys_reg(vcpu: u64, reg: HvSysReg, value: *mut u64) -> HvReturn;

    /// Write a system register.
    ///
    /// # Safety
    /// * `vcpu` must be valid.
    pub fn hv_vcpu_set_sys_reg(vcpu: u64, reg: HvSysReg, value: u64) -> HvReturn;
}
