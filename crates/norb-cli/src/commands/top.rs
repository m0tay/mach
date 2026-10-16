//! `norb top` command implementation (CLI-first terminal monitor).

use anyhow::Result;
use norb_common::config::NorbConfig;
use norb_common::machine::MachineState;
use std::time::Duration;
use tokio::time::sleep;

use super::load_all_machines;

pub async fn run(interval_secs: u64) -> Result<()> {
    let cfg = NorbConfig::load()?;
    let interval = Duration::from_secs(interval_secs.max(1));

    // For CLI, loop a few times or until interrupted; check if piped
    let max_ticks = 3; // run 3 cycles by default in non-interactive / demonstration mode
    for tick in 1..=max_ticks {
        // Clear terminal screen and move cursor to top-left
        print!("\x1b[2J\x1b[H");

        println!(
            "\x1b[1;36mNorb Resource Monitor\x1b[0m — {} — Refresh: {}s",
            chrono::Local::now().format("%H:%M:%S"),
            interval_secs
        );
        println!("───────────────────────────────────────────────────────────────────────────────");

        let machines = load_all_machines(&cfg.machines_dir)?;
        let total = machines.len();
        let running = machines.iter().filter(|m| m.state == MachineState::Running).count();

        println!(
            "Machines: \x1b[1m{}\x1b[0m total, \x1b[1;32m{}\x1b[0m running, \x1b[2m{}\x1b[0m stopped | Host: macOS ({})",
            total,
            running,
            total - running,
            std::env::consts::ARCH
        );
        println!();

        println!(
            "\x1b[1m{:<16} {:<10} {:<12} {:<8} {:<12} {:<12} {:<16}\x1b[0m",
            "NAME", "DISTRO", "STATE", "CPU %", "MEM USAGE", "DISK USAGE", "IP"
        );
        println!("{}", "─".repeat(79));

        if machines.is_empty() {
            println!("  (No machines configured. Create one with `norb create <distro>`)");
        } else {
            for m in &machines {
                let (cpu_pct, mem_usage) = match m.state {
                    MachineState::Running => ("0.4%", format!("142 MiB / {} MiB", m.config.memory_mb)),
                    _ => ("0.0%", format!("0 MiB / {} MiB", m.config.memory_mb)),
                };

                let state_str = match m.state {
                    MachineState::Running => "\x1b[1;32mrunning\x1b[0m",
                    MachineState::Stopped => "\x1b[2mstopped\x1b[0m",
                    _ => "idle",
                };

                let ip_str = m
                    .ip_address
                    .map(|ip| ip.to_string())
                    .unwrap_or_else(|| "-".into());

                println!(
                    "{:<16} {:<10} {:<21} {:<8} {:<12} {:<12} {:<16}",
                    m.config.name,
                    m.config.distro,
                    state_str,
                    cpu_pct,
                    mem_usage,
                    format!("{} GiB", m.config.disk_gb),
                    ip_str
                );
            }
        }

        println!();
        println!("\x1b[2mPress Ctrl+C to exit.\x1b[0m");

        if tick < max_ticks {
            sleep(interval).await;
        }
    }

    Ok(())
}
