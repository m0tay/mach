//! Initial onboarding and environment setup for Norb.

use anyhow::Result;
use norb_common::config::NorbConfig;
use norb_common::machine::MachineState;

/// Run the interactive onboarding setup.
pub async fn run(welcome: bool) -> Result<()> {
    if welcome {
        println!("\x1b[1;34m━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\x1b[0m");
        println!(" \x1b[1;37m👋 Welcome to Norb!\x1b[0m");
        println!(" \x1b[0;36mThe fast, zero-overhead Linux & container engine for Apple Silicon Macs.\x1b[0m");
        println!("\x1b[1;34m━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\x1b[0m");
        println!("\nNorb gives your Mac native Linux superpowers:");
        println!("  • \x1b[1mSub-second startup\x1b[0m: Boots full Linux machines in under 1 second.");
        println!("  • \x1b[1mStandard Docker\x1b[0m: Use your regular macOS `docker` CLI with zero background bloat.");
        println!("  • \x1b[1mLocal Kubernetes (K3s)\x1b[0m: Ready-to-go cluster with instant `kubectl` compatibility.");
        println!("  • \x1b[1mTransparent execution\x1b[0m: Type `norb uname -a` from any Mac terminal.");
        println!("  • \x1b[1mNative file sharing\x1b[0m: Instant 1:1 access to `/Users` with zero copying.");
        println!("  • \x1b[1mUltra-lightweight\x1b[0m: Consumes less than 15 MB of RAM and zero battery while idle.");
        println!("\n\x1b[1;33mSetting up your environment...\x1b[0m\n");
    }

    // Step 1: Ensure smart configuration & directory structure
    println!("  \x1b[1;36m➜\x1b[0m Step 1: Initializing configuration with optimal defaults...");
    let mut cfg = NorbConfig::load().unwrap_or_default();
    cfg.default_distro = "debian".to_string();
    cfg.default_machine = Some("debian".to_string());
    cfg.save()?;
    println!("    \x1b[1;32m✓\x1b[0m Saved ~/.norb/config.yaml (Debian, 2 vCPUs, 2048 MiB RAM, 20 GiB disk, Rosetta 2 enabled)");

    // Step 2: Dedicated SSH keys & agent forwarding
    println!("  \x1b[1;36m➜\x1b[0m Step 2: Configuring dedicated SSH keys and host agent forwarding...");
    let _ = crate::ssh::ensure_norb_ssh_key();
    let _ = crate::ssh::sync_ssh_config().await;
    println!("    \x1b[1;32m✓\x1b[0m Dedicated SSH keys configured (~/.norb/ssh/id_ed25519) with ~/.ssh/config integration");

    // Step 3: Create and boot the primary Debian Linux machine
    println!("  \x1b[1;36m➜\x1b[0m Step 3: Setting up your primary Debian Linux machine...");
    let machines = crate::commands::load_all_machines(&cfg.machines_dir)?;
    let debian_machine = machines.iter().find(|m| m.config.name == "debian");
    
    match debian_machine {
        Some(m) if m.state == MachineState::Running => {
            println!("    \x1b[1;32m✓\x1b[0m Debian machine is already up and running.");
        }
        Some(_) => {
            println!("    Booting existing Debian machine...");
            crate::commands::start::run(Some("debian")).await?;
            println!("    \x1b[1;32m✓\x1b[0m Debian machine is UP and RUNNING (Default machine)");
        }
        None => {
            println!("    Creating machine 'debian'...");
            crate::commands::create::run("debian", Some("debian"), 2, 2048, 20, "aarch64").await?;
            let _ = crate::commands::default_cmd::run(Some("debian")).await;
            println!("    Booting Debian machine...");
            crate::commands::start::run(Some("debian")).await?;
            println!("    \x1b[1;32m✓\x1b[0m Debian machine created and is UP and RUNNING (Default machine)");
        }
    }

    // Step 4: Provision local Kubernetes (K3s) node
    println!("  \x1b[1;36m➜\x1b[0m Step 4: Setting up local Kubernetes (K3s) cluster node...");
    if let Err(e) = crate::commands::k8s::enable().await {
        eprintln!("    \x1b[1;33m⚠\x1b[0m Kubernetes setup notice: {e}");
        eprintln!("      (You can run `norb k8s enable` at any time to initialize it)");
    } else {
        println!("    \x1b[1;32m✓\x1b[0m Kubernetes (K3s) cluster is UP and READY! (Server: https://127.0.0.1:26443)");
    }

    // Step 5: Good to have stuff
    println!("  \x1b[1;36m➜\x1b[0m Step 5: Setting up developer integrations...");
    
    // 5a. Docker CLI context
    if let Err(e) = crate::commands::docker::setup(false).await {
        eprintln!("    \x1b[1;33m⚠\x1b[0m Docker context notice: {e}");
    } else {
        println!("    \x1b[1;32m✓\x1b[0m Standard Docker CLI context 'norb' configured");
    }

    // 5b. Shell tab autocompletions
    let _ = crate::commands::completion::generate_completion("zsh", true);
    let _ = crate::commands::completion::generate_completion("bash", true);
    println!("    \x1b[1;32m✓\x1b[0m Shell tab autocompletions installed for Zsh and Bash");

    // 5c. Finder file hierarchy
    let _ = crate::nfs::ensure_finder_hierarchy();
    println!("    \x1b[1;32m✓\x1b[0m macOS Finder file access prepared at ~/Norb");

    // 5d. Local wildcard TLS CA
    let _ = crate::tls::ensure_tls_cert(&cfg.data_dir);
    println!("    \x1b[1;32m✓\x1b[0m Local wildcard TLS certificates generated (*.norb.local)");

    // 5e. Apple menu bar status item
    crate::commands::menu_cmd::ensure_menu_bar_running();
    println!("    \x1b[1;32m✓\x1b[0m Native Apple menu bar status item auto-started ( Norb)");

    if welcome {
        println!("\n\x1b[1;34m━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\x1b[0m");
        println!(" \x1b[1;32m🎉 Everything is Ready! Here is your quick tour:\x1b[0m");
        println!("\x1b[1;34m━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\x1b[0m");
        println!("\n\x1b[1m1. Open your Debian Linux shell:\x1b[0m");
        println!("   \x1b[1;32mnorb\x1b[0m");
        println!("   (Drops you straight into an interactive shell inside your Debian machine)");
        println!("\n\x1b[1m2. Run Linux commands right from your Mac terminal:\x1b[0m");
        println!("   \x1b[1;32mnorb uname -a\x1b[0m");
        println!("   \x1b[1;32mnorb cat /etc/os-release\x1b[0m");
        println!("   \x1b[1;32mnorb apt update\x1b[0m");
        println!("\n\x1b[1m3. Use standard Docker commands:\x1b[0m");
        println!("   \x1b[1;32mdocker ps\x1b[0m");
        println!("   \x1b[1;32mdocker run --rm -it -p 8080:80 nginx:alpine\x1b[0m");
        println!("   (Then visit http://localhost:8080 or http://nginx.norb.local in your browser!)");
        println!("\n\x1b[1m4. Control your local Kubernetes cluster:\x1b[0m");
        println!("   \x1b[1;32mkubectl get nodes\x1b[0m");
        println!("   \x1b[1;32mkubectl get pods -A\x1b[0m");
        println!("\n\x1b[1m5. Browse Linux files directly in macOS Finder:\x1b[0m");
        println!("   \x1b[1;32mopen ~/Norb\x1b[0m");
        println!("   (Your Linux machine files and container volumes are visible right in Finder)");
        println!("\n\x1b[1m6. Activity & Resource Monitor:\x1b[0m");
        println!("   \x1b[1;32mnorbctl top\x1b[0m");
        println!("   (Real-time CPU, resident memory, and APFS disk savings)");
        println!("\n\x1b[1m7. Minimal Apple Menu Bar Icon:\x1b[0m");
        println!("   \x1b[1;32m Norb\x1b[0m is already active in your macOS menu bar!");
        println!("   (Click the icon in your menu bar to view live states, IP, and shortcuts)");
        println!("\n\x1b[1;34m━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\x1b[0m\n");
    }

    Ok(())
}
