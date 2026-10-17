//! `mach list` / `mach ls` command implementation.

use anyhow::Result;
use mach_common::config::MachConfig;
use mach_common::machine::MachineState;

use super::load_all_machines;

pub async fn run(all: bool, show_system: bool) -> Result<()> {
    let cfg = MachConfig::load()?;
    let machines = load_all_machines(&cfg.machines_dir)?;

    let user_machines: Vec<_> = machines.iter().filter(|m| !m.is_system()).cloned().collect();
    let sys_machines: Vec<_> = machines.iter().filter(|m| m.is_system()).cloned().collect();

    if user_machines.is_empty() && sys_machines.is_empty() {
        println!("No machines found.");
        println!("Run \x1b[1;34mmach create <distro>\x1b[0m to create your first machine (e.g. `mach create ubuntu`).");
        return Ok(());
    }

    // 1. User Machines section
    if user_machines.is_empty() {
        println!("\x1b[1;36mUSER MACHINES\x1b[0m");
        println!("No user machines created yet.");
        println!("Run \x1b[1;34mmach create <distro>\x1b[0m to create a Linux machine (e.g. `mach create debian` or `mach create ubuntu`).\n");
    } else {
        println!("\x1b[1;36mUSER MACHINES\x1b[0m");
        println!(
            "\x1b[1m{:<20} {:<12} {:<10} {:<12} {:<6} {:<10} {:<8} {:<16}\x1b[0m",
            "NAME", "DISTRO", "ARCH", "STATE", "CPUS", "MEMORY", "DISK", "IP ADDRESS"
        );
        println!("{}", "─".repeat(96));

        for m in &user_machines {
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
        println!();
    }

    // 2. System Infrastructure Machines (Protected)
    if show_system || all {
        if !sys_machines.is_empty() {
            println!("\x1b[1;35mSYSTEM INFRASTRUCTURE (Protected)\x1b[0m");
            println!(
                "\x1b[1m{:<18} {:<14} {:<12} {:<10} {:<16} {:<32}\x1b[0m",
                "NAME", "SERVICE", "STATE", "MEMORY", "IP ADDRESS", "DETAILS"
            );
            println!("{}", "─".repeat(96));

            for m in &sys_machines {
                let svc = m.system_service_name().unwrap_or("System");
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

                let details = if m.config.name == "mach-docker" {
                    "Docker Engine (docker.sock)"
                } else if m.config.name.contains("k8s") || m.config.name.contains("kubernetes") {
                    "Kubernetes API (:26443)"
                } else {
                    "Protected background service"
                };

                println!(
                    "{:<18} {:<14} {:<23} {:<10} {:<16} {:<32}",
                    m.config.name,
                    svc,
                    state_str,
                    mem_str,
                    ip_str,
                    details
                );
            }
            println!("\x1b[2m  Tip: To debug system machines, run `machctl docker shell` or `machctl k8s shell`.\x1b[0m");
        }
    } else if !sys_machines.is_empty() {
        let count = sys_machines.len();
        let names = sys_machines
            .iter()
            .map(|m| m.config.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let (noun, verb) = if count == 1 {
            ("machine", "is running")
        } else {
            ("machines", "are running")
        };
        println!(
            "\x1b[2m  Tip: {} system infrastructure {} ({}) {}. Pass --system to show them.\x1b[0m",
            count, noun, names, verb
        );
    }

    Ok(())
}
