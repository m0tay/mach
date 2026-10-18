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
    command: &[String],
) -> Result<()> {
    if command.is_empty() {
        return super::shell::run(machine_opt, system, &[]).await;
    }

    let cfg = MachConfig::load()?;
    let mut effective_cmd = command;

    let mut target_name_opt = match machine_opt {
        Some(n) => Some(n),
        None => {
            if command.len() > 1 && find_machine(&cfg.machines_dir, &command[0])?.is_some() {
                effective_cmd = &command[1..];
                Some(command[0].as_str())
            } else {
                None
            }
        }
    };

    if effective_cmd.is_empty() {
        return super::shell::run(target_name_opt, system, &[]).await;
    }

    let cfg = if !system {
        let all_machines = super::load_all_machines(&cfg.machines_dir)?;
        let user_machines: Vec<_> = all_machines.into_iter().filter(|m| !m.is_system()).collect();

        if user_machines.is_empty() {
            eprintln!("No Linux machine found. Preparing your environment...");
            crate::commands::setup::run_streamlined(true).await?;
            eprintln!("\x1b[1;32m✓\x1b[0m Environment ready");
            let reloaded_cfg = MachConfig::load()?;
            if target_name_opt.is_none() && effective_cmd.len() > 1 && find_machine(&reloaded_cfg.machines_dir, &effective_cmd[0])?.is_some() {
                target_name_opt = Some(effective_cmd[0].as_str());
                effective_cmd = &effective_cmd[1..];
            }
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

    let mut status = None;
    for attempt in 0..6 {
        let mut cmd = crate::ssh::new_ssh_command(&ip, user);
        cmd.args(effective_cmd);
        if let Ok(st) = cmd.status() {
            if st.success() || attempt == 5 || st.code() != Some(255) {
                status = Some(st);
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }

    let status = status.context("failed to execute ssh command")?;
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
