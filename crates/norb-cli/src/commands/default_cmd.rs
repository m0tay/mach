//! `norb default` command implementation.

use anyhow::{bail, Result};
use norb_common::config::NorbConfig;

use super::{find_machine, load_all_machines};

pub async fn run(name: Option<&str>) -> Result<()> {
    let mut cfg = NorbConfig::load()?;

    match name {
        Some(machine_name) => {
            if find_machine(&cfg.machines_dir, machine_name)?.is_none() {
                bail!("machine '{machine_name}' not found");
            }
            cfg.default_machine = Some(machine_name.to_string());
            cfg.save()?;
            println!("\x1b[1;32m✓\x1b[0m Default machine set to \x1b[1m{}\x1b[0m.", machine_name);
        }
        None => {
            if let Some(ref def) = cfg.default_machine {
                println!("{}", def);
            } else {
                let machines = load_all_machines(&cfg.machines_dir)?;
                if let Some(first) = machines.first() {
                    println!("{} (auto-detected)", first.config.name);
                } else {
                    println!("No default machine set. Create one with `norb create <distro>`.");
                }
            }
        }
    }

    Ok(())
}
