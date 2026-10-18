//! `mach config` command implementation.

use anyhow::Result;
use mach_common::config::MachConfig;

pub async fn show() -> Result<()> {
    let cfg = MachConfig::load()?;
    let json = serde_json::to_string_pretty(&cfg)?;
    println!("{}", json);
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

    match key {
        "default_cpus" => {
            cfg.default_cpus = value.parse()?;
        }
        "default_memory_mb" | "default_memory_mib" => {
            let mib: u64 = value.parse()?;
            cfg.set_memory_mib(mib);
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
            eprintln!("Valid keys: default_cpus, default_memory_mb, default_distro, default_disk_gib, rosetta, log_level, default_machine, docker.was_running, k8s.was_running");
            return Ok(());
        }
    }

    cfg.save()?;
    println!("\x1b[1;32m✓\x1b[0m Config key \x1b[1m{}\x1b[0m set to \x1b[1m{}\x1b[0m.", key, value);
    Ok(())
}
