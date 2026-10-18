//! Menu command dispatcher for Mach.
//!
//! Launches the native Apple Silicon macOS menu bar status item.
//! Stays active in the macOS menu bar to provide real-time status and quick controls.

use anyhow::Result;
use mach_common::config::MachConfig;
use std::fs;
use std::path::Path;
use std::process::Command;

/// Check if the menu bar process is currently running.
pub fn is_menu_running(run_dir: &Path) -> Option<u32> {
    let pid_file = run_dir.join("menu.pid");
    if let Ok(content) = fs::read_to_string(&pid_file) {
        if let Ok(pid) = content.trim().parse::<i32>() {
            if nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok() {
                // Verify the pid is actually a running mach menu process
                if let Ok(out) = Command::new("ps").args(["-p", &pid.to_string(), "-o", "command="]).output() {
                    let cmdline = String::from_utf8_lossy(&out.stdout);
                    if cmdline.contains("mach") && cmdline.contains("menu") {
                        return Some(pid as u32);
                    }
                }
            }
        }
    }
    None
}

/// Terminate any running macOS menu bar status item process.
pub fn stop_menu_bar() {
    let my_pid = std::process::id();

    // 1. Check PID file in data_dir and ~/.mach/run fallback
    let mut check_dirs = Vec::new();
    if let Ok(cfg) = MachConfig::load() {
        check_dirs.push(cfg.data_dir.join("run"));
    }
    if let Some(home) = dirs::home_dir() {
        check_dirs.push(home.join(".mach/run"));
    }
    for run_dir in check_dirs {
        if let Some(pid) = is_menu_running(&run_dir) {
            if pid != my_pid {
                let _ = nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(pid as i32),
                    nix::sys::signal::Signal::SIGKILL,
                );
            }
            let _ = fs::remove_file(run_dir.join("menu.pid"));
        }
    }

    // 2. Scan process table for any running `mach menu` or `machctl menu`
    if let Ok(output) = Command::new("ps").args(["-eo", "pid,command"]).output() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let has_menu = line.contains("mach menu")
                || line.contains("machctl menu")
                || (line.contains("mach") && line.contains(" menu"));
            if has_menu && !line.contains("grep") {
                if let Some(pid_str) = line.split_whitespace().next() {
                    if let Ok(pid) = pid_str.parse::<i32>() {
                        if pid as u32 != my_pid {
                            let _ = nix::sys::signal::kill(
                                nix::unistd::Pid::from_raw(pid),
                                nix::sys::signal::Signal::SIGKILL,
                            );
                        }
                    }
                }
            }
        }
    }

    // 3. Direct pkill to terminate any lingering menu bar worker instances
    let _ = Command::new("pkill").args(["-9", "-f", "mach menu"]).output();
    let _ = Command::new("pkill").args(["-9", "-f", "machctl menu"]).output();
}

/// Automatically start the macOS menu bar status item in the background if not already active.
pub fn ensure_menu_bar_running() {
    let (run_dir, log_dir) = if let Ok(cfg) = MachConfig::load() {
        (cfg.data_dir.join("run"), cfg.data_dir.join("logs"))
    } else if let Some(home) = dirs::home_dir() {
        (home.join(".mach/run"), home.join(".mach/logs"))
    } else {
        return;
    };

    if is_menu_running(&run_dir).is_none() {
        let _ = fs::create_dir_all(&run_dir);
        let _ = fs::create_dir_all(&log_dir);
        if let Ok(exe) = std::env::current_exe() {
            let log_path = log_dir.join("menu.log");
            if let Ok(log_file) = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
            {
                use std::os::unix::process::CommandExt;
                let log_err = log_file.try_clone().ok();
                let mut cmd = Command::new(exe);
                cmd.arg("menu")
                    .stdout(log_file)
                    .stdin(std::process::Stdio::null())
                    .process_group(0);
                if let Some(err_file) = log_err {
                    cmd.stderr(err_file);
                } else {
                    cmd.stderr(std::process::Stdio::null());
                }
                if let Ok(child) = cmd.spawn() {
                    let _ = fs::write(run_dir.join("menu.pid"), child.id().to_string());
                }
            }
        }
    }
}

/// Execute the macOS menu bar status item application.
pub async fn run() -> Result<()> {
    if let Ok(cfg) = MachConfig::load() {
        let run_dir = cfg.data_dir.join("run");
        let _ = fs::create_dir_all(&run_dir);
        let _ = fs::write(run_dir.join("menu.pid"), std::process::id().to_string());
    }
    let res = crate::menu::run_menu_bar();
    if let Ok(cfg) = MachConfig::load() {
        let _ = fs::remove_file(cfg.data_dir.join("run").join("menu.pid"));
    }
    res
}
