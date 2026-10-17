//! Machine filesystem mount and Finder integration for Norb.

use anyhow::{bail, Result};
use mach_common::config::NorbConfig;
use mach_common::machine::MachineState;
use std::path::Path;
use std::process::Command;

use crate::commands::find_machine;
use crate::nfs;

/// Mount or open a machine's filesystem in macOS Finder.
/// Establishes Unified Finder Hierarchy (`~/Norb/machines/` and `~/Norb/volumes/`),
/// supports `~/Norb/<machine>` direct mounting, and utilizes native macOS NFS loopback
/// with fallback to sshfs and Finder SFTP.
pub async fn run_mount(name: &str, target: Option<&Path>) -> Result<()> {
    let cfg = NorbConfig::load()?;
    let machine = match find_machine(&cfg.machines_dir, name)? {
        Some(m) => m,
        None => bail!("Machine '{}' does not exist.", name),
    };

    if machine.state != MachineState::Running {
        bail!("Machine '{}' is not running. Start it with `mach start {}` first.", name, name);
    }

    let ip = match machine.ip_address {
        Some(ip) => ip,
        None => bail!("Machine '{}' does not have an active IP address.", name),
    };

    nfs::mount_machine(name, &ip.to_string(), target).await?;
    Ok(())
}

/// Unmount a machine's filesystem if mounted.
#[allow(dead_code)]
pub async fn run_unmount(name: &str, target: Option<&Path>) -> Result<()> {
    let (primary_dir, alias_dir) = nfs::resolve_mount_paths(name, target);
    if nfs::is_mounted(&primary_dir) {
        println!("  \x1b[1;36m➜\x1b[0m Unmounting {}...", primary_dir.display());
        nfs::unmount(&primary_dir, false)?;
        println!("\x1b[1;32m✓\x1b[0m Successfully unmounted {}", primary_dir.display());
    } else {
        println!("  \x1b[1;33m!\x1b[0m {} is not mounted.", primary_dir.display());
    }

    if let Some(alias) = alias_dir {
        if nfs::is_mounted(&alias) {
            let _ = nfs::unmount(&alias, false);
        }
    }

    Ok(())
}

/// Open VS Code Remote SSH session directly into the machine.
pub async fn run_code(name: &str, path: Option<&str>) -> Result<()> {
    let cfg = mach_common::config::MachConfig::load()?;
    let machine = match find_machine(&cfg.machines_dir, name)? {
        Some(m) => m,
        None => bail!("Machine '{}' does not exist.", name),
    };

    if machine.state != MachineState::Running {
        bail!("Machine '{}' is not running. Start it with `mach start {}` first.", name, name);
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
