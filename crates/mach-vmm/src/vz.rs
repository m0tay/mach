//! Safe Rust interface to Apple Virtualization (`vz.m`).

use anyhow::{bail, Result};
use std::ffi::CString;
use std::os::unix::io::RawFd;
use std::path::Path;

extern "C" {
    fn vz_start_vm(
        kernel_path: *const libc::c_char,
        initrd_path: *const libc::c_char,
        disk_path: *const libc::c_char,
        cidata_path: *const libc::c_char,
        shared_dir_path: *const libc::c_char,
        cmdline: *const libc::c_char,
        cpus: u32,
        memory_mb: u64,
        mac_addr: *const libc::c_char,
        in_fd: libc::c_int,
        out_fd: libc::c_int,
    ) -> libc::c_int;
}

/// Run a native hardware-accelerated Linux virtual machine via Apple Virtualization.
///
/// If `kernel` is `Some`, boots directly using `VZLinuxBootLoader`.
/// If `kernel` is `None`, boots using `VZEFIBootLoader` (for cloud images).
/// VM termination and shutdown events are handled cleanly via `NorbVMDelegate`.
pub fn run_vz_vm(
    kernel: Option<&Path>,
    initrd: Option<&Path>,
    disk: Option<&Path>,
    cidata: Option<&Path>,
    shared_dir: Option<&Path>,
    cmdline: &str,
    cpus: u32,
    memory_mb: u64,
    mac_addr: &str,
    in_fd: RawFd,
    out_fd: RawFd,
) -> Result<()> {
    let kernel_c = kernel
        .map(|p| CString::new(p.to_string_lossy().as_bytes()))
        .transpose()?;
    let initrd_c = initrd
        .map(|p| CString::new(p.to_string_lossy().as_bytes()))
        .transpose()?;
    let disk_c = disk
        .map(|p| CString::new(p.to_string_lossy().as_bytes()))
        .transpose()?;
    let cidata_c = cidata
        .map(|p| CString::new(p.to_string_lossy().as_bytes()))
        .transpose()?;
    let shared_c = shared_dir
        .map(|p| CString::new(p.to_string_lossy().as_bytes()))
        .transpose()?;
    let cmdline_c = CString::new(cmdline)?;
    let mac_c = CString::new(mac_addr)?;

    let ret = unsafe {
        vz_start_vm(
            kernel_c.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()),
            initrd_c.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()),
            disk_c.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()),
            cidata_c.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()),
            shared_c.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()),
            cmdline_c.as_ptr(),
            cpus,
            memory_mb,
            mac_c.as_ptr(),
            in_fd,
            out_fd,
        )
    };

    if ret != 0 {
        bail!("Apple Virtualization failed to run VM (error code: {ret})");
    }

    Ok(())
}
