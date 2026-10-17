//! Export and import Norb machines.

use anyhow::{bail, Context, Result};
use norb_common::config::NorbConfig;
use norb_common::machine::{Architecture, Machine, MachineConfig, MachineState};
use std::fs;
use std::path::{Path, PathBuf};
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

    // Write sidecar <path>.norb-meta with JSON machine config
    let sidecar_path = PathBuf::from(format!("{}.norb-meta", out_file.display()));
    let meta_json = serde_json::to_string_pretty(&machine.config)?;
    let _ = fs::write(&sidecar_path, meta_json);

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

    // Restore original vCPU, RAM, distro settings from sidecar if present
    let sidecar_path = PathBuf::from(format!("{}.norb-meta", in_file.display()));
    let alt_sidecar_path = in_file.with_extension("norb-meta");
    let maybe_config: Option<MachineConfig> = if sidecar_path.exists() {
        fs::read_to_string(&sidecar_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
    } else if alt_sidecar_path.exists() {
        fs::read_to_string(&alt_sidecar_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
    } else {
        None
    };

    let (distro, cpus, memory_mb, disk_gb, arch) = if let Some(saved) = maybe_config {
        (saved.distro, saved.cpus, saved.memory_mb, saved.disk_gb, saved.arch)
    } else {
        let disk_gb = fs::metadata(&target_disk)
            .map(|m| (m.len() / (1024 * 1024 * 1024)).max(1))
            .unwrap_or(20);
        ("custom".to_string(), cfg.default_cpus, cfg.default_memory_mb, disk_gb, Architecture::Aarch64)
    };

    let machine = Machine {
        id: Uuid::new_v4(),
        config: MachineConfig {
            name: name.to_string(),
            distro: distro.clone(),
            cpus,
            memory_mb,
            disk_gb,
            arch,
        },
        state: MachineState::Stopped,
        ip_address: None,
        pid: None,
        created_at: chrono::Utc::now(),
    };

    save_machine(&cfg.machines_dir, &machine)?;

    if !distro.eq_ignore_ascii_case("alpine") && !distro.eq_ignore_ascii_case("custom") {
        let cidata_path = target_dir.join("cidata.iso");
        let _ = crate::cloud_init::generate_cidata_iso(&machine.id, name, &cidata_path);
    }

    println!("\x1b[1;32m✓\x1b[0m Imported machine \x1b[1m{}\x1b[0m successfully.", name);
    println!("Run \x1b[1;34mnorb start {}\x1b[0m to launch it.", name);
    Ok(())
}
