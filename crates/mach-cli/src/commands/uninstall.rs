//! `machctl uninstall` command implementation.
//!
//! Provides a complete, clean uninstallation of Mach, stopping all virtual machines,
//! terminating background processes and daemons, removing Docker and Kubernetes contexts,
//! restoring ~/.ssh/config and Keychain trust stores, and optionally wiping all data.

use anyhow::Result;
use mach_common::config::MachConfig;
use std::io::{self, Write};
use std::path::Path;
use std::process::Command;

pub async fn run(all: bool, force: bool, keep_data: bool) -> Result<()> {
    println!("\x1b[1;36m➜\x1b[0m Preparing to uninstall Mach...");

    if !force {
        if all {
            print!("\x1b[1;33m⚠ WARNING:\x1b[0m This will stop all Mach VMs and \x1b[1mPERMANENTLY DELETE\x1b[0m all machine disks in ~/.mach.\nAre you sure you want to proceed? [y/N]: ");
        } else if keep_data {
            print!("This will stop all running Mach processes and remove host integrations (Docker, Kubernetes, SSH).\nAre you sure you want to proceed? [y/N]: ");
        } else {
            print!("This will stop all running Mach VMs, remove host integrations, and clean up ~/.mach.\nAre you sure you want to proceed? [y/N]: ");
        }
        let _ = io::stdout().flush();
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let choice = input.trim().to_lowercase();
        if choice != "y" && choice != "yes" {
            println!("Uninstallation cancelled.");
            return Ok(());
        }
    }

    let cfg_opt = MachConfig::load().ok();

    // 1. Stop all virtual machines cleanly
    println!("  \x1b[1;36m➜\x1b[0m Stopping all running virtual machines...");
    if let Some(ref cfg) = cfg_opt {
        if let Ok(machines) = super::load_all_machines(&cfg.machines_dir) {
            for mut m in machines {
                let _ = super::stop::stop_single_machine(&mut m, cfg, true);
            }
        }
    }
    // Force kill any remaining VM hypervisor child processes
    super::kill_running_vm_processes(None);
    println!("  \x1b[1;32m✓\x1b[0m All virtual machine processes stopped");

    // 2. Stop background daemon and menu bar status item
    println!("  \x1b[1;36m➜\x1b[0m Stopping background daemon and menu bar status item...");
    let _ = super::daemon::stop_daemon();
    super::menu_cmd::stop_menu_bar();
    println!("  \x1b[1;32m✓\x1b[0m Background daemons terminated");

    // 3. Terminate background tunnels
    super::docker::terminate_docker_ssh_tunnel();
    super::k8s::terminate_k8s_ssh_tunnel();

    // 4. Clean up Docker CLI integration
    println!("  \x1b[1;36m➜\x1b[0m Cleaning up Docker CLI context and sockets...");
    let _ = Command::new("docker").args(["context", "use", "default"]).output();
    let _ = Command::new("docker").args(["context", "rm", "-f", "mach"]).output();

    let sys_sock = Path::new("/var/run/docker.sock");
    if sys_sock.is_symlink() {
        if let Ok(target) = std::fs::read_link(sys_sock) {
            if target.to_string_lossy().contains(".mach") {
                let _ = std::fs::remove_file(sys_sock);
                println!("  \x1b[1;32m✓\x1b[0m Removed system /var/run/docker.sock symlink");
            }
        }
    }
    println!("  \x1b[1;32m✓\x1b[0m Docker CLI context reset to default");

    // 5. Clean up Kubernetes CLI integration
    println!("  \x1b[1;36m➜\x1b[0m Cleaning up Kubernetes configuration...");
    let _ = Command::new("kubectl").args(["config", "delete-context", "mach"]).output();
    let _ = Command::new("kubectl").args(["config", "delete-cluster", "mach"]).output();
    let _ = Command::new("kubectl").args(["config", "delete-user", "mach"]).output();
    if let Some(home) = dirs::home_dir() {
        let _ = std::fs::remove_file(home.join(".kube/config.mach"));
    }
    println!("  \x1b[1;32m✓\x1b[0m Kubernetes context cleaned");

    // 6. Clean up ~/.ssh/config managed block
    println!("  \x1b[1;36m➜\x1b[0m Restoring ~/.ssh/config...");
    if let Some(home) = dirs::home_dir() {
        let ssh_dir = home.join(".ssh");
        let _ = crate::ssh::remove_mach_ssh_config_from_dir(&ssh_dir);
    }
    println!("  \x1b[1;32m✓\x1b[0m Removed Mach block from ~/.ssh/config (user hosts preserved)");

    // 7. Uninstall TLS Root CA from macOS Keychain
    println!("  \x1b[1;36m➜\x1b[0m Removing TLS Root CA from Keychain...");
    let _ = super::tls_cmd::uninstall().await;

    // 8. Remove local unicast resolver if present
    let resolver = Path::new("/etc/resolver/mach.local");
    if resolver.exists() {
        if std::fs::remove_file(resolver).is_ok() {
            println!("  \x1b[1;32m✓\x1b[0m Removed /etc/resolver/mach.local");
        } else {
            println!("  \x1b[1;33mTip:\x1b[0m To remove macOS DNS resolver: sudo rm /etc/resolver/mach.local");
        }
    }

    // 9. Wipe data directories if requested or default full uninstall
    if all || !keep_data {
        println!("  \x1b[1;36m➜\x1b[0m Removing ~/.mach data and ~/Mach Finder mount...");
        if let Some(ref cfg) = cfg_opt {
            let _ = std::fs::remove_dir_all(&cfg.data_dir);
        }
        if let Some(home) = dirs::home_dir() {
            let mach_dir = home.join(".mach");
            let _ = std::fs::remove_dir_all(&mach_dir);
            let finder_dir = home.join("Mach");
            let _ = std::fs::remove_dir_all(&finder_dir);
        }
        println!("  \x1b[1;32m✓\x1b[0m Removed ~/.mach and ~/Mach");
    } else {
        println!("  \x1b[1;34mℹ\x1b[0m Preserved machine disk images in ~/.mach (pass --all to remove)");
    }

    // 10. Clean cached Homebrew download archives
    if let Some(home) = dirs::home_dir() {
        let cache_dir = home.join("Library/Caches/Homebrew/downloads");
        if let Ok(entries) = std::fs::read_dir(&cache_dir) {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().contains("mach") {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }

    println!("\n\x1b[1;32m✓\x1b[0m Mach has been cleanly uninstalled from your system!");
    println!("\nTo complete binary package removal via Homebrew, run:");
    println!("  \x1b[1;36mbrew uninstall mach\x1b[0m\n");

    Ok(())
}
