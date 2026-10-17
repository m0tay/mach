//! `norb run` command implementation (transparent command runner inside VM).

use anyhow::{bail, Context, Result};
use norb_common::config::NorbConfig;
use norb_common::machine::MachineState;
use std::os::unix::process::ExitStatusExt;

use super::{find_machine, resolve_target_machine};

pub async fn run(
    machine_opt: Option<&str>,
    user_opt: Option<&str>,
    command: &[String],
) -> Result<()> {
    if command.is_empty() {
        bail!("no command specified to run");
    }

    let cfg = NorbConfig::load()?;
    let mut effective_cmd = command;

    let target_name_opt = match machine_opt {
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
        bail!("no command specified to run");
    }

    let mut machine = resolve_target_machine(&cfg, target_name_opt)?;

    // Auto-start if stopped
    if machine.state != MachineState::Running {
        println!("\x1b[1;36m➜\x1b[0m Starting machine \x1b[1m{}\x1b[0m...", machine.config.name);
        super::start::run(Some(&machine.config.name)).await?;
        machine = find_machine(&cfg.machines_dir, &machine.config.name)?
            .with_context(|| format!("machine '{}' not found after start", machine.config.name))?;
    }

    let ip = machine
        .ip_address
        .with_context(|| format!("machine '{}' has no IP address assigned", machine.config.name))?;

    let user = user_opt.unwrap_or("root");

    let mut cmd = crate::ssh::new_ssh_command(&ip, user);
    cmd.args(effective_cmd);

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
