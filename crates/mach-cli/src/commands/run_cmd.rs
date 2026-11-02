//! `mach run` command implementation (transparent command runner inside VM).

use anyhow::{Context, Result};
use mach_common::config::MachConfig;
use mach_common::machine::MachineState;
use std::os::unix::process::ExitStatusExt;

use super::find_machine;

pub async fn run(
    machine_opt: Option<&str>,
    user_opt: Option<&str>,
    system: bool,
    workdir: Option<&str>,
    command: &[String],
) -> Result<()> {
    if command.is_empty() {
        return super::shell::run_with_options(machine_opt, user_opt, system, workdir, &[]).await;
    }

    let cfg = MachConfig::load()?;
    let effective_cmd = command;
    let target_name_opt = machine_opt;

    let cfg = if !system && machine_opt.is_none() {
        let all_machines = super::load_all_machines(&cfg.machines_dir)?;
        let user_machines: Vec<_> = all_machines.into_iter().filter(|m| !m.is_system()).collect();

        if user_machines.is_empty() {
            eprintln!("No Linux machine found. Preparing your environment...");
            crate::commands::setup::run_streamlined(true).await?;
            eprintln!("\x1b[1;32m✓\x1b[0m Environment ready");
            let reloaded_cfg = MachConfig::load()?;
            reloaded_cfg
        } else {
            cfg
        }
    } else {
        cfg
    };

    let mut machine = super::resolve_target_machine_ext(&cfg, target_name_opt, system)?;

    // Auto-start if stopped: send all progress to stderr for clean stdout composability
    if machine.state != MachineState::Running {
        eprintln!("\x1b[1;36m➜\x1b[0m Starting machine \x1b[1m{}\x1b[0m...", machine.config.name);
        super::start::run_ext(Some(&machine.config.name), true).await?;
        machine = find_machine(&cfg.machines_dir, &machine.config.name)?
            .with_context(|| format!("machine '{}' not found after start", machine.config.name))?;
    }

    let ip = machine
        .ip_address
        .with_context(|| format!("machine '{}' has no IP address assigned", machine.config.name))?;

    let user = user_opt.unwrap_or("root");

    // Execute exactly once: exit 255 can be the guest program's own exit status.
    let mut cmd = crate::ssh::new_session_command(&ip, user);
    cmd.arg(crate::ssh::session_script(effective_cmd, workdir)?);
    let status = cmd.status().context("failed to execute ssh command")?;
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
