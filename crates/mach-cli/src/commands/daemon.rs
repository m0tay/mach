//! Background daemon manager for Norb DNS and dynamic port forwarding.

use anyhow::{Context, Result};
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use mach_common::config::NorbConfig;
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

use crate::dns;
use crate::port_forward;

pub async fn run_foreground() -> Result<()> {
    let cfg = NorbConfig::load()?;
    let run_dir = cfg.data_dir.join("run");
    fs::create_dir_all(&run_dir)?;

    let docker_sock = run_dir.join("docker.sock");
    let machines_dir = cfg.machines_dir.clone();

    println!("[mach-daemon] Starting embedded DNS responder on 127.0.0.1:5354...");
    println!("[mach-daemon] Starting native Apple Silicon mDNS responder (*.mach.local)...");
    println!("[mach-daemon] Starting Docker dynamic port forwarder...");
    println!("[mach-daemon] Starting HTTP/HTTPS reverse proxy on 127.0.0.1:80/443 (*.mach.local)...");
    println!("[mach-daemon] Starting macOS host command bridge...");

    let m_dir = machines_dir.clone();
    let dns_task = tokio::spawn(async move {
        loop {
            if let Err(e) = dns::run_dns_server(&m_dir).await {
                eprintln!("[mach-daemon] DNS responder error: {e:?}. Restarting in 1s...");
            } else {
                eprintln!("[mach-daemon] DNS responder stopped unexpectedly. Restarting in 1s...");
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });

    let m_dir_mdns = machines_dir.clone();
    let mdns_task = tokio::spawn(async move {
        crate::mdns::run_mdns_loop(&m_dir_mdns).await;
    });

    let ds_clone = docker_sock.clone();
    let m_dir2 = machines_dir.clone();
    let forwarder_task = tokio::spawn(async move {
        loop {
            if let Err(e) = port_forward::run_port_forwarder(&ds_clone, &m_dir2).await {
                eprintln!("[mach-daemon] Port forwarder error: {e:?}. Restarting in 1s...");
            } else {
                eprintln!("[mach-daemon] Port forwarder stopped unexpectedly. Restarting in 1s...");
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });

    let ds_clone2 = docker_sock.clone();
    let m_dir3 = machines_dir.clone();
    let proxy_task = tokio::spawn(async move {
        loop {
            if let Err(e) = crate::proxy::run_reverse_proxy(&ds_clone2, &m_dir3).await {
                eprintln!("[mach-daemon] Reverse proxy error: {e:?}. Restarting in 1s...");
            } else {
                eprintln!("[mach-daemon] Reverse proxy stopped unexpectedly. Restarting in 1s...");
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });

    let rd_clone = run_dir.clone();
    let bridge_task = tokio::spawn(async move {
        loop {
            if let Err(e) = crate::bridge::run_host_bridge(&rd_clone).await {
                eprintln!("[mach-daemon] Host bridge error: {e:?}. Restarting in 1s...");
            } else {
                eprintln!("[mach-daemon] Host bridge stopped unexpectedly. Restarting in 1s...");
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });

    let _ = tokio::join!(dns_task, mdns_task, forwarder_task, proxy_task, bridge_task);
    Ok(())
}

pub fn is_daemon_running(run_dir: &Path) -> Option<u32> {
    let pid_file = run_dir.join("daemon.pid");
    if let Ok(content) = fs::read_to_string(&pid_file) {
        if let Ok(pid) = content.trim().parse::<i32>() {
            if kill(Pid::from_raw(pid), None).is_ok() {
                return Some(pid as u32);
            }
        }
    }
    None
}

pub fn start_daemon() -> Result<()> {
    let cfg = NorbConfig::load()?;
    let run_dir = cfg.data_dir.join("run");
    let log_dir = cfg.data_dir.join("logs");
    fs::create_dir_all(&run_dir)?;
    fs::create_dir_all(&log_dir)?;

    if let Some(pid) = is_daemon_running(&run_dir) {
        println!("\x1b[1;32m✓\x1b[0m Norb daemon is already running (PID: {}).", pid);
        return Ok(());
    }

    let exe = std::env::current_exe()?;
    let log_file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_dir.join("daemon.log"))?;

    let child = Command::new(exe)
        .args(["daemon", "run"])
        .stdout(log_file.try_clone()?)
        .stderr(log_file)
        .stdin(std::process::Stdio::null())
        .process_group(0)
        .spawn()
        .with_context(|| "failed to spawn background daemon")?;

    let pid = child.id();
    fs::write(run_dir.join("daemon.pid"), pid.to_string())?;

    println!("\x1b[1;32m✓\x1b[0m Started Mach background daemon (PID: {}).", pid);
    println!("  DNS Server:    \x1b[1;34m127.0.0.1:5354\x1b[0m (*.mach.local)");
    println!("  Port Proxy:    \x1b[1;34mlocalhost:PORT\x1b[0m (auto-forwarded)");
    println!("  Daemon Logs:   {}", log_dir.join("daemon.log").display());

    Ok(())
}

pub fn stop_daemon() -> Result<()> {
    let cfg = NorbConfig::load()?;
    let run_dir = cfg.data_dir.join("run");
    let pid_file = run_dir.join("daemon.pid");

    if let Some(pid) = is_daemon_running(&run_dir) {
        let _ = kill(Pid::from_raw(pid as i32), Signal::SIGTERM);
        let _ = fs::remove_file(&pid_file);
        println!("\x1b[1;32m✓\x1b[0m Stopped Mach background daemon (PID: {}).", pid);
    } else {
        println!("Mach daemon is not currently running.");
    }

    Ok(())
}

pub fn status_daemon() -> Result<()> {
    let cfg = NorbConfig::load()?;
    let run_dir = cfg.data_dir.join("run");

    if let Some(pid) = is_daemon_running(&run_dir) {
        println!("\x1b[1;32m●\x1b[0m Mach daemon is \x1b[1;32mrunning\x1b[0m (PID: {}).", pid);
        println!("  DNS Responder: \x1b[1;34m127.0.0.1:5354\x1b[0m");
        println!("  Docker Proxy:  \x1b[1;34mActive\x1b[0m");
    } else {
        println!("\x1b[1;31m○\x1b[0m Mach daemon is \x1b[1mstopped\x1b[0m.");
    }

    let (configured, hint) = dns::check_or_setup_resolver();
    if configured {
        println!("  macOS DNS:     \x1b[1;32mConfigured\x1b[0m (/etc/resolver/mach.local)");
    } else {
        println!("  macOS DNS:     \x1b[1;33mNot Configured\x1b[0m");
        if let Some(cmd) = hint {
            println!("  \x1b[2mRun:\x1b[0m {cmd}");
        }
    }

    Ok(())
}

/// Helper to ensure the background daemon is running without noisy output if already active.
pub fn ensure_daemon_running() {
    let cfg = match NorbConfig::load() {
        Ok(c) => c,
        Err(_) => return,
    };
    let run_dir = cfg.data_dir.join("run");
    if is_daemon_running(&run_dir).is_none() {
        let _ = start_daemon();
    }
}
