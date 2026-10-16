//! `norb shell` command implementation.

use anyhow::{bail, Context, Result};
use norb_common::config::NorbConfig;
use norb_common::machine::MachineState;
use std::process::Command;

use super::{find_machine, load_all_machines};

pub async fn run(name: Option<&str>, command: &[String]) -> Result<()> {
    let cfg = NorbConfig::load()?;

    let target_name = match name {
        Some(n) => n.to_string(),
        None => {
            let machines = load_all_machines(&cfg.machines_dir)?;
            let running: Vec<_> = machines
                .into_iter()
                .filter(|m| m.state == MachineState::Running)
                .collect();
            if running.is_empty() {
                bail!("no running machines found. Run `norb start <machine>` first.");
            }
            running[0].config.name.clone()
        }
    };

    let machine = find_machine(&cfg.machines_dir, &target_name)?
        .with_context(|| format!("machine '{target_name}' not found"))?;

    if machine.state != MachineState::Running {
        bail!(
            "machine '{}' is not running (state: {}). Run `norb start {}` first.",
            machine.config.name,
            machine.state,
            machine.config.name
        );
    }

    let ip = machine
        .ip_address
        .with_context(|| format!("machine '{}' does not have an IP address", machine.config.name))?;

    let ip_str = ip.to_string();

    let mut ssh_args = crate::ssh::norb_ssh_args();
    ssh_args.extend([
        "-o".to_string(),
        "ConnectTimeout=4".to_string(),
        format!("root@{ip_str}"),
    ]);

    if !command.is_empty() {
        ssh_args.extend(command.iter().cloned());
        let status = Command::new("ssh").args(&ssh_args).status()?;
        if !status.success() {
            if let Some(code) = status.code() {
                std::process::exit(code);
            }
        }
    } else {
        println!("\x1b[1;36m➜\x1b[0m Connecting to \x1b[1m{}\x1b[0m ({})...", machine.config.name, ip_str);
        let status = Command::new("ssh").args(&ssh_args).status()?;
        if !status.success() {
            if let Some(code) = status.code() {
                std::process::exit(code);
            }
        }
    }

    Ok(())
}
