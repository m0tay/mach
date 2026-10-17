//! `norb delete` / `norb rm` command implementation.

use std::io::{self, IsTerminal, Write};
use anyhow::{bail, Context, Result};
use mach_common::config::NorbConfig;
use std::fs;

use super::find_machine;

pub async fn run(name: &str, yes: bool, system: bool) -> Result<()> {
    let cfg = NorbConfig::load()?;

    let machine = find_machine(&cfg.machines_dir, name)?
        .with_context(|| format!("machine '{name}' not found"))?;

    if machine.is_system() && !system {
        let svc = machine.system_service_name().unwrap_or("system");
        bail!(
            "'{}' is a protected system infrastructure machine powering {svc}.\n\
            Deleting it directly would destroy your {svc} environment.\n\n\
            To tear down {svc}, run:\n  machctl {} destroy\n\
            Or pass `--system`:\n  machctl delete {} --system -y",
            machine.config.name,
            svc.to_lowercase(),
            machine.config.name
        );
    }

    if !yes {
        if io::stdin().is_terminal() {
            print!("Delete machine '{}' and all its data? [y/N]: ", machine.config.name);
            io::stdout().flush()?;
            let mut line = String::new();
            io::stdin().read_line(&mut line)?;
            let trimmed = line.trim();
            if !trimmed.eq_ignore_ascii_case("y") && !trimmed.eq_ignore_ascii_case("yes") {
                println!("Aborted.");
                return Ok(());
            }
        } else {
            bail!("cannot delete machine in non-interactive environment without --yes / -y flag");
        }
    }

    // Ensure any running VMM process is terminated
    let _ = super::stop::run(Some(&machine.config.name), true, true).await;

    let machine_dir = cfg.machines_dir.join(&machine.config.name);
    if machine_dir.exists() {
        fs::remove_dir_all(&machine_dir)
            .with_context(|| format!("failed to remove machine directory: {}", machine_dir.display()))?;
    }

    let mut cfg = cfg;
    if cfg.default_machine.as_deref() == Some(&machine.config.name) {
        cfg.default_machine = None;
        let _ = cfg.save();
    }

    println!(
        "\x1b[1;32m✓\x1b[0m Machine \x1b[1m{}\x1b[0m deleted successfully.",
        machine.config.name
    );

    Ok(())
}
