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
        println!("{}", "─".repeat(98));

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

            println!(
                "{} {:<12} {:<10} {} {:<6} {:<10} {:<8} {:<16}",
                super::pad_to_width(&name_display, 20),
                m.config.distro,
                m.config.arch.to_string(),
                super::pad_to_width(&state_str, 12),
                m.config.cpus,
                mem_str,
                disk_str,
                ip_str
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
            println!("{}", "─".repeat(106));

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

                let details = if m.config.name == "mach-system" || m.config.name == "mach-docker" {
                    if cfg.k8s.was_running {
                        "Docker Engine & K8s Cluster"
                    } else {
                        "Docker Engine (docker.sock)"
                    }
                } else if m.config.name.contains("k8s") || m.config.name.contains("kubernetes") {
                    "Kubernetes API (:26443)"
                } else {
                    "Protected background service"
                };

                println!(
                    "{:<18} {:<14} {} {:<10} {:<16} {:<32}",
                    m.config.name,
                    svc,
                    super::pad_to_width(&state_str, 12),
                    mem_str,
                    ip_str,
                    details
                );
            }
            println!("\x1b[2m  Tip: To debug system machines, run `machctl docker shell` or `machctl k8s shell`.\x1b[0m");
        }
    } else if !sys_machines.is_empty() {
        let running_sys: Vec<_> = sys_machines.iter().filter(|m| m.state == MachineState::Running).collect();
        let stopped_sys: Vec<_> = sys_machines.iter().filter(|m| m.state != MachineState::Running).collect();
        let running_count = running_sys.len();
        let stopped_count = stopped_sys.len();

        let tip = if stopped_count == 0 {
            let names = running_sys
                .iter()
                .map(|m| m.config.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            if running_count == 1 {
                format!("Tip: 1 system infrastructure machine ({}) is running. Pass --system to show them.", names)
            } else {
                format!("Tip: {} system infrastructure machines ({}) are running. Pass --system to show them.", running_count, names)
            }
        } else if running_count == 0 {
            if stopped_count == 1 {
                "Tip: 1 system infrastructure machine is stopped. Pass --system to show them.".to_string()
            } else {
                format!("Tip: {} system infrastructure machines are stopped. Pass --system to show them.", stopped_count)
            }
        } else {
            let run_noun = if running_count == 1 { "machine" } else { "machines" };
            format!(
                "Tip: {} system infrastructure {} running, {} stopped. Pass --system to show them.",
                running_count, run_noun, stopped_count
            )
        };

        println!("\x1b[2m  {}\x1b[0m", tip);
    }

    Ok(())
}
