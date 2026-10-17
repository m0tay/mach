//! Menu command dispatcher for Norb.
//!
//! Launches the native Apple Silicon macOS menu bar status item (` Norb`).

use anyhow::Result;
use std::fs;
use std::path::Path;
use std::process::Command;

/// Check if the menu bar process is currently running.
pub fn is_menu_running(run_dir: &Path) -> Option<u32> {
    let pid_file = run_dir.join("menu.pid");
    if let Ok(content) = fs::read_to_string(&pid_file) {
        if let Ok(pid) = content.trim().parse::<i32>() {
            if nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok() {
                return Some(pid as u32);
            }
        }
    }
    None
}

/// Automatically start the macOS menu bar status item in the background if not already active.
pub fn ensure_menu_bar_running() {
    if let Ok(cfg) = norb_common::config::NorbConfig::load() {
        let run_dir = cfg.data_dir.join("run");
        if is_menu_running(&run_dir).is_none() {
            let log_dir = cfg.data_dir.join("logs");
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
                    if let Ok(child) = Command::new(exe)
                        .arg("menu")
                        .stdout(log_file.try_clone().unwrap_or_else(|_| log_file))
                        .stderr(std::process::Stdio::null())
                        .stdin(std::process::Stdio::null())
                        .process_group(0)
                        .spawn()
                    {
                        let _ = fs::write(run_dir.join("menu.pid"), child.id().to_string());
                    }
                }
            }
        }
    }
}

/// Execute the macOS menu bar status item application.
pub async fn run() -> Result<()> {
    if let Ok(cfg) = norb_common::config::NorbConfig::load() {
        let run_dir = cfg.data_dir.join("run");
        let _ = fs::create_dir_all(&run_dir);
        let _ = fs::write(run_dir.join("menu.pid"), std::process::id().to_string());
    }
    let res = crate::menu::run_menu_bar();
    if let Ok(cfg) = norb_common::config::NorbConfig::load() {
        let _ = fs::remove_file(cfg.data_dir.join("run").join("menu.pid"));
    }
    res
}
