//! `norb ssh` command implementation.

use anyhow::{bail, Context, Result};
use norb_common::config::NorbConfig;
use norb_common::machine::MachineState;
use std::process::Command;

use super::{find_machine, load_all_machines};

pub async fn run(name: Option<&str>, extra_args: &[String], details_only: bool) -> Result<()> {
    let cfg = NorbConfig::load()?;
    let target_name = match name {
        Some(n) => n.to_string(),
        None => {
            if let Some(ref def) = cfg.default_machine {
                def.clone()
            } else {
                let machines = load_all_machines(&cfg.machines_dir)?;
                if machines.is_empty() {
                    bail!("no machines found. Run `norb create <distro>` first.");
                }
                machines[0].config.name.clone()
            }
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
        .with_context(|| format!("machine '{}' has no IP address assigned", machine.config.name))?;

    let ip_str = ip.to_string();

    if details_only {
        println!("\x1b[1mSSH Details for {}\x1b[0m:", machine.config.name);
        println!("  Host:        {}", ip_str);
        println!("  Hostname:    {}.norb.local", machine.config.name);
        println!("  User:        root");
        println!("  Port:        22");
        println!("  SSH Command: \x1b[1;32mssh root@{}\x1b[0m", ip_str);
        return Ok(());
    }

    let mut ssh_args = vec![
        "-o".to_string(),
        "StrictHostKeyChecking=no".to_string(),
        "-o".to_string(),
        "UserKnownHostsFile=/dev/null".to_string(),
        "-o".to_string(),
        "LogLevel=ERROR".to_string(),
        format!("root@{ip_str}"),
    ];
    ssh_args.extend(extra_args.iter().cloned());

    let status = Command::new("ssh").args(&ssh_args).status()?;
    if !status.success() {
        if let Some(code) = status.code() {
            std::process::exit(code);
        }
    }

    Ok(())
}
