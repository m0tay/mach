//! `norb run-vm` internal background VMM executor.

use anyhow::Result;
use norb_vmm::run_vz_vm;
use std::os::unix::io::AsRawFd;
use std::path::Path;

pub fn run(
    _name: &str,
    kernel: Option<&Path>,
    initrd: Option<&Path>,
    disk: Option<&Path>,
    cidata: Option<&Path>,
    shared_dir: Option<&Path>,
    cpus: u32,
    memory_mb: u64,
    mac: &str,
) -> Result<()> {
    let cmdline = "console=hvc0 rdinit=/init quiet";

    // Use an unread pipe for stdin and stdout (vm.log) for console output
    let in_pipe = nix::unistd::pipe()?;
    let out_fd = std::io::stdout().as_raw_fd();

    run_vz_vm(
        kernel,
        initrd,
        disk,
        cidata,
        shared_dir,
        cmdline,
        cpus,
        memory_mb,
        mac,
        in_pipe.0.as_raw_fd(),
        out_fd,
    )?;

    Ok(())
}
