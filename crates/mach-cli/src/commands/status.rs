//! `mach status` command implementation.

use anyhow::Result;
use mach_common::config::MachConfig;
use mach_common::machine::MachineState;

use super::load_all_machines;

pub async fn run() -> Result<()> {
    let cfg = MachConfig::load()?;
    let machines = load_all_machines(&cfg.machines_dir)?;

    let total = machines.len();
    let running = machines.iter().filter(|m| m.state == MachineState::Running).count();
    let stopped = machines.iter().filter(|m| m.state == MachineState::Stopped).count();

    println!("\x1b[1mmach system status\x1b[0m");
    println!("─────────────────────────────────────────");
    println!("  Version:            v{}", env!("CARGO_PKG_VERSION"));
    println!("  Hypervisor Engine:  Apple Hypervisor.framework (Rust native VMM)");
    println!("  Architecture:       {}", std::env::consts::ARCH);
    println!("  Total Machines:     {}", total);
    println!("  Running Machines:   \x1b[1;32m{}\x1b[0m", running);
    println!("  Stopped Machines:   {}", stopped);
    println!("  Data Directory:     {}", cfg.data_dir.display());
    println!("  Machines Directory: {}", cfg.machines_dir.display());

    Ok(())
}
