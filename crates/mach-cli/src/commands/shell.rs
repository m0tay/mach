//! `mach shell` command implementation.

use anyhow::{Context, Result};
use mach_common::config::MachConfig;
use mach_common::machine::MachineState;
use std::os::unix::process::ExitStatusExt;

pub async fn run(name: Option<&str>, system: bool, command: &[String]) -> Result<()> {
    run_with_options(name, None, system, None, command).await
}

pub async fn run_with_options(name: Option<&str>, user: Option<&str>, system: bool, workdir: Option<&str>, command: &[String]) -> Result<()> {
    let cfg = MachConfig::load()?;

    let cfg = if !system && name.is_none() {
        let all_machines = super::load_all_machines(&cfg.machines_dir)?;
        let user_machines: Vec<_> = all_machines.into_iter().filter(|m| !m.is_system()).collect();

        if user_machines.is_empty() {
            eprintln!("No Linux machine found. Preparing your environment...");
            crate::commands::setup::run_streamlined(true).await?;
            eprintln!("\x1b[1;32m✓\x1b[0m Environment ready!\n");
            MachConfig::load()?
        } else {
            cfg
        }
    } else {
        cfg
    };

    let mut machine = super::resolve_target_machine_ext(&cfg, name, system)?;

    if machine.state != MachineState::Running {
        eprintln!("\x1b[1;36m➜\x1b[0m Starting machine \x1b[1m{}\x1b[0m...", machine.config.name);
        super::start::run_ext(Some(&machine.config.name), true).await?;
        machine = super::find_machine(&cfg.machines_dir, &machine.config.name)?
            .with_context(|| format!("machine '{}' not found after start", machine.config.name))?;
    }

    let ip = machine
        .ip_address
        .with_context(|| format!("machine '{}' does not have an IP address", machine.config.name))?;

    let mut cmd = crate::ssh::new_session_command(&ip, user.unwrap_or("root"));
    cmd.arg(crate::ssh::session_script(command, workdir)?);

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
