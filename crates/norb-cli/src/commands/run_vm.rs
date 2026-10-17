//! `norb run-vm` internal background VMM executor.

use anyhow::Result;
use norb_vmm::run_vz_vm;
use std::os::unix::io::AsRawFd;
use std::path::Path;

pub fn run(
    name: &str,
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

    // Determine if the machine is configured for isolated sandbox mode.
    // If isolated == true, omit shared directory to prevent host directory access.
    let is_isolated = std::env::args().any(|arg| arg == "--isolated")
        || norb_common::config::NorbConfig::load()
            .ok()
            .and_then(|cfg| {
                let meta_path = cfg.machines_dir.join(name).join("machine.json");
                std::fs::read_to_string(meta_path)
                    .ok()
                    .and_then(|s| serde_json::from_str::<norb_common::machine::Machine>(&s).ok())
            })
            .map(|m| m.config.isolated)
            .unwrap_or(false);

    let effective_shared_dir = if is_isolated {
        eprintln!("[norb-vmm] Machine '{name}' running in isolated sandbox mode (host directory sharing disabled)");
        None
    } else {
        shared_dir
    };

    // Use an unread pipe for stdin and stdout (vm.log) for console output
    let in_pipe = nix::unistd::pipe()?;
    let out_fd = std::io::stdout().as_raw_fd();

    run_vz_vm(
        kernel,
        initrd,
        disk,
        cidata,
        effective_shared_dir,
        cmdline,
        cpus,
        memory_mb,
        mac,
        in_pipe.0.as_raw_fd(),
        out_fd,
    )?;

    Ok(())
}
