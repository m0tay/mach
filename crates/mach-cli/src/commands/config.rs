//! `norb config` command implementation.

use anyhow::Result;
use mach_common::config::NorbConfig;

pub async fn show() -> Result<()> {
    let cfg = NorbConfig::load()?;
    let json = serde_json::to_string_pretty(&cfg)?;
    println!("{}", json);
    Ok(())
}

pub async fn set(key: &str, value: &str) -> Result<()> {
    let mut cfg = NorbConfig::load()?;

    match key {
        "default_cpus" => {
            cfg.default_cpus = value.parse()?;
        }
        "default_memory_mb" => {
            cfg.default_memory_mb = value.parse()?;
        }
        "log_level" => {
            cfg.log_level = value.to_string();
        }
        "default_machine" => {
            cfg.default_machine = Some(value.to_string());
        }
        other => {
            eprintln!("\x1b[1;31mUnknown config key: {}\x1b[0m", other);
            eprintln!("Valid keys: default_cpus, default_memory_mb, log_level, default_machine");
            return Ok(());
        }
    }

    cfg.save()?;
    println!("\x1b[1;32m✓\x1b[0m Config key \x1b[1m{}\x1b[0m set to \x1b[1m{}\x1b[0m.", key, value);
    Ok(())
}
