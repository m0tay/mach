//! Clone an existing Norb machine using APFS Copy-on-Write.

use anyhow::{bail, Context, Result};
use norb_common::config::NorbConfig;
use norb_common::machine::MachineState;
use std::fs;
use uuid::Uuid;

use crate::commands::{find_machine, save_machine};

pub async fn run(source_name: &str, target_name: &str) -> Result<()> {
    let cfg = NorbConfig::load()?;
    let source = match find_machine(&cfg.machines_dir, source_name)? {
        Some(m) => m,
        None => bail!("Source machine '{}' does not exist.", source_name),
    };

    if source.state == MachineState::Running {
        bail!("Source machine '{}' is currently running. Stop it before cloning.", source_name);
    }

    if find_machine(&cfg.machines_dir, target_name)?.is_some() {
        bail!("A machine named '{}' already exists.", target_name);
    }

    println!("  \x1b[1;36m➜\x1b[0m Cloning \x1b[1m{}\x1b[0m to \x1b[1m{}\x1b[0m via APFS Copy-on-Write...", source_name, target_name);

    let src_dir = cfg.machines_dir.join(source_name);
    let target_dir = cfg.machines_dir.join(target_name);
    fs::create_dir_all(&target_dir)?;

    let src_disk = src_dir.join("disk.raw");
    let target_disk = target_dir.join("disk.raw");

    if src_disk.exists() {
        norb_common::fs::apfs_clone(&src_disk, &target_disk)
            .with_context(|| format!("failed to clone disk from {} to {}", src_disk.display(), target_disk.display()))?;
    }

    // Build new machine descriptor
    let mut cloned = source.clone();
    cloned.id = Uuid::new_v4();
    cloned.config.name = target_name.to_string();
    cloned.state = MachineState::Stopped;
    cloned.ip_address = None;
    cloned.pid = None;
    cloned.created_at = chrono::Utc::now();

    // Regenerate cloud-init seed disk with new machine hostname if applicable
    let src_cidata = src_dir.join("cidata.iso");
    if src_cidata.exists() {
        let _ = crate::cloud_init::generate_cidata_iso(&cloned.id, target_name, &target_dir.join("cidata.iso"));
    }

    save_machine(&cfg.machines_dir, &cloned)?;

    println!(
        "\x1b[1;32m✓\x1b[0m Successfully cloned \x1b[1m{}\x1b[0m to \x1b[1m{}\x1b[0m.",
        source_name, target_name
    );
    println!("Run \x1b[1;34mnorb start {}\x1b[0m to launch the new clone.", target_name);

    Ok(())
}
