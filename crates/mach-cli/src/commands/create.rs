//! `mach create` command implementation.

use anyhow::{bail, Result};
use mach_common::config::MachConfig;
use mach_common::machine::{Architecture, Machine, MachineConfig, MachineState};
use std::collections::HashSet;

use super::save_machine;
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
    let isolated = std::env::args().any(|arg| arg == "--isolated");
    run_ext(distro, name, cpus, memory, disk, arch_str, isolated, true, false).await
}

pub async fn run_with_isolated(
    distro: &str,
    name: Option<&str>,
    cpus: u32,
    memory: u64,
    disk: u64,
    arch_str: &str,
    isolated: bool,
) -> Result<()> {
    run_ext(distro, name, cpus, memory, disk, arch_str, isolated, true, false).await
}

pub async fn run_ext(
    distro: &str,
    name: Option<&str>,
    cpus: u32,
    memory: u64,
    disk: u64,
    arch_str: &str,
    isolated: bool,
    show_next_steps: bool,
    to_stderr: bool,
) -> Result<()> {
    macro_rules! out_ln {
        ($($arg:tt)*) => {
            if to_stderr {
                eprintln!($($arg)*);
            } else {
                println!($($arg)*);
            }
        };
    }

    let mut cfg = MachConfig::load()?;

    let mut existing_names = HashSet::new();
    if cfg.machines_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&cfg.machines_dir) {
            for entry in entries.flatten() {
                if let Ok(entry_name) = entry.file_name().into_string() {
                    existing_names.insert(entry_name.to_lowercase());
                }
            }
        }
    }
    if let Ok(machines) = super::load_all_machines(&cfg.machines_dir) {
        for m in machines {
            existing_names.insert(m.config.name.to_lowercase());
        }
    }

    let machine_name = match name {
        Some(explicit_name) => {
            let m_name = explicit_name.to_lowercase();
            if existing_names.contains(&m_name) {
                bail!("machine '{m_name}' already exists");
            }
            m_name
        }
        None => {
            let base = distro.to_lowercase();
            if !existing_names.contains(&base) {
                base
            } else {
                let mut idx = 2;
                loop {
                    let candidate = format!("{base}-{idx}");
                    if !existing_names.contains(&candidate) {
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

    out_ln!("\x1b[1;36m➜\x1b[0m Creating machine \x1b[1m{}\x1b[0m...", machine_name);
    out_ln!("  Distribution: \x1b[33m{}\x1b[0m", distro);
    out_ln!("  Architecture: \x1b[33m{}\x1b[0m", arch);
    out_ln!("  CPUs:         \x1b[33m{}\x1b[0m", cpus);
    out_ln!("  Memory:       \x1b[33m{} MiB\x1b[0m", memory);
    out_ln!("  Disk:         \x1b[33m{} GiB\x1b[0m", disk);
    if isolated {
        out_ln!("  Isolated:     \x1b[33mtrue (host file sharing disabled)\x1b[0m");
    }

    let is_sys = machine_name == "mach-docker"
        || machine_name == "mach-k8s"
        || machine_name == "mach-kubernetes";

    let machine_cfg = MachineConfig {
        name: machine_name.clone(),
        distro: distro.to_string(),
        arch,
        cpus,
        memory_mb: memory,
        disk_gb: disk,
        isolated,
        is_system: is_sys,
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
        let cached_raw = images::ensure_cached_image_ext(&cache_dir, info, to_stderr).await?;

        if !disk_path.exists() {
            out_ln!("  \x1b[1;36m➜\x1b[0m Provisioning disk image (APFS instant clone)...");
            mach_common::fs::apfs_clone(&cached_raw, &disk_path)?;

            // Expand disk to requested size
            let file = std::fs::OpenOptions::new().write(true).open(&disk_path)?;
            file.set_len(disk * 1024 * 1024 * 1024)?;
        }

        // Generate cloud-init seed disk
        let cidata_path = machine_dir.join("cidata.iso");
        out_ln!("  \x1b[1;36m➜\x1b[0m Generating cloud-init seed configuration...");
        cloud_init::generate_cidata_iso(&machine.id, &machine_name, &cidata_path)?;
    }

    save_machine(&cfg.machines_dir, &machine)?;

    if !is_sys && cfg.default_machine.is_none() {
        cfg.default_machine = Some(machine_name.clone());
        cfg.save()?;
    }

    out_ln!(
        "\x1b[1;32m✓\x1b[0m Machine \x1b[1m{}\x1b[0m created successfully (id: {}).",
        machine_name,
        machine.short_id()
    );
    if show_next_steps {
        out_ln!("\nNext steps:");
        out_ln!("  ➜ To start machine:   \x1b[1;34mmach start {}\x1b[0m", machine_name);
        out_ln!("  ➜ To open shell:      \x1b[1;32mmach\x1b[0m (or \x1b[1;32mmach shell {}\x1b[0m)", machine_name);
        out_ln!("  ➜ To run commands:    \x1b[1;32mmach uname -a\x1b[0m");
        out_ln!("  ➜ To view dashboard:  \x1b[1;34mmachctl top\x1b[0m");
    }

    Ok(())
}
