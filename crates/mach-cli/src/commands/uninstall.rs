//! `machctl uninstall` command implementation.
//!
//! Provides a complete, clean uninstallation of Mach, stopping all virtual machines,
//! terminating background processes and daemons, removing Docker and Kubernetes contexts,
//! restoring ~/.ssh/config and Keychain trust stores, and optionally wiping all data.

use anyhow::Result;
use mach_common::config::MachConfig;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

fn find_binary_executable(name: &str) -> Option<PathBuf> {
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let p = dir.join(name);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    let mut candidates = vec![
        PathBuf::from(format!("/opt/homebrew/bin/{name}")),
        PathBuf::from(format!("/usr/local/bin/{name}")),
        PathBuf::from(format!("/usr/bin/{name}")),
    ];
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(format!(".rd/bin/{name}")));
        candidates.push(home.join(format!(".orbstack/bin/{name}")));
        candidates.push(home.join(format!(".colima/bin/{name}")));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Surgically clean Docker CLI context integration from the user's home directory.
///
/// 1. Resets `currentContext` in `~/.docker/config.json` to "default" if it was "mach" (or "norb").
///    Never touches user auth tokens, credsStore, credHelpers, plugins, or other contexts!
/// 2. Removes `~/.docker/contexts/meta/*/meta.json` (and its associated tls dir) where name is "mach" or "norb".
/// 3. Removes `/var/run/docker.sock` if it is a symlink pointing into `~/.mach` or `~/.norb`.
pub fn clean_docker_cli_integration(home: &Path) {
    // 1. Reset currentContext in ~/.docker/config.json
    let docker_cfg = home.join(".docker/config.json");
    if docker_cfg.exists() {
        if let Ok(content) = std::fs::read_to_string(&docker_cfg) {
            if let Ok(mut val) = serde_json::from_str::<serde_json::Value>(&content) {
                let mut modified = false;
                if let Some(ctx) = val.get("currentContext").and_then(|c| c.as_str()) {
                    if ctx == "mach" || ctx == "norb" {
                        if let Some(obj) = val.as_object_mut() {
                            obj.insert(
                                "currentContext".to_string(),
                                serde_json::Value::String("default".to_string()),
                            );
                            modified = true;
                        }
                    }
                }
                if modified {
                    if let Ok(serialized) = serde_json::to_string_pretty(&val) {
                        let _ = std::fs::write(&docker_cfg, serialized);
                    }
                }
            }
        }
    }

    // 2. Remove ~/.docker/contexts/meta/*/meta.json where name is "mach" or "norb"
    let meta_dir = home.join(".docker/contexts/meta");
    if meta_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&meta_dir) {
            for entry in entries.flatten() {
                let meta_file = entry.path().join("meta.json");
                if meta_file.exists() {
                    if let Ok(txt) = std::fs::read_to_string(&meta_file) {
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&txt) {
                            let name = val.get("Name").and_then(|n| n.as_str());
                            if name == Some("mach") || name == Some("norb") {
                                let hash_name = entry.file_name();
                                let _ = std::fs::remove_dir_all(entry.path());
                                let tls_dir = home.join(".docker/contexts/tls").join(&hash_name);
                                if tls_dir.exists() {
                                    let _ = std::fs::remove_dir_all(&tls_dir);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // 3. System /var/run/docker.sock symlink check
    let sys_sock = Path::new("/var/run/docker.sock");
    if sys_sock.is_symlink() {
        if let Ok(target) = std::fs::read_link(sys_sock) {
            let target_str = target.to_string_lossy();
            if target_str.contains(".mach") || target_str.contains(".norb") {
                let _ = std::fs::remove_file(sys_sock);
                println!("  \x1b[1;32m✓\x1b[0m Removed system /var/run/docker.sock symlink");
            }
        }
    }
}

/// Clean temporary files and directories created by Mach in `/tmp` and `std::env::temp_dir()`.
pub fn clean_temp_files() {
    let dirs_to_check = [PathBuf::from("/tmp"), std::env::temp_dir()];
    let mut checked = std::collections::HashSet::new();

    for temp_dir in &dirs_to_check {
        let key = temp_dir.canonicalize().unwrap_or_else(|_| temp_dir.clone());
        if !checked.insert(key) {
            continue;
        }

        if let Ok(entries) = std::fs::read_dir(temp_dir) {
            for entry in entries.flatten() {
                let file_name = entry.file_name();
                let name_str = file_name.to_string_lossy();
                if name_str.starts_with("mach-")
                    || name_str.starts_with("mach_")
                    || name_str.starts_with("norb-")
                    || name_str.starts_with("norb_")
                    || name_str.starts_with("cidata_")
                    || name_str.starts_with("cidata-")
                {
                    let p = entry.path();
                    if p.is_dir() {
                        let _ = std::fs::remove_dir_all(&p);
                    } else {
                        let _ = std::fs::remove_file(&p);
                    }
                }
            }
        }
    }
}

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
    let _ = Command::new("pkill").args(["-9", "-f", "mach menu"]).output();
    let _ = Command::new("pkill").args(["-9", "-f", "machctl menu"]).output();
    let _ = Command::new("pkill").args(["-f", "dns-sd.*mach.local"]).output();
    println!("  \x1b[1;32m✓\x1b[0m Background daemons terminated");

    // 3. Terminate background tunnels
    super::docker::terminate_docker_ssh_tunnel();
    super::k8s::terminate_k8s_ssh_tunnel();

    // 4. Clean up Docker CLI integration
    println!("  \x1b[1;36m➜\x1b[0m Cleaning up Docker CLI context and sockets...");
    if let Some(bin) = find_binary_executable("docker") {
        let _ = Command::new(&bin).args(["context", "use", "default"]).output();
        let _ = Command::new(&bin).args(["context", "rm", "-f", "mach"]).output();
        let _ = Command::new(&bin).args(["context", "rm", "-f", "norb"]).output();
    }
    if let Some(home) = dirs::home_dir() {
        clean_docker_cli_integration(&home);
    }
    println!("  \x1b[1;32m✓\x1b[0m Docker CLI context reset to default");

    // 5. Clean up Kubernetes CLI integration (~/.kube/config)
    println!("  \x1b[1;36m➜\x1b[0m Cleaning up Kubernetes configuration...");
    if let Some(home) = dirs::home_dir() {
        let kube_dir = home.join(".kube");
        if let Err(e) = super::k8s::clean_kubeconfig(&kube_dir) {
            tracing::warn!("Failed to cleanly remove Mach from kubeconfig: {e}");
        }
        if kube_dir.join("config").exists() {
            if let Some(bin) = find_binary_executable("kubectl") {
                let _ = Command::new(&bin).args(["config", "delete-context", "mach"]).output();
                let _ = Command::new(&bin).args(["config", "delete-cluster", "mach"]).output();
                let _ = Command::new(&bin).args(["config", "delete-user", "mach"]).output();
            }
        }
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
    let resolvers = [
        Path::new("/etc/resolver/mach.local"),
        Path::new("/etc/resolver/norb.local"),
    ];
    for resolver in &resolvers {
        if resolver.exists() {
            if std::fs::remove_file(resolver).is_ok() {
                println!("  \x1b[1;32m✓\x1b[0m Removed {}", resolver.display());
            } else {
                let status = Command::new("sudo")
                    .args(["rm", "-f", resolver.to_str().unwrap()])
                    .status();
                if status.map(|s| s.success()).unwrap_or(false) {
                    println!("  \x1b[1;32m✓\x1b[0m Removed {}", resolver.display());
                } else {
                    println!("  \x1b[1;33mTip:\x1b[0m To remove macOS DNS resolver: sudo rm {}", resolver.display());
                }
            }
        }
    }

    // 9. Unmount and remove ~/Mach and ~/Norb Finder mounts
    if let Some(home) = dirs::home_dir() {
        for name in &["Mach", "Norb"] {
            let finder_dir = home.join(name);
            if finder_dir.exists() {
                if let Ok(entries) = std::fs::read_dir(finder_dir.join("machines")) {
                    for entry in entries.flatten() {
                        let _ = crate::nfs::unmount(&entry.path(), true);
                    }
                }
                if let Ok(entries) = std::fs::read_dir(finder_dir.join("volumes")) {
                    for entry in entries.flatten() {
                        let _ = crate::nfs::unmount(&entry.path(), true);
                    }
                }
                let _ = crate::nfs::unmount(&finder_dir, true);
                let _ = std::fs::remove_dir_all(&finder_dir);
            }
        }
    }

    // 10. Clean temporary files in /tmp/mach-* and std::env::temp_dir()
    clean_temp_files();

    // 11. Purge storage and image caches
    if let Some(home) = dirs::home_dir() {
        for root in &[home.join(".mach"), home.join(".norb")] {
            let _ = std::fs::remove_dir_all(root.join("images"));
            let _ = std::fs::remove_dir_all(root.join("cache"));
        }
    }
    if let Some(ref cfg) = cfg_opt {
        let _ = std::fs::remove_dir_all(cfg.data_dir.join("images"));
        let _ = std::fs::remove_dir_all(cfg.data_dir.join("cache"));
    }

    // 12. Wipe complete data directories if requested or default full uninstall
    if all || !keep_data {
        println!("  \x1b[1;36m➜\x1b[0m Removing ~/.mach data directory...");
        if let Some(ref cfg) = cfg_opt {
            let _ = std::fs::remove_dir_all(&cfg.data_dir);
        }
        if let Some(home) = dirs::home_dir() {
            let _ = std::fs::remove_dir_all(home.join(".mach"));
            let _ = std::fs::remove_dir_all(home.join(".norb"));
        }
        println!("  \x1b[1;32m✓\x1b[0m Removed ~/.mach and ~/Mach");
    } else {
        // In keep_data mode, clear runtime state/sockets but keep ~/.mach/machines
        if let Some(ref cfg) = cfg_opt {
            let _ = std::fs::remove_dir_all(cfg.data_dir.join("run"));
            let _ = std::fs::remove_dir_all(cfg.data_dir.join("logs"));
        }
        if let Some(home) = dirs::home_dir() {
            let _ = std::fs::remove_dir_all(home.join(".mach/run"));
            let _ = std::fs::remove_dir_all(home.join(".mach/logs"));
        }
        println!("  \x1b[1;34mℹ\x1b[0m Preserved machine disk images in ~/.mach (pass --all to remove)");
    }

    println!("\n\x1b[1;32m✓\x1b[0m Mach has been cleanly uninstalled from your system!\n");

    if !force {
        println!("To complete binary package removal via Homebrew, run:");
        println!("  \x1b[1;36mbrew uninstall mach\x1b[0m\n");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_docker_cli_integration_resets_mach_and_preserves_auth() {
        let temp_dir = std::env::temp_dir().join(format!("mach-docker-clean-{}", uuid::Uuid::new_v4()));
        let docker_dir = temp_dir.join(".docker");
        let meta_dir = docker_dir.join("contexts/meta");
        let mach_meta = meta_dir.join("111111");
        let other_meta = meta_dir.join("222222");

        std::fs::create_dir_all(&mach_meta).unwrap();
        std::fs::create_dir_all(&other_meta).unwrap();

        let initial_cfg = serde_json::json!({
            "auths": {
                "https://index.docker.io/v1/": {
                    "auth": "secret-token-12345"
                }
            },
            "credsStore": "osxkeychain",
            "currentContext": "mach"
        });
        std::fs::write(docker_dir.join("config.json"), serde_json::to_string(&initial_cfg).unwrap()).unwrap();

        std::fs::write(
            mach_meta.join("meta.json"),
            serde_json::json!({"Name": "mach", "Endpoints": {}}).to_string(),
        ).unwrap();

        std::fs::write(
            other_meta.join("meta.json"),
            serde_json::json!({"Name": "desktop-linux", "Endpoints": {}}).to_string(),
        ).unwrap();

        clean_docker_cli_integration(&temp_dir);

        // Verify config.json
        let cfg_str = std::fs::read_to_string(docker_dir.join("config.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&cfg_str).unwrap();

        assert_eq!(parsed["currentContext"], "default");
        assert_eq!(parsed["credsStore"], "osxkeychain");
        assert_eq!(parsed["auths"]["https://index.docker.io/v1/"]["auth"], "secret-token-12345");

        // Verify context directories
        assert!(!mach_meta.exists(), "mach context meta should be deleted");
        assert!(other_meta.exists(), "desktop-linux context meta must be preserved");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_clean_docker_cli_integration_preserves_unrelated_context() {
        let temp_dir = std::env::temp_dir().join(format!("mach-docker-clean2-{}", uuid::Uuid::new_v4()));
        let docker_dir = temp_dir.join(".docker");
        std::fs::create_dir_all(&docker_dir).unwrap();

        let initial_cfg = serde_json::json!({
            "currentContext": "colima"
        });
        std::fs::write(docker_dir.join("config.json"), serde_json::to_string(&initial_cfg).unwrap()).unwrap();

        clean_docker_cli_integration(&temp_dir);

        let cfg_str = std::fs::read_to_string(docker_dir.join("config.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&cfg_str).unwrap();
        assert_eq!(parsed["currentContext"], "colima", "unrelated currentContext must remain untouched");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}

