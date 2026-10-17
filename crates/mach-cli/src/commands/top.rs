//! `mach top` command implementation (CLI-first terminal monitor).

use anyhow::Result;
use mach_common::config::MachConfig;
use mach_common::machine::MachineState;
use std::collections::HashMap;
use std::io::{IsTerminal, Write};
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

#[derive(Debug, Clone)]
struct FsStats {
    fstype: String,
    total_bytes: u64,
    _free_bytes: u64,
    avail_bytes: u64,
    used_bytes: u64,
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

#[cfg(target_os = "macos")]
fn get_fs_stats(path: &Path) -> Option<FsStats> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let target_path = if path.exists() {
        path.to_path_buf()
    } else {
        path.parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| Path::new("/").to_path_buf())
    };

    let c_path = CString::new(target_path.as_os_str().as_bytes()).ok()?;
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    let ret = unsafe { libc::statfs(c_path.as_ptr(), &mut stat) };
    if ret == 0 {
        let bsize = stat.f_bsize as u64;
        let total_bytes = stat.f_blocks * bsize;
        let free_bytes = stat.f_bfree * bsize;
        let avail_bytes = stat.f_bavail * bsize;
        let used_bytes = total_bytes.saturating_sub(free_bytes);
        let fstype = unsafe {
            std::ffi::CStr::from_ptr(stat.f_fstypename.as_ptr())
                .to_string_lossy()
                .into_owned()
        };
        Some(FsStats {
            fstype,
            total_bytes,
            _free_bytes: free_bytes,
            avail_bytes,
            used_bytes,
        })
    } else {
        None
    }
}

#[cfg(not(target_os = "macos"))]
fn get_fs_stats(_path: &Path) -> Option<FsStats> {
    None
}

fn get_terminal_width() -> usize {
    #[cfg(target_os = "macos")]
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0 {
            return ws.ws_col as usize;
        }
    }
    100
}

fn get_machine_pid(machines_dir: &Path, m: &mach_common::machine::Machine) -> Option<u32> {
    m.pid.or_else(|| {
        let pid_file = machines_dir.join(&m.config.name).join("pid");
        std::fs::read_to_string(&pid_file)
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
    })
}

fn format_bytes(bytes: u64) -> String {
    const GIB: f64 = (1024 * 1024 * 1024) as f64;
    const MIB: f64 = (1024 * 1024) as f64;
    const KIB: f64 = 1024.0;

    let b = bytes as f64;
    if b >= GIB {
        format!("{:.1} GiB", b / GIB)
    } else if b >= MIB {
        format!("{:.0} MiB", b / MIB)
    } else if b >= KIB {
        format!("{:.0} KiB", b / KIB)
    } else {
        format!("{} B", bytes)
    }
}

fn render_bar(used: u64, total: u64, width: usize, colorize: bool) -> String {
    if total == 0 {
        return format!("[{}]", "░".repeat(width));
    }
    let ratio = (used as f64 / total as f64).clamp(0.0, 1.0);
    let filled = (ratio * width as f64).round() as usize;
    let empty = width.saturating_sub(filled);

    if !colorize {
        return format!("[{}{}]", "■".repeat(filled), "░".repeat(empty));
    }

    let color = if ratio < 0.60 {
        "\x1b[32m" // green
    } else if ratio < 0.85 {
        "\x1b[33m" // yellow
    } else {
        "\x1b[1;31m" // red
    };

    format!(
        "{color}[{}{}\x1b[0m{color}]\x1b[0m",
        "■".repeat(filled),
        format!("\x1b[90m{}\x1b[0m", "░".repeat(empty))
    )
}

struct CursorGuard {
    active: bool,
}

impl CursorGuard {
    fn new(is_interactive: bool) -> Self {
        if is_interactive {
            print!("\x1b[?25l"); // Hide cursor
            let _ = std::io::stdout().flush();
        }
        Self {
            active: is_interactive,
        }
    }
}

impl Drop for CursorGuard {
    fn drop(&mut self) {
        if self.active {
            print!("\x1b[?25h\x1b[0m\n"); // Restore cursor & reset color
            let _ = std::io::stdout().flush();
        }
    }
}

pub async fn run(interval_secs: u64) -> Result<()> {
    let cfg = MachConfig::load()?;
    let interval = Duration::from_secs(interval_secs.max(1));
    let is_interactive = std::io::stdout().is_terminal();

    let _cursor_guard = CursorGuard::new(is_interactive);
    let mut last_samples: HashMap<u32, CpuSample> = HashMap::new();
    let mut frame_count: u64 = 0;

    loop {
        // Flicker-free refresh: clear screen on frame 0, home cursor on subsequent frames
        if is_interactive {
            if frame_count == 0 {
                print!("\x1b[2J\x1b[H");
            } else {
                print!("\x1b[H");
            }
        }

        let term_width = get_terminal_width().max(80);
        let divider_width = term_width.min(108);
        let show_bars = term_width >= 104;

        let machines = load_all_machines(&cfg.machines_dir)?;
        let total = machines.len();
        let running = machines
            .iter()
            .filter(|m| m.state == MachineState::Running)
            .count();
        let stopped = total - running;

        // Query host APFS filesystem pool statistics
        let host_fs = get_fs_stats(&cfg.machines_dir);

        // Calculate aggregated metrics
        let mut total_cpu_pct = 0.0;
        let mut total_rss_bytes = 0u64;
        let mut total_alloc_ram_bytes = 0u64;
        let mut total_phys_disk_bytes = 0u64;
        let mut total_prov_disk_bytes = 0u64;

        struct MachineRow {
            name: String,
            distro: String,
            state: MachineState,
            pid: Option<u32>,
            cpu_pct: f64,
            rss_bytes: u64,
            alloc_bytes: u64,
            phys_disk_bytes: u64,
            prov_disk_bytes: u64,
            ip_str: String,
        }

        let now = Instant::now();
        let mut rows = Vec::with_capacity(machines.len());

        for m in &machines {
            let pid_opt = get_machine_pid(&cfg.machines_dir, m);
            let mut cpu_pct = 0.0;
            let mut rss_bytes = 0u64;
            let alloc_bytes = m.config.memory_mb * 1024 * 1024;

            if m.state == MachineState::Running {
                total_alloc_ram_bytes += alloc_bytes;
                if let Some(pid) = pid_opt {
                    if let Some(metrics) = get_proc_task_info(pid) {
                        rss_bytes = metrics.resident_bytes;
                        total_rss_bytes += rss_bytes;

                        if let Some(prev) = last_samples.get(&pid) {
                            let elapsed_wall_ns = now.duration_since(prev.instant).as_nanos();
                            let delta_cpu_ns =
                                metrics.total_cpu_ns.saturating_sub(prev.total_cpu_ns);
                            if elapsed_wall_ns > 0 {
                                cpu_pct =
                                    (delta_cpu_ns as f64 / elapsed_wall_ns as f64) * 100.0;
                                total_cpu_pct += cpu_pct;
                            }
                        }
                        last_samples.insert(
                            pid,
                            CpuSample {
                                total_cpu_ns: metrics.total_cpu_ns,
                                instant: now,
                            },
                        );
                    }
                }
            }

            // APFS physical allocated bytes vs provisioned capacity
            let disk_path = cfg.machines_dir.join(&m.config.name).join("disk.raw");
            let (phys_disk_bytes, prov_disk_bytes) = if let Ok(meta) = std::fs::metadata(&disk_path)
            {
                #[cfg(unix)]
                let phys = {
                    use std::os::unix::fs::MetadataExt;
                    meta.blocks() * 512
                };
                #[cfg(not(unix))]
                let phys = meta.len();

                let prov = (m.config.disk_gb * 1024 * 1024 * 1024).max(meta.len());
                (phys, prov)
            } else {
                (0, m.config.disk_gb * 1024 * 1024 * 1024)
            };

            total_phys_disk_bytes += phys_disk_bytes;
            total_prov_disk_bytes += prov_disk_bytes;

            let ip_str = m
                .ip_address
                .map(|ip| ip.to_string())
                .unwrap_or_else(|| "-".into());

            let distro_display = if m.is_system() {
                format!("{} (sys)", m.system_service_name().unwrap_or("sys").to_lowercase())
            } else {
                m.config.distro.clone()
            };

            rows.push(MachineRow {
                name: m.config.name.clone(),
                distro: distro_display,
                state: m.state.clone(),
                pid: pid_opt,
                cpu_pct,
                rss_bytes,
                alloc_bytes,
                phys_disk_bytes,
                prov_disk_bytes,
                ip_str,
            });
        }

        // Clean up last_samples for deceased PIDs
        let active_pids: Vec<u32> = rows.iter().filter_map(|r| r.pid).collect();
        last_samples.retain(|pid, _| active_pids.contains(pid));

        let total_cow_savings = total_prov_disk_bytes.saturating_sub(total_phys_disk_bytes);
        let guest_rss_pct = if total_alloc_ram_bytes > 0 {
            (total_rss_bytes as f64 / total_alloc_ram_bytes as f64) * 100.0
        } else {
            0.0
        };

        // Render Dashboard Header
        println!(
            "\x1b[1;36mmach top\x1b[0m ── \x1b[1m{}\x1b[0m ── Refresh: {}s \x1b[90m│\x1b[0m Host: macOS Apple Silicon ({}) \x1b[90m│\x1b[0m VMM: Apple Virtualization",
            chrono::Local::now().format("%H:%M:%S"),
            interval_secs.max(1),
            std::env::consts::ARCH
        );
        println!("{}", "─".repeat(divider_width));

        // Machine & Memory Aggregates Summary
        println!(
            "Machines: \x1b[1m{}\x1b[0m total (\x1b[1;32m● {} running\x1b[0m, \x1b[2m○ {} stopped\x1b[0m)  \x1b[90m│\x1b[0m  Guest CPU: \x1b[1m{:.1}%\x1b[0m  \x1b[90m│\x1b[0m  Guest RSS: \x1b[1m{}\x1b[0m / {} ({:.1}%)",
            total,
            running,
            stopped,
            total_cpu_pct,
            format_bytes(total_rss_bytes),
            format_bytes(total_alloc_ram_bytes),
            guest_rss_pct
        );

        // APFS Copy-on-Write Storage Aggregates
        let host_free_str = host_fs
            .as_ref()
            .map(|s| format_bytes(s.avail_bytes))
            .unwrap_or_else(|| "N/A".into());
        println!(
            "APFS Disk: \x1b[1m{}\x1b[0m on disk vs \x1b[1m{}\x1b[0m prov (\x1b[1;32m{} saved via CoW\x1b[0m)  \x1b[90m│\x1b[0m  Host Free: \x1b[1m{}\x1b[0m (APFS)",
            format_bytes(total_phys_disk_bytes),
            format_bytes(total_prov_disk_bytes),
            format_bytes(total_cow_savings),
            host_free_str
        );
        println!("{}", "─".repeat(divider_width));

        // Table Header
        if show_bars {
            println!(
                "\x1b[1m{:<14} {:<10} {:<12} {:<7} {:<8} {:<33} {:<32} {:<15}\x1b[0m",
                "NAME", "DISTRO", "STATE", "PID", "CPU %", "MEMORY (RSS / ALLOC)", "APFS DISK (USED/PROV)", "IP ADDRESS"
            );
        } else {
            println!(
                "\x1b[1m{:<14} {:<10} {:<12} {:<7} {:<8} {:<24} {:<24} {:<15}\x1b[0m",
                "NAME", "DISTRO", "STATE", "PID", "CPU %", "MEMORY (RSS / ALLOC)", "APFS DISK (USED/PROV)", "IP ADDRESS"
            );
        }
        println!("{}", "─".repeat(divider_width));

        if rows.is_empty() {
            println!("  \x1b[2m(No machines configured. Create one with `mach create <distro>`)\x1b[0m");
        } else {
            for row in rows {
                let pid_str = row
                    .pid
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "-".into());

                let state_str = match row.state {
                    MachineState::Running => "\x1b[1;32m● running\x1b[0m",
                    MachineState::Starting => "\x1b[1;33m◐ starting\x1b[0m",
                    MachineState::Stopping => "\x1b[1;33m◑ stopping\x1b[0m",
                    MachineState::Stopped => "\x1b[2m○ stopped\x1b[0m",
                    MachineState::Creating => "\x1b[1;34m◌ creating\x1b[0m",
                    MachineState::Error(_) => "\x1b[1;31m✖ error\x1b[0m",
                };

                let cpu_str = if row.state == MachineState::Running {
                    let color = if row.cpu_pct < 50.0 {
                        "\x1b[32m"
                    } else if row.cpu_pct < 85.0 {
                        "\x1b[33m"
                    } else {
                        "\x1b[1;31m"
                    };
                    format!("{color}{:>6.1}%\x1b[0m", row.cpu_pct)
                } else {
                    format!("\x1b[2m  0.0%\x1b[0m")
                };

                let mem_pct = if row.alloc_bytes > 0 {
                    (row.rss_bytes as f64 / row.alloc_bytes as f64) * 100.0
                } else {
                    0.0
                };
                let mem_str = if show_bars {
                    let bar = render_bar(row.rss_bytes, row.alloc_bytes, 6, is_interactive);
                    format!(
                        "{} {:>4} / {:>4} MiB ({:>5.1}%)",
                        bar,
                        row.rss_bytes / (1024 * 1024),
                        row.alloc_bytes / (1024 * 1024),
                        mem_pct
                    )
                } else {
                    format!(
                        "{:>4} / {:>4} MiB ({:>5.1}%)",
                        row.rss_bytes / (1024 * 1024),
                        row.alloc_bytes / (1024 * 1024),
                        mem_pct
                    )
                };

                let disk_pct = if row.prov_disk_bytes > 0 {
                    (row.phys_disk_bytes as f64 / row.prov_disk_bytes as f64) * 100.0
                } else {
                    0.0
                };
                let disk_str = if show_bars {
                    let bar =
                        render_bar(row.phys_disk_bytes, row.prov_disk_bytes, 6, is_interactive);
                    format!(
                        "{} {:>4.1} / {:>4.1} GiB ({:>4.1}%)",
                        bar,
                        row.phys_disk_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                        row.prov_disk_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                        disk_pct
                    )
                } else {
                    format!(
                        "{:>4.1} / {:>4.1} GiB ({:>4.1}%)",
                        row.phys_disk_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                        row.prov_disk_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                        disk_pct
                    )
                };

                let name_truncated = if row.name.len() > 13 {
                    format!("{}…", &row.name[..12])
                } else {
                    row.name
                };
                let distro_truncated = if row.distro.len() > 9 {
                    format!("{}…", &row.distro[..8])
                } else {
                    row.distro
                };

                if show_bars {
                    println!(
                        "{:<14} {:<10} {} {:<7} {} {} {} {:<15}",
                        name_truncated,
                        distro_truncated,
                        super::pad_to_width(state_str, 12),
                        pid_str,
                        super::pad_to_width(&cpu_str, 8),
                        super::pad_to_width(&mem_str, 33),
                        super::pad_to_width(&disk_str, 32),
                        row.ip_str
                    );
                } else {
                    println!(
                        "{:<14} {:<10} {} {:<7} {} {} {} {:<15}",
                        name_truncated,
                        distro_truncated,
                        super::pad_to_width(state_str, 12),
                        pid_str,
                        super::pad_to_width(&cpu_str, 8),
                        super::pad_to_width(&mem_str, 24),
                        super::pad_to_width(&disk_str, 24),
                        row.ip_str
                    );
                }
            }
        }

        println!("{}", "─".repeat(divider_width));

        // Footer with APFS pool stats and shortcuts
        if let Some(ref fs) = host_fs {
            let free_pct = if fs.total_bytes > 0 {
                (fs.avail_bytes as f64 / fs.total_bytes as f64) * 100.0
            } else {
                0.0
            };
            println!(
                "Host Pool: {} used / {} total ({} avail, {:.1}% free) \x1b[90m│\x1b[0m Type: {} (Native CoW)",
                format_bytes(fs.used_bytes),
                format_bytes(fs.total_bytes),
                format_bytes(fs.avail_bytes),
                free_pct,
                fs.fstype.to_uppercase()
            );
        }

        println!(
            "\x1b[2m[Ctrl+C] Exit  •  Memory compaction supported via drop_caches\x1b[0m"
        );

        if is_interactive {
            print!("\x1b[J"); // Clear any leftover rows below
            let _ = std::io::stdout().flush();
        }

        if !is_interactive {
            break;
        }

        frame_count += 1;

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                break;
            }
            _ = sleep(interval) => {}
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2048), "2 KiB");
        assert_eq!(format_bytes(10 * 1024 * 1024), "10 MiB");
        assert_eq!(format_bytes(5 * 1024 * 1024 * 1024), "5.0 GiB");
    }

    #[test]
    fn test_render_bar() {
        let bar_empty = render_bar(0, 100, 10, false);
        assert_eq!(bar_empty, "[░░░░░░░░░░]");

        let bar_full = render_bar(100, 100, 10, false);
        assert_eq!(bar_full, "[■■■■■■■■■■]");

        let bar_half = render_bar(50, 100, 10, false);
        assert_eq!(bar_half, "[■■■■■░░░░░]");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_get_fs_stats() {
        let stats = get_fs_stats(Path::new("/")).expect("failed to get root fs stats on macOS");
        assert!(stats.total_bytes > 0);
        assert!(!stats.fstype.is_empty());
    }
}

