//! Export and import Norb machines.

use anyhow::{bail, Context, Result};
use norb_common::config::NorbConfig;
use norb_common::machine::{Architecture, Machine, MachineConfig, MachineState};
use std::fs;
use std::path::Path;
use std::process::Command;
use uuid::Uuid;

use crate::commands::{find_machine, save_machine};

pub async fn export(name: &str, out_file: &Path) -> Result<()> {
    let cfg = NorbConfig::load()?;
    let machine = match find_machine(&cfg.machines_dir, name)? {
        Some(m) => m,
        None => bail!("Machine '{}' does not exist.", name),
    };

    if machine.state == MachineState::Running {
        bail!("Machine '{}' is running. Stop it before exporting.", name);
    }

    let disk_path = cfg.machines_dir.join(name).join("disk.raw");
    if !disk_path.exists() {
        bail!("Disk not found for machine '{}'.", name);
    }

    println!("  \x1b[1;36m➜\x1b[0m Exporting disk to {}...", out_file.display());
    let status = Command::new("/bin/cp")
        .args(["-c", disk_path.to_str().unwrap(), out_file.to_str().unwrap()])
        .status();

    if status.is_err() || !status.unwrap().success() {
        fs::copy(&disk_path, out_file)
            .with_context(|| format!("failed to copy disk to {}", out_file.display()))?;
    }

    println!("\x1b[1;32m✓\x1b[0m Successfully exported '{}' to {}", name, out_file.display());
    Ok(())
}

pub async fn import(in_file: &Path, name: &str) -> Result<()> {
    let cfg = NorbConfig::load()?;
    if find_machine(&cfg.machines_dir, name)?.is_some() {
        bail!("Machine '{}' already exists.", name);
    }

    if !in_file.exists() {
        bail!("Input disk file '{}' does not exist.", in_file.display());
    }

    let target_dir = cfg.machines_dir.join(name);
    fs::create_dir_all(&target_dir)?;
    let target_disk = target_dir.join("disk.raw");

    println!("  \x1b[1;36m➜\x1b[0m Importing disk from {}...", in_file.display());
    let status = Command::new("/bin/cp")
        .args(["-c", in_file.to_str().unwrap(), target_disk.to_str().unwrap()])
        .status();

    if status.is_err() || !status.unwrap().success() {
        fs::copy(in_file, &target_disk)
            .with_context(|| format!("failed to copy {} to {}", in_file.display(), target_disk.display()))?;
    }

    let machine = Machine {
        id: Uuid::new_v4(),
        config: MachineConfig {
            name: name.to_string(),
            distro: "custom".to_string(),
            cpus: cfg.default_cpus,
            memory_mb: cfg.default_memory_mb,
            disk_gb: 20,
            arch: Architecture::Aarch64,
        },
        state: MachineState::Stopped,
        ip_address: None,
        pid: None,
        created_at: chrono::Utc::now(),
    };

    save_machine(&cfg.machines_dir, &machine)?;
    println!("\x1b[1;32m✓\x1b[0m Imported machine \x1b[1m{}\x1b[0m successfully.", name);
    println!("Run \x1b[1;34mnorb start {}\x1b[0m to launch it.", name);
    Ok(())
}
