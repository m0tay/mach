//! `norb create` command implementation.

use anyhow::{bail, Result};
use norb_common::config::NorbConfig;
use norb_common::machine::{Architecture, Machine, MachineConfig, MachineState};
use std::process::Command;

use super::{find_machine, save_machine};
use crate::cloud_init;
use crate::images;

pub async fn run(
    distro: &str,
    name: Option<&str>,
    cpus: u32,
    memory: u64,
    disk: u64,
    arch_str: &str,
) -> Result<()> {
    let mut cfg = NorbConfig::load()?;

    let is_machine_taken = |m_name: &str| -> Result<bool> {
        if find_machine(&cfg.machines_dir, m_name)?.is_some() {
            return Ok(true);
        }
        if cfg.machines_dir.join(m_name).exists() {
            return Ok(true);
        }
        Ok(false)
    };

    let machine_name = match name {
        Some(explicit_name) => {
            let m_name = explicit_name.to_lowercase();
            if is_machine_taken(&m_name)? {
                bail!("machine '{m_name}' already exists");
            }
            m_name
        }
        None => {
            let base = distro.to_lowercase();
            if !is_machine_taken(&base)? {
                base
            } else {
                let mut idx = 2;
                loop {
                    let candidate = format!("{base}-{idx}");
                    if !is_machine_taken(&candidate)? {
                        break candidate;
                    }
                    idx += 1;
                }
            }
        }
    };

    let arch = match arch_str.to_lowercase().as_str() {
        "aarch64" | "arm64" => Architecture::Aarch64,
        "x86_64" | "amd64" | "x64" => Architecture::X86_64,
        other => bail!("unsupported architecture '{other}' (valid: aarch64, x86_64)"),
    };

    let distro_lower = distro.to_lowercase();
    let is_alpine = distro_lower == "alpine";
    let distro_info = images::get_distro(&distro_lower);

    if !is_alpine && distro_info.is_none() {
        bail!(
            "unsupported distribution '{distro}'. Supported distributions: alpine, debian, ubuntu, fedora"
        );
    }

    println!("\x1b[1;36m➜\x1b[0m Creating machine \x1b[1m{}\x1b[0m...", machine_name);
    println!("  Distribution: \x1b[33m{}\x1b[0m", distro);
    println!("  Architecture: \x1b[33m{}\x1b[0m", arch);
    println!("  CPUs:         \x1b[33m{}\x1b[0m", cpus);
    println!("  Memory:       \x1b[33m{} MiB\x1b[0m", memory);
    println!("  Disk:         \x1b[33m{} GiB\x1b[0m", disk);

    let machine_cfg = MachineConfig {
        name: machine_name.clone(),
        distro: distro.to_string(),
        arch,
        cpus,
        memory_mb: memory,
        disk_gb: disk,
    };

    let mut machine = Machine::new(machine_cfg);
    machine.state = MachineState::Stopped;

    let machine_dir = cfg.machines_dir.join(&machine_name);
    std::fs::create_dir_all(&machine_dir)?;
    let disk_path = machine_dir.join("disk.raw");

    if is_alpine {
        // Alpine: sparse disk image (formatted on first boot by 2-stage initramfs)
        if !disk_path.exists() {
            let file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&disk_path)?;
            file.set_len(disk * 1024 * 1024 * 1024)?;
        }
    } else if let Some(info) = distro_info {
        // Multi-distro cloud image
        let cache_dir = cfg.data_dir.join("cache");
        let cached_raw = images::ensure_cached_image(&cache_dir, info)?;

        if !disk_path.exists() {
            println!("  \x1b[1;36m➜\x1b[0m Provisioning disk image (APFS instant clone)...");
            let cp_status = Command::new("/bin/cp")
                .args(["-c", cached_raw.to_str().unwrap(), disk_path.to_str().unwrap()])
                .status();

            if cp_status.is_err() || !cp_status.unwrap().success() {
                std::fs::copy(&cached_raw, &disk_path)?;
            }

            // Expand disk to requested size
            let file = std::fs::OpenOptions::new().write(true).open(&disk_path)?;
            file.set_len(disk * 1024 * 1024 * 1024)?;
        }

        // Generate cloud-init seed disk
        let cidata_path = machine_dir.join("cidata.iso");
        println!("  \x1b[1;36m➜\x1b[0m Generating cloud-init seed configuration...");
        cloud_init::generate_cidata_iso(&machine.id, &machine_name, &cidata_path)?;
    }

    save_machine(&cfg.machines_dir, &machine)?;

    if cfg.default_machine.is_none() {
        cfg.default_machine = Some(machine_name.clone());
        cfg.save()?;
    }

    println!(
        "\x1b[1;32m✓\x1b[0m Machine \x1b[1m{}\x1b[0m created successfully (id: {}).",
        machine_name,
        machine.short_id()
    );
    println!("\nRun \x1b[1;34mnorb start {}\x1b[0m to boot the machine.", machine_name);

    Ok(())
}
