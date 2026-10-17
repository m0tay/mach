//! `norb boot` command implementation.
//!
//! Directly exercises the low-level `norb-vmm` hypervisor engine to boot a Linux kernel.

use anyhow::Result;
use mach_vmm::{Vmm, VmmConfig};
use std::io::Write;
use std::path::Path;

pub async fn run(
    kernel: &Path,
    initrd: Option<&Path>,
    cmdline: &str,
    memory_mb: u64,
    cpus: u32,
) -> Result<()> {
    println!("\x1b[1;36m➜\x1b[0m Preparing Norb VMM for direct kernel boot...");
    println!("  Kernel:   \x1b[33m{}\x1b[0m", kernel.display());
    if let Some(i) = initrd {
        println!("  Initrd:   \x1b[33m{}\x1b[0m", i.display());
    }
    println!("  Cmdline:  \x1b[33m{}\x1b[0m", cmdline);
    println!("  Memory:   \x1b[33m{} MiB\x1b[0m", memory_mb);
    println!("  CPUs:     \x1b[33m{}\x1b[0m", cpus);
    println!("─────────────────────────────────────────────────────────────\n");

    let config = VmmConfig {
        kernel_path: kernel.to_path_buf(),
        initrd_path: initrd.map(|p| p.to_path_buf()),
        cmdline: cmdline.to_string(),
        cpus,
        memory_mb,
    };

    let mut vmm = Vmm::new(config)?;

    // Spawn async task to stream console output to terminal stdout
    if let Some(mut rx) = vmm.take_console_output() {
        tokio::spawn(async move {
            let mut stdout = std::io::stdout();
            while let Some(bytes) = rx.recv().await {
                let _ = stdout.write_all(&bytes);
                let _ = stdout.flush();
            }
        });
    }

    // Run VMM
    vmm.run().await?;

    println!("\n\x1b[1;32m✓\x1b[0m Guest execution completed.");
    Ok(())
}
