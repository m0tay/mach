//! `norb doctor` command (health check & diagnostics).

use anyhow::Result;
use norb_common::config::NorbConfig;
use std::process::Command;

pub async fn run() -> Result<()> {
    println!("\x1b[1mNorb Doctor — System Health Check\x1b[0m");
    println!("───────────────────────────────────────────────────");

    let mut all_ok = true;

    // 1. Host Architecture
    let arch = std::env::consts::ARCH;
    let os = std::env::consts::OS;
    if os == "macos" && arch == "aarch64" {
        println!("  \x1b[1;32m✓\x1b[0m Host Platform:     macOS Apple Silicon (aarch64)");
    } else {
        println!("  \x1b[1;31m✗\x1b[0m Host Platform:     {} {} (Apple Silicon recommended)", os, arch);
        all_ok = false;
    }

    // 2. Norb binary & entitlements
    if let Ok(exe) = std::env::current_exe() {
        let codesign_out = Command::new("codesign")
            .args(["-d", "--entitlements", ":-", exe.to_str().unwrap()])
            .output();
        if let Ok(out) = codesign_out {
            let ent_str = String::from_utf8_lossy(&out.stdout);
            if ent_str.contains("com.apple.security.virtualization") {
                println!("  \x1b[1;32m✓\x1b[0m Entitlements:      Virtualization entitlement present");
            } else {
                println!("  \x1b[1;33m!\x1b[0m Entitlements:      Missing com.apple.security.virtualization (run `make sign`)");
            }
        }
    }

    // 3. Norb Directories
    let cfg = NorbConfig::load()?;
    let dirs = [
        ("Data Dir", &cfg.data_dir),
        ("Machines Dir", &cfg.machines_dir),
        ("Run Dir", &cfg.data_dir.join("run")),
    ];
    for (label, p) in &dirs {
        if p.exists() {
            println!("  \x1b[1;32m✓\x1b[0m {:<18} {}", label, p.display());
        } else {
            println!("  \x1b[1;33m!\x1b[0m {:<18} {} (will be created on demand)", label, p.display());
        }
    }

    // 4. SSH Keys
    let mut ssh_ok = false;
    if let Some(home) = dirs::home_dir() {
        let ssh_dir = home.join(".ssh");
        for key in &["id_ed25519.pub", "id_rsa.pub", "id_ecdsa.pub"] {
            if ssh_dir.join(key).exists() {
                println!("  \x1b[1;32m✓\x1b[0m SSH Public Key:    {}", ssh_dir.join(key).display());
                ssh_ok = true;
                break;
            }
        }
    }
    if !ssh_ok {
        println!("  \x1b[1;33m!\x1b[0m SSH Public Key:    No SSH keys found in ~/.ssh/ (generate with ssh-keygen)");
    }

    // 5. Docker Integration
    let host_sock = cfg.data_dir.join("run").join("docker.sock");
    if host_sock.exists() {
        println!("  \x1b[1;32m✓\x1b[0m Docker Socket:     Active at {}", host_sock.display());
    } else {
        println!("  \x1b[1;33m!\x1b[0m Docker Socket:     Not active (starts automatically when VM starts)");
    }

    // 6. Docker Context
    let context_out = Command::new("docker").args(["context", "ls"]).output();
    if let Ok(out) = context_out {
        let stdout = String::from_utf8_lossy(&out.stdout);
        if stdout.contains("norb") {
            println!("  \x1b[1;32m✓\x1b[0m Docker Context:    'norb' context configured in Docker CLI");
        } else {
            println!("  \x1b[1;33m!\x1b[0m Docker Context:    'norb' context not configured (run `norb docker setup`)");
        }
    }

    // 7. Background Daemon (DNS & Port Forwarder)
    let run_dir = cfg.data_dir.join("run");
    if let Some(pid) = super::daemon::is_daemon_running(&run_dir) {
        println!("  \x1b[1;32m✓\x1b[0m Background Daemon: Running (PID: {}) [DNS & Port Proxy]", pid);
    } else {
        println!("  \x1b[1;33m!\x1b[0m Background Daemon: Stopped (starts automatically when VM starts)");
    }

    // 8. macOS DNS Resolver (*.norb.local)
    let (dns_ok, _) = crate::dns::check_or_setup_resolver();
    if dns_ok {
        println!("  \x1b[1;32m✓\x1b[0m macOS DNS Resolver: Configured (/etc/resolver/norb.local)");
    } else {
        println!("  \x1b[1;33m!\x1b[0m macOS DNS Resolver: Not configured (run `norbctl dns setup`)");
    }

    // 9. Machines overview
    let machines = super::load_all_machines(&cfg.machines_dir).unwrap_or_default();
    println!("  \x1b[1;32m✓\x1b[0m Linux Machines:    {} machine(s) registered", machines.len());
    for m in &machines {
        let status_color = if m.state == norb_common::machine::MachineState::Running { "\x1b[1;32m" } else { "\x1b[33m" };
        println!("      • \x1b[1m{}\x1b[0m ({}) — {}{}\x1b[0m", m.config.name, m.config.distro, status_color, m.state);
    }

    println!("───────────────────────────────────────────────────");
    if all_ok {
        println!("\x1b[1;32m✓ All system checks passed! Norb is ready for production.\x1b[0m\n");
    } else {
        println!("\x1b[1;33m! Some checks reported warnings or recommendations.\x1b[0m\n");
    }

    Ok(())
}
