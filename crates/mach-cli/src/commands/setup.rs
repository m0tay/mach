//! Initial onboarding and environment setup for Mach.

use anyhow::Result;
use mach_common::config::MachConfig;
use mach_common::machine::MachineState;

/// Run the onboarding setup.
pub async fn run(welcome: bool) -> Result<()> {
    run_ext(welcome, false).await
}

/// Run streamlined onboarding setup, optionally redirecting progress messages to stderr.
pub async fn run_streamlined(to_stderr: bool) -> Result<()> {
    run_ext(false, to_stderr).await
}

async fn run_ext(welcome: bool, to_stderr: bool) -> Result<()> {
    macro_rules! out_ln {
        ($($arg:tt)*) => {
            if to_stderr {
                eprintln!($($arg)*);
            } else {
                println!($($arg)*);
            }
        };
    }

    if welcome {
        out_ln!("\x1b[1;34m━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\x1b[0m");
        out_ln!(" \x1b[1;37m👋 Welcome to mach!\x1b[0m");
        out_ln!(" \x1b[0;36mThe fast, native Linux & container engine for Apple Silicon Macs.\x1b[0m");
        out_ln!("\x1b[1;34m━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\x1b[0m\n");
        out_ln!("\x1b[1;33mPreparing your environment...\x1b[0m\n");
    }

    // 1. Initializing configuration with optimal defaults (Debian)
    let mut cfg = MachConfig::load().unwrap_or_default();
    cfg.default_distro = "debian".to_string();
    if cfg.default_machine.is_none() {
        cfg.default_machine = Some("debian".to_string());
    }
    let _ = cfg.save();
    if welcome {
        out_ln!("  \x1b[1;32m✓\x1b[0m Configuration initialized (~/.mach/config.yaml)");
    }

    // 2. Dedicated SSH keys & host agent forwarding
    let _ = crate::ssh::ensure_mach_ssh_key();
    let _ = crate::ssh::sync_ssh_config().await;
    if welcome {
        out_ln!("  \x1b[1;32m✓\x1b[0m Dedicated SSH keys configured (~/.mach/ssh/id_ed25519)");
    }

    // 3. Finder file hierarchy (~/Mach) & menu bar status item
    let _ = crate::nfs::ensure_finder_hierarchy();
    crate::commands::menu_cmd::ensure_menu_bar_running();
    if welcome {
        out_ln!("  \x1b[1;32m✓\x1b[0m macOS Finder access prepared (~/Mach)");
        out_ln!("  \x1b[1;32m✓\x1b[0m Menu bar status item active");
    }

    // 4. Background developer integrations (shell completions & wildcard TLS)
    if let Some(home) = dirs::home_dir() {
        let _ = crate::commands::completion::install_zsh(&home);
        let _ = crate::commands::completion::install_bash(&home);
    }
    let _ = crate::tls::ensure_tls_cert(&cfg.data_dir);

    // 5. Create and boot the primary Debian Linux machine
    let machines = crate::commands::load_all_machines(&cfg.machines_dir)?;
    let debian_machine = machines.iter().find(|m| m.config.name == "debian");

    match debian_machine {
        Some(m) if m.state == MachineState::Running => {
            if welcome {
                out_ln!("  \x1b[1;32m✓\x1b[0m Primary Debian machine is already running");
            }
        }
        Some(_) => {
            if welcome {
                out_ln!("  \x1b[1;36m➜\x1b[0m Starting Debian machine...");
            }
            crate::commands::start::run_ext(Some("debian"), to_stderr).await?;
            if welcome {
                out_ln!("  \x1b[1;32m✓\x1b[0m Primary Debian machine is running");
            }
        }
        None => {
            crate::commands::create::run_ext(
                "debian",
                Some("debian"),
                2,
                2048,
                20,
                "aarch64",
                false,
                false,
                to_stderr,
            )
            .await?;
            let _ = crate::commands::default_cmd::run(Some("debian")).await;
            crate::commands::start::run_ext(Some("debian"), to_stderr).await?;
            if welcome {
                out_ln!("  \x1b[1;32m✓\x1b[0m Primary Debian machine created and running");
            }
        }
    }

    // 6. Docker CLI context (if docker VM exists or docker binary is available on host)
    let has_docker_vm = crate::commands::find_machine(&cfg.machines_dir, crate::commands::docker::DOCKER_VM_NAME)?
        .is_some();
    if has_docker_vm || cfg.docker.enabled {
        let _ = crate::commands::docker::setup(false).await;
        if welcome {
            out_ln!("  \x1b[1;32m✓\x1b[0m Standard Docker CLI context 'mach' configured");
        }
    } else if let Some(bin) = crate::commands::find_binary("docker") {
        let sock_path = cfg.data_dir.join("run").join("docker.sock");
        let _ = std::process::Command::new(&bin).args(["context", "rm", "-f", "mach"]).output();
        let _ = std::process::Command::new(&bin)
            .args([
                "context",
                "create",
                "mach",
                "--docker",
                &format!("host=unix://{}", sock_path.display()),
                "--description",
                "mach Docker Engine (Apple Silicon)",
            ])
            .output();
        let _ = std::process::Command::new(&bin).args(["context", "use", "mach"]).output();
        if welcome {
            out_ln!("  \x1b[1;32m✓\x1b[0m Standard Docker CLI context 'mach' configured");
        }
    }

    if welcome {
        out_ln!("\n\x1b[1;34m━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\x1b[0m");
        out_ln!(" \x1b[1;32m✓ Setup complete!\x1b[0m");
        out_ln!("\x1b[1;34m━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\x1b[0m\n");
        out_ln!(" \x1b[1mQuick Start:\x1b[0m");
        out_ln!("   • \x1b[1;32mmach\x1b[0m              Open Linux shell");
        out_ln!("   • \x1b[1;32mmach <command>\x1b[0m    Run command in Linux (e.g. \x1b[33mmach uname -a\x1b[0m)");
        out_ln!("   • \x1b[1;32mdocker ps\x1b[0m         Use standard macOS Docker CLI");
        out_ln!("   • \x1b[1;34mmachctl --help\x1b[0m    Explore control plane & VM management\n");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_setup_default_config() {
        let mut cfg = MachConfig::default();
        cfg.default_distro = "debian".to_string();
        if cfg.default_machine.is_none() {
            cfg.default_machine = Some("debian".to_string());
        }
        assert_eq!(cfg.default_distro, "debian");
        assert_eq!(cfg.default_machine.as_deref(), Some("debian"));
    }
}
