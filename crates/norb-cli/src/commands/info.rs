//! `norb info` command implementation.

use anyhow::{bail, Context, Result};
use norb_common::config::NorbConfig;

use super::{find_machine, load_all_machines};

pub async fn run(name: Option<&str>) -> Result<()> {
    let cfg = NorbConfig::load()?;

    let target_name = match name {
        Some(n) => n.to_string(),
        None => {
            let machines = load_all_machines(&cfg.machines_dir)?;
            if machines.is_empty() {
                bail!("no machines found");
            }
            machines[0].config.name.clone()
        }
    };

    let machine = find_machine(&cfg.machines_dir, &target_name)?
        .with_context(|| format!("machine '{target_name}' not found"))?;

    println!("\x1b[1mMachine Information\x1b[0m");
    println!("─────────────────────────────────────────");
    println!("  Name:         \x1b[1m{}\x1b[0m", machine.config.name);
    println!("  UUID:         {}", machine.id);
    println!("  Distribution: {}", machine.config.distro);
    println!("  Architecture: {}", machine.config.arch);
    println!("  State:        {}", machine.state);
    println!("  CPUs:         {}", machine.config.cpus);
    println!("  Memory:       {} MiB", machine.config.memory_mb);
    println!("  Disk:         {} GiB", machine.config.disk_gb);
    println!(
        "  IP Address:   {}",
        machine
            .ip_address
            .map(|ip| ip.to_string())
            .unwrap_or_else(|| "none (stopped)".into())
    );
    println!("  Domain:       {}.norb.local", machine.config.name);
    println!("  Created:      {}", machine.created_at.to_rfc2822());

    Ok(())
}
