//! `mach config` command implementation.

use anyhow::Result;
use mach_common::config::MachConfig;

pub async fn show() -> Result<()> {
    let cfg = MachConfig::load()?;
    let json = serde_json::to_string_pretty(&cfg)?;
    println!("{}", json);
    Ok(())
}

pub async fn get(key: &str) -> Result<()> {
    let cfg = MachConfig::load()?;
    match key {
        "default_cpus" => println!("{}", cfg.default_cpus),
        "default_memory_mb" | "default_memory_mib" => println!("{}", cfg.default_memory_mib),
        "system_memory_mb" | "system_memory_mib" | "system_memory" | "docker.memory_mb" | "k8s.memory_mb" => {
            println!("{}", cfg.system_memory_mib());
        }
        "default_distro" => println!("{}", cfg.default_distro),
        "default_disk_gib" | "default_disk_gb" => println!("{}", cfg.default_disk_gib),
        "rosetta" => println!("{}", cfg.rosetta),
        "log_level" => println!("{}", cfg.log_level),
        "default_machine" => println!("{}", cfg.default_machine.as_deref().unwrap_or("none")),
        "docker.was_running" | "docker.autostart" => println!("{}", cfg.docker.was_running),
        "docker.enabled" => println!("{}", cfg.docker.enabled),
        "k8s.was_running" | "k8s.autostart" => println!("{}", cfg.k8s.was_running),
        other => {
            eprintln!("\x1b[1;31mUnknown config key: {}\x1b[0m", other);
            eprintln!("Valid keys: default_cpus, default_memory_mb, system_memory_mb, default_distro, default_disk_gib, rosetta, log_level, default_machine, docker.was_running, k8s.was_running");
        }
    }
    Ok(())
}

pub async fn set(key: &str, value: &str) -> Result<()> {
    let mut cfg = MachConfig::load()?;

    let parse_bool = |val: &str| -> Result<bool> {
        match val.to_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Ok(true),
            "false" | "0" | "no" | "off" => Ok(false),
            _ => anyhow::bail!("invalid boolean value '{}' (expected true/false)", val),
        }
    };

    let mut applied_sys_memory = false;

    match key {
        "default_cpus" => {
            cfg.default_cpus = value.parse()?;
        }
        "default_memory_mb" | "default_memory_mib" => {
            let mib: u64 = value.parse()?;
            cfg.set_memory_mib(mib);
        }
        "system_memory_mb" | "system_memory_mib" | "system_memory" | "docker.memory_mb" | "docker.memory_mib" | "k8s.memory_mb" | "k8s.memory_mib" => {
            let mib: u64 = value.parse()?;
            if mib < 1024 {
                anyhow::bail!("system memory must be at least 1024 MiB (e.g. 4096)");
            }
            cfg.set_system_memory_mib(mib);
            applied_sys_memory = true;
        }
        "default_distro" => {
            cfg.default_distro = value.to_string();
        }
        "default_disk_gib" | "default_disk_gb" => {
            cfg.default_disk_gib = value.parse()?;
        }
        "rosetta" => {
            cfg.rosetta = parse_bool(value)?;
        }
        "log_level" => {
            cfg.log_level = value.to_string();
        }
        "default_machine" => {
            cfg.default_machine = if value.is_empty() || value == "none" {
                None
            } else {
                Some(value.to_string())
            };
        }
        "docker.was_running" | "docker.autostart" | "docker_was_running" => {
            cfg.docker.was_running = parse_bool(value)?;
        }
        "docker.enabled" => {
            cfg.docker.enabled = parse_bool(value)?;
        }
        "k8s.was_running" | "k8s.autostart" | "k8s_was_running" => {
            cfg.k8s.was_running = parse_bool(value)?;
        }
        other => {
            eprintln!("\x1b[1;31mUnknown config key: {}\x1b[0m", other);
            eprintln!("Valid keys: default_cpus, default_memory_mb, system_memory_mb, default_distro, default_disk_gib, rosetta, log_level, default_machine, docker.was_running, k8s.was_running");
            return Ok(());
        }
    }

    cfg.save()?;
    println!("\x1b[1;32m✓\x1b[0m Config key \x1b[1m{}\x1b[0m set to \x1b[1m{}\x1b[0m.", key, value);

    if applied_sys_memory {
        let mib = cfg.system_memory_mib();
        if let Ok(Some(mut m)) = super::find_machine(&cfg.machines_dir, super::docker::DOCKER_VM_NAME) {
            m.config.memory_mb = mib;
            let _ = m.save(&cfg.machines_dir);
            if m.state == mach_common::machine::MachineState::Running {
                println!("  \x1b[1;33mTip:\x1b[0m mach-docker is currently running. Run 'machctl docker restart' to apply the new memory allocation.");
            }
        }
    }

    Ok(())
}
