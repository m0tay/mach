//! `norb shell` command implementation.

use anyhow::{bail, Context, Result};
use norb_common::config::NorbConfig;
use norb_common::machine::MachineState;
use std::os::unix::process::ExitStatusExt;

use super::resolve_target_machine;

pub async fn run(name: Option<&str>, command: &[String]) -> Result<()> {
    let cfg = NorbConfig::load()?;

    let machine = resolve_target_machine(&cfg, name)?;

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
