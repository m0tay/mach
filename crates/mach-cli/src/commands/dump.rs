//! `machctl dump` and `mach dump` command implementation.
//!
//! Dumps raw machine or system configuration objects for scripting, backups, and pipelines.
//! Output is formatted as YAML by default, with `--json` support.

use anyhow::{bail, Context, Result};
use mach_common::config::MachConfig;
use mach_common::machine::Machine;
use serde::Serialize;

use super::{find_machine, load_all_machines};

#[derive(Serialize)]
pub struct SystemDump {
    pub config: MachConfig,
    pub machines: Vec<Machine>,
}

pub async fn run(name: Option<&str>, all: bool, _yaml: bool, json: bool) -> Result<()> {
    let cfg = MachConfig::load()?;

    if all {
        let machines = load_all_machines(&cfg.machines_dir)?;
        let dump = SystemDump {
            config: cfg,
            machines,
        };
        if json {
            println!("{}", serde_json::to_string_pretty(&dump)?);
        } else {
            print!("{}", serde_yaml::to_string(&dump)?);
        }
        return Ok(());
    }

    let target_name = match name {
        Some(n) => n.to_string(),
        None => {
            if let Some(ref def) = cfg.default_machine {
                def.clone()
            } else {
                let machines = load_all_machines(&cfg.machines_dir)?;
                if machines.is_empty() {
                    bail!("no machines found. Run `mach create <distro>` first.");
                }
                machines[0].config.name.clone()
            }
        }
    };

    let machine = find_machine(&cfg.machines_dir, &target_name)?
        .with_context(|| format!("machine '{target_name}' not found"))?;

    if json {
        println!("{}", serde_json::to_string_pretty(&machine)?);
    } else {
        print!("{}", serde_yaml::to_string(&machine)?);
    }

    Ok(())
}
