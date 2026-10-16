//! `norb run` command implementation (transparent command runner inside VM).

use anyhow::{bail, Context, Result};
use norb_common::config::NorbConfig;
use norb_common::machine::MachineState;
use std::process::Command;

use super::{find_machine, load_all_machines};

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
    let target_name = match machine_opt {
        Some(n) => n.to_string(),
        None => {
            if command.len() > 1 && find_machine(&cfg.machines_dir, &command[0])?.is_some() {
                let name = command[0].clone();
                effective_cmd = &command[1..];
                name
            } else {
                let machines = load_all_machines(&cfg.machines_dir)?;
                if let Some(ref def) = cfg.default_machine {
                    if machines.iter().any(|m| &m.config.name == def) {
                        def.clone()
                    } else if let Some(running) = machines.iter().find(|m| m.state == MachineState::Running) {
                        running.config.name.clone()
                    } else if !machines.is_empty() {
                        machines[0].config.name.clone()
                    } else {
                        bail!("no machines found. Run `norb create <distro>` first.");
                    }
                } else {
                    let running: Vec<_> = machines
                        .iter()
                        .filter(|m| m.state == MachineState::Running)
                        .collect();
                    if running.is_empty() {
                        if !machines.is_empty() {
                            machines[0].config.name.clone()
                        } else {
                            bail!("no machines found. Run `norb create <distro>` first.");
                        }
                    } else {
                        running[0].config.name.clone()
                    }
                }
            }
        }
    };

    if effective_cmd.is_empty() {
        bail!("no command specified to run");
    }

    let mut machine = find_machine(&cfg.machines_dir, &target_name)?
        .with_context(|| format!("machine '{target_name}' not found"))?;

    // Auto-start if stopped
    if machine.state != MachineState::Running {
        println!("\x1b[1;36m➜\x1b[0m Starting machine \x1b[1m{}\x1b[0m...", target_name);
        super::start::run(Some(&target_name)).await?;
        machine = find_machine(&cfg.machines_dir, &target_name)?.unwrap();
    }

    let ip = machine
        .ip_address
        .with_context(|| format!("machine '{}' has no IP address assigned", machine.config.name))?;

    let user = user_opt.unwrap_or("root");
    let ip_str = ip.to_string();

    let mut ssh_args = crate::ssh::norb_ssh_args();
    ssh_args.push(format!("{user}@{ip_str}"));
    ssh_args.extend(effective_cmd.iter().cloned());

    let status = Command::new("ssh").args(&ssh_args).status()?;
    if !status.success() {
        if let Some(code) = status.code() {
            std::process::exit(code);
        }
    }

    Ok(())
}
