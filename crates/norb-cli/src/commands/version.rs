//! `norb version` command implementation.

use anyhow::Result;

pub async fn run() -> Result<()> {
    println!("\x1b[1mNorb\x1b[0m — Rust-based macOS VM & Container Platform");
    println!("  Version:       v{}", env!("CARGO_PKG_VERSION"));
    println!("  Target:        {}-apple-darwin", std::env::consts::ARCH);
    println!("  Hypervisor:    Apple Hypervisor.framework");
    println!("  Virtio:        Modern Virtio 1.2 MMIO (console, block, net)");
    println!("  Storage:       APFS sparse images with dynamic TRIM");
    println!("  Networking:    User-space virtual switch with .norb.local DNS");
    println!("  License:       MIT");
    Ok(())
}
