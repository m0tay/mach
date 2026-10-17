//! `norb top` command implementation (CLI-first terminal monitor).

use anyhow::Result;
use norb_common::config::NorbConfig;
use norb_common::machine::MachineState;
use std::collections::HashMap;
use std::io::IsTerminal;
use std::path::Path;
use std::time::{Duration, Instant};
use tokio::time::sleep;

use super::load_all_machines;

#[derive(Debug, Clone, Copy)]
struct TaskMetrics {
    resident_bytes: u64,
    total_cpu_ns: u64,
}

#[derive(Debug, Clone, Copy)]
struct CpuSample {
    total_cpu_ns: u64,
    instant: Instant,
}

#[cfg(target_os = "macos")]
fn get_proc_task_info(pid: u32) -> Option<TaskMetrics> {
    unsafe {
        let mut info: libc::proc_taskinfo = std::mem::zeroed();
        let size = std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int;
        let ret = libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTASKINFO,
            0,
            &mut info as *mut _ as *mut libc::c_void,
            size,
        );
        if ret == size {
            Some(TaskMetrics {
                resident_bytes: info.pti_resident_size,
                total_cpu_ns: info.pti_total_user + info.pti_total_system,
            })
        } else {
            None
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn get_proc_task_info(_pid: u32) -> Option<TaskMetrics> {
    None
}

fn get_machine_pid(machines_dir: &Path, m: &norb_common::machine::Machine) -> Option<u32> {
    m.pid.or_else(|| {
        let pid_file = machines_dir.join(&m.config.name).join("pid");
        std::fs::read_to_string(&pid_file)
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
    })
}

pub async fn run(interval_secs: u64) -> Result<()> {
    let cfg = NorbConfig::load()?;
    let interval = Duration::from_secs(interval_secs.max(1));
    let is_interactive = std::io::stdout().is_terminal();

    let mut last_samples: HashMap<u32, CpuSample> = HashMap::new();

    loop {
        // Clear terminal screen and move cursor to top-left if interactive
        if is_interactive {
            print!("\x1b[2J\x1b[H");
        }

        println!(
            "\x1b[1;36mNorb Resource Monitor\x1b[0m — {} — Refresh: {}s",
            chrono::Local::now().format("%H:%M:%S"),
            interval_secs.max(1)
        );
        println!("─────────────────────────────────────────────────────────────────────────────────────────────");

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
            "\x1b[1m{:<16} {:<10} {:<12} {:<8} {:<20} {:<16} {:<16}\x1b[0m",
            "NAME", "DISTRO", "STATE", "CPU %", "MEM USAGE", "DISK USAGE", "IP"
        );
        println!("{}", "─".repeat(93));

        if machines.is_empty() {
            println!("  (No machines configured. Create one with `norb create <distro>`)");
        } else {
            for m in &machines {
                let pid_opt = get_machine_pid(&cfg.machines_dir, m);
                let (cpu_pct, mem_usage) = if m.state == MachineState::Running {
                    if let Some(pid) = pid_opt {
                        if let Some(metrics) = get_proc_task_info(pid) {
                            let rss_mib = metrics.resident_bytes / (1024 * 1024);
                            let mem_str = format!("{} MiB / {} MiB", rss_mib, m.config.memory_mb);
                            let now = Instant::now();
                            let cpu_str = if let Some(prev) = last_samples.get(&pid) {
                                let elapsed_wall_ns = now.duration_since(prev.instant).as_nanos();
                                let delta_cpu_ns = metrics.total_cpu_ns.saturating_sub(prev.total_cpu_ns);
                                if elapsed_wall_ns > 0 {
                                    let pct = (delta_cpu_ns as f64 / elapsed_wall_ns as f64) * 100.0;
                                    format!("{:.1}%", pct)
                                } else {
                                    "0.0%".to_string()
                                }
                            } else {
                                "0.0%".to_string()
                            };
                            last_samples.insert(
                                pid,
                                CpuSample {
                                    total_cpu_ns: metrics.total_cpu_ns,
                                    instant: now,
                                },
                            );
                            (cpu_str, mem_str)
                        } else {
                            ("0.0%".to_string(), format!("0 MiB / {} MiB", m.config.memory_mb))
                        }
                    } else {
                        ("0.0%".to_string(), format!("0 MiB / {} MiB", m.config.memory_mb))
                    }
                } else {
                    ("0.0%".to_string(), format!("0 MiB / {} MiB", m.config.memory_mb))
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

                let disk_path = cfg.machines_dir.join(&m.config.name).join("disk.raw");
                let disk_usage = if let Ok(meta) = std::fs::metadata(&disk_path) {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::MetadataExt;
                        let bytes = meta.blocks() * 512;
                        if bytes >= 1024 * 1024 * 1024 {
                            format!("{:.1} GiB / {} GiB", bytes as f64 / (1024.0 * 1024.0 * 1024.0), m.config.disk_gb)
                        } else {
                            format!("{:.0} MiB / {} GiB", bytes as f64 / (1024.0 * 1024.0), m.config.disk_gb)
                        }
                    }
                    #[cfg(not(unix))]
                    {
                        format!("{} GiB", m.config.disk_gb)
                    }
                } else {
                    format!("{} GiB", m.config.disk_gb)
                };

                println!(
                    "{:<16} {:<10} {:<21} {:<8} {:<20} {:<16} {:<16}",
                    m.config.name,
                    m.config.distro,
                    state_str,
                    cpu_pct,
                    mem_usage,
                    disk_usage,
                    ip_str
                );
            }
        }

        if !is_interactive {
            break;
        }

        println!();
        println!("\x1b[2mPress Ctrl+C to exit.\x1b[0m");

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!();
                break;
            }
            _ = sleep(interval) => {}
        }
    }

    Ok(())
}
