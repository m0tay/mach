//! `mach version` command implementation.

use anyhow::Result;

pub async fn run() -> Result<()> {
    println!("\x1b[1mMach\x1b[0m — Supersonic Linux Virtualization & Container Engine for Apple Silicon");
    println!("  Version:       v{}", env!("CARGO_PKG_VERSION"));
    println!("  Target:        {}-apple-darwin", std::env::consts::ARCH);
    println!("  Hypervisor:    Apple Virtualization.framework");
    println!("  Virtio:        VirtioFS, VirtioNet, VirtioBlock, VirtioConsole");
    println!("  Storage:       Native APFS Copy-on-Write (CoW)");
    println!("  Networking:    User-space NAT with *.mach.local DNS & dynamic port proxy");
    println!("  License:       MIT");
    Ok(())
}
