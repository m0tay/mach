//! `norb list` / `norb ls` command implementation.

use anyhow::Result;
use norb_common::config::NorbConfig;
use norb_common::machine::MachineState;

use super::load_all_machines;

pub async fn run() -> Result<()> {
    let cfg = NorbConfig::load()?;
    let machines = load_all_machines(&cfg.machines_dir)?;

    if machines.is_empty() {
        println!("No machines found.");
        println!("Run \x1b[1;34mnorb create <distro>\x1b[0m to create your first machine (e.g. `norb create ubuntu`).");
        return Ok(());
    }

    // Print table header
    println!(
        "\x1b[1m{:<20} {:<12} {:<10} {:<12} {:<6} {:<10} {:<8} {:<16}\x1b[0m",
        "NAME", "DISTRO", "ARCH", "STATE", "CPUS", "MEMORY", "DISK", "IP ADDRESS"
    );
    println!("{}", "─".repeat(96));

    for m in machines {
        let is_default = cfg.default_machine.as_deref() == Some(&m.config.name);
        let name_display = if is_default {
            format!("\x1b[1m{}\x1b[0m \x1b[33m*\x1b[0m", m.config.name)
        } else {
            m.config.name.clone()
        };

        let state_str = match &m.state {
            MachineState::Running => format!("\x1b[1;32m● running\x1b[0m"),
            MachineState::Starting => format!("\x1b[1;33m◐ starting\x1b[0m"),
            MachineState::Stopping => format!("\x1b[1;33m◑ stopping\x1b[0m"),
            MachineState::Stopped => format!("\x1b[2m○ stopped\x1b[0m"),
            MachineState::Creating => format!("\x1b[36m◌ creating\x1b[0m"),
            MachineState::Error(err) => format!("\x1b[1;31m✖ error: {}\x1b[0m", err),
        };

        let ip_str = m
            .ip_address
            .map(|ip| ip.to_string())
            .unwrap_or_else(|| "-".to_string());

        let mem_str = if m.config.memory_mb >= 1024 {
            format!("{:.1} GiB", m.config.memory_mb as f64 / 1024.0)
        } else {
            format!("{} MiB", m.config.memory_mb)
        };

        let disk_str = format!("{} GiB", m.config.disk_gb);

        let name_pad = if is_default { 20 + 13 } else { 20 };

        println!(
            "{:<name_pad$} {:<12} {:<10} {:<23} {:<6} {:<10} {:<8} {:<16}",
            name_display,
            m.config.distro,
            m.config.arch.to_string(),
            state_str,
            m.config.cpus,
            mem_str,
            disk_str,
            ip_str,
            name_pad = name_pad
        );
    }

    Ok(())
}
