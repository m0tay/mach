//! `mach init` command (automated safe environment setup & initialization).

use anyhow::{Context, Result};
use mach_common::config::MachConfig;
use std::fs;
use std::process::Command;

pub async fn run(non_interactive: bool) -> Result<()> {
    if !non_interactive {
        println!("\x1b[1;36m➜\x1b[0m Initializing mach environment...");
    } else {
        println!("==> Initializing mach environment...");
    }

    let home = dirs::home_dir().context("could not determine user home directory")?;
    let mach_dir = home.join(".mach");

    // 1. Ensure directory hierarchy
    let dirs_to_create = [
        mach_dir.join("machines"),
        mach_dir.join("run"),
        mach_dir.join("logs"),
        mach_dir.join("ssh"),
        mach_dir.join("cache"),
        mach_dir.join("tls"),
        home.join("Mach"),
    ];

    for dir in &dirs_to_create {
        fs::create_dir_all(dir)
            .with_context(|| format!("failed to create directory {}", dir.display()))?;
    }
    println!("  \x1b[1;32m✓\x1b[0m Prepared ~/.mach directories (machines, run, logs, ssh, cache, tls)");

    // 2. Safe Mach configuration (load or create with defaults, NEVER overwrite existing user settings)
    let config_path = mach_dir.join("config.yaml");
    let legacy_config = mach_dir.join("config.json");
    let mut cfg = MachConfig::load().context("failed to load existing Mach configuration")?;

    if !config_path.exists() && !legacy_config.exists() {
        cfg.default_distro = "debian".to_string();
        cfg.default_cpus = 2;
        cfg.default_memory_mib = 2048;
        cfg.default_disk_gib = 20;
        cfg.rosetta = true;
        cfg.save()?;
        println!("  \x1b[1;32m✓\x1b[0m Initialized ~/.mach/config.yaml with optimal defaults (Debian, 2 vCPUs, 2048 MiB RAM, 20 GiB disk, Rosetta 2 enabled)");
    } else {
        let active_cfg = if config_path.exists() {
            config_path.display().to_string()
        } else {
            legacy_config.display().to_string()
        };
        println!("  \x1b[1;32m✓\x1b[0m Preserved existing mach configuration ({})", active_cfg);
    }

    // 3. Dedicated Mach SSH key pair
    let ssh_key_path = crate::ssh::ensure_mach_ssh_key()
        .context("failed to generate or verify mach dedicated SSH key")?;
    println!("  \x1b[1;32m✓\x1b[0m Dedicated SSH keys ready ({})", ssh_key_path.display());

    // 4. Safe ~/.ssh/config integration (NEVER OVERWRITE USER CONFIGS)
    let ssh_dir = home.join(".ssh");
    let user_ssh_config = ssh_dir.join("config");
    if user_ssh_config.exists() {
        // If file exists and doesn't contain Mach block, make a safety backup first
        if let Ok(content) = fs::read_to_string(&user_ssh_config) {
            if !content.contains("BEGIN MACH MANAGED BLOCK") {
                let backup_path = ssh_dir.join("config.mach-bak");
                fs::copy(&user_ssh_config, &backup_path).context("failed to back up user SSH configuration")?;
                println!("  \x1b[1;32m✓\x1b[0m Created safety backup of user SSH config at {}", backup_path.display());
            }
        }
    }
    crate::ssh::sync_ssh_config_to_dir(&ssh_dir)
        .context("failed to safely synchronize Mach block to ~/.ssh/config")?;
    println!("  \x1b[1;32m✓\x1b[0m Safely integrated with ~/.ssh/config (user hosts and keys preserved)");

    // 5. Docker CLI Context (if Docker CLI exists)
    let docker_bin = super::find_binary("docker");
    if let Some(ref bin) = docker_bin {
        let sock_path = mach_dir.join("run").join("docker.sock");
        let exists = Command::new(bin).args(["context", "inspect", "mach"]).output()
            .context("failed to inspect Docker context")?.status.success();
        let action = if exists { "update" } else { "create" };
        let output = Command::new(bin)
            .args(["context", action, "mach", "--docker", &format!("host=unix://{}", sock_path.display()), "--description", "mach Docker Engine (Apple Silicon)"])
            .output().context("failed to configure Docker context")?;
        anyhow::ensure!(output.status.success(), "failed to configure Docker context: {}", String::from_utf8_lossy(&output.stderr));
        println!("  \x1b[1;32m✓\x1b[0m Docker CLI context 'mach' configured (unix://{})", sock_path.display());
    } else {
        println!("  - Docker CLI not detected on macOS (run 'machctl docker start' when installed)");
    }

    // Optional integrations report their own failures without claiming they succeeded.
    match crate::tls::ensure_tls_cert(&mach_dir) {
        Ok(_) => println!("  ✓ Local wildcard TLS certificates ready (*.mach.local)"),
        Err(error) => eprintln!("  Warning: local TLS certificates unavailable: {error:#}"),
    }
    match crate::nfs::ensure_finder_hierarchy() {
        Ok(_) => println!("  ✓ macOS Finder integration prepared at ~/Mach"),
        Err(error) => eprintln!("  Warning: Finder integration unavailable: {error:#}"),
    }
    crate::commands::menu_cmd::ensure_menu_bar_running();
    println!("  Menu bar launch requested");
    crate::commands::doctor::run().await.context("initialization health check failed")?;
    println!("\n✓ Mach configuration initialized. Review health-check results above before use.");

    Ok(())
}
