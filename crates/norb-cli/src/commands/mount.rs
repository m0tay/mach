//! Machine filesystem mount and Finder integration for Norb.

use anyhow::{bail, Context, Result};
use norb_common::config::NorbConfig;
use norb_common::machine::MachineState;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::commands::find_machine;

/// Mount or open a machine's filesystem in macOS Finder.
pub async fn run_mount(name: &str, target: Option<&Path>) -> Result<()> {
    let cfg = NorbConfig::load()?;
    let machine = match find_machine(&cfg.machines_dir, name)? {
        Some(m) => m,
        None => bail!("Machine '{}' does not exist.", name),
    };

    if machine.state != MachineState::Running {
        bail!("Machine '{}' is not running. Start it with `norb start {}` first.", name, name);
    }

    let ip = match machine.ip_address {
        Some(ip) => ip,
        None => bail!("Machine '{}' does not have an active IP address.", name),
    };

    let mount_dir = if let Some(t) = target {
        t.to_path_buf()
    } else {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/Users/Shared"))
            .join("Norb")
            .join(name)
    };

    fs::create_dir_all(&mount_dir)?;

    // Check if sshfs is installed on macOS
    let which_sshfs = Command::new("which").arg("sshfs").output();
    let has_sshfs = which_sshfs.map(|o| o.status.success()).unwrap_or(false);

    if has_sshfs {
        println!("  \x1b[1;36m➜\x1b[0m Mounting {} filesystem at {} via sshfs...", name, mount_dir.display());
        let st = Command::new("sshfs")
            .args([
                &format!("root@{ip}:/"),
                mount_dir.to_str().unwrap(),
                "-o", "StrictHostKeyChecking=no",
                "-o", "UserKnownHostsFile=/dev/null",
                "-o", "reconnect",
                "-o", "follow_symlinks",
            ])
            .status()
            .with_context(|| "failed to run sshfs")?;

        if st.success() {
            println!("\x1b[1;32m✓\x1b[0m Successfully mounted \x1b[1m{}\x1b[0m to {}", name, mount_dir.display());
            let _ = Command::new("open").arg(&mount_dir).spawn();
            return Ok(());
        }
    }

    // Native macOS Finder SFTP / Network browser integration
    println!("  \x1b[1;36m➜\x1b[0m Opening \x1b[1m{}\x1b[0m ({}) in macOS Network Browser...", name, ip);
    let sftp_url = format!("sftp://root@{ip}/");
    let status = Command::new("open")
        .arg(&sftp_url)
        .status();

    if status.is_ok() && status.unwrap().success() {
        println!("\x1b[1;32m✓\x1b[0m Opened network share in macOS Finder!");
    } else {
        println!("  To browse guest files from Finder, connect via SFTP:");
        println!("  \x1b[1;34m{}\x1b[0m", sftp_url);
    }

    Ok(())
}

/// Open VS Code Remote SSH session directly into the machine.
pub async fn run_code(name: &str, path: Option<&str>) -> Result<()> {
    let cfg = NorbConfig::load()?;
    let machine = match find_machine(&cfg.machines_dir, name)? {
        Some(m) => m,
        None => bail!("Machine '{}' does not exist.", name),
    };

    if machine.state != MachineState::Running {
        bail!("Machine '{}' is not running. Start it with `norb start {}` first.", name, name);
    }

    let ip = match machine.ip_address {
        Some(ip) => ip,
        None => bail!("Machine '{}' does not have an active IP address.", name),
    };

    let target_path = path.unwrap_or("/root");
    let target_arg = format!("ssh-remote+root@{ip}{target_path}");

    println!("  \x1b[1;36m➜\x1b[0m Opening VS Code connected to \x1b[1m{}\x1b[0m ({})...", name, ip);
    let status = Command::new("code")
        .args(["--remote", &target_arg])
        .status();

    match status {
        Ok(st) if st.success() => {
            println!("\x1b[1;32m✓\x1b[0m VS Code remote session launched!");
        }
        _ => {
            println!("\x1b[1;33m!\x1b[0m 'code' CLI not found in PATH or failed to launch.");
            println!("  Connect manually in VS Code using SSH host: root@{}", ip);
        }
    }

    Ok(())
}
