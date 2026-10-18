//! `mach shell` command implementation.

use anyhow::{Context, Result};
use mach_common::config::MachConfig;
use mach_common::machine::MachineState;
use std::os::unix::process::ExitStatusExt;

pub async fn run(name: Option<&str>, system: bool, command: &[String]) -> Result<()> {
    let cfg = MachConfig::load()?;

    let cfg = if !system && name.is_none() {
        let all_machines = super::load_all_machines(&cfg.machines_dir)?;
        let user_machines: Vec<_> = all_machines.into_iter().filter(|m| !m.is_system()).collect();

        if user_machines.is_empty() {
            println!("No Linux machine found. Preparing your environment...");
            crate::commands::setup::run(false).await?;
            println!("\x1b[1;32m✓\x1b[0m Environment ready!\n");
            MachConfig::load()?
        } else {
            cfg
        }
    } else {
        cfg
    };

    let mut machine = super::resolve_target_machine_ext(&cfg, name, system)?;

    if machine.state != MachineState::Running {
        println!("\x1b[1;36m➜\x1b[0m Starting machine \x1b[1m{}\x1b[0m...", machine.config.name);
        super::start::run(Some(&machine.config.name)).await?;
        machine = super::find_machine(&cfg.machines_dir, &machine.config.name)?
            .with_context(|| format!("machine '{}' not found after start", machine.config.name))?;
    }

    let ip = machine
        .ip_address
        .with_context(|| format!("machine '{}' does not have an IP address", machine.config.name))?;

    let mut cmd = crate::ssh::new_ssh_command(&ip, "root");

    if !command.is_empty() {
        cmd.args(command);
    } else {
        println!(
            "\x1b[1;36m➜\x1b[0m Connecting to \x1b[1m{}\x1b[0m ({})...",
            machine.config.name, ip
        );
    }

    let status = cmd.status()?;
    if !status.success() {
        if let Some(code) = status.code() {
            std::process::exit(code);
        } else if let Some(sig) = status.signal() {
            std::process::exit(128 + sig);
        } else {
            std::process::exit(1);
        }
    }

    Ok(())
}
