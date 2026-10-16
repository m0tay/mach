//! `norb delete` / `norb rm` command implementation.

use anyhow::{Context, Result};
use norb_common::config::NorbConfig;
use std::fs;

use super::find_machine;

pub async fn run(name: &str, _yes: bool) -> Result<()> {
    let cfg = NorbConfig::load()?;

    let machine = find_machine(&cfg.machines_dir, name)?
        .with_context(|| format!("machine '{name}' not found"))?;

    // Ensure any running VMM process is terminated
    let _ = super::stop::run(Some(&machine.config.name), true).await;

    let machine_dir = cfg.machines_dir.join(&machine.config.name);
    if machine_dir.exists() {
        fs::remove_dir_all(&machine_dir)
            .with_context(|| format!("failed to remove machine directory: {}", machine_dir.display()))?;
    }

    println!(
        "\x1b[1;32m✓\x1b[0m Machine \x1b[1m{}\x1b[0m deleted successfully.",
        machine.config.name
    );

    Ok(())
}
