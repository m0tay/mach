//! Rename an existing Norb machine.

use anyhow::{bail, Result};
use norb_common::config::NorbConfig;
use norb_common::machine::MachineState;
use std::fs;

use crate::commands::{find_machine, save_machine};

pub async fn run(old_name: &str, new_name: &str) -> Result<()> {
    let mut cfg = NorbConfig::load()?;
    let mut machine = match find_machine(&cfg.machines_dir, old_name)? {
        Some(m) => m,
        None => bail!("Machine '{}' does not exist.", old_name),
    };

    if machine.state == MachineState::Running {
        bail!("Machine '{}' is currently running. Stop it before renaming.", old_name);
    }

    if find_machine(&cfg.machines_dir, new_name)?.is_some() {
        bail!("A machine named '{}' already exists.", new_name);
    }

    let old_dir = cfg.machines_dir.join(old_name);
    let new_dir = cfg.machines_dir.join(new_name);

    fs::rename(&old_dir, &new_dir)?;

    machine.config.name = new_name.to_string();
    save_machine(&cfg.machines_dir, &machine)?;

    let cidata_path = new_dir.join("cidata.iso");
    if cidata_path.exists() {
        crate::cloud_init::generate_cidata_iso(&machine.id, new_name, &cidata_path)?;
    }

    if cfg.default_machine.as_deref() == Some(old_name) {
        cfg.default_machine = Some(new_name.to_string());
        cfg.save()?;
    }

    println!("\x1b[1;32m✓\x1b[0m Renamed machine \x1b[1m{}\x1b[0m to \x1b[1m{}\x1b[0m.", old_name, new_name);
    Ok(())
}
