//! `norb docker` command implementation.

use anyhow::Result;
use norb_common::config::NorbConfig;
use std::path::Path;
use std::process::Command;

pub async fn setup(system: bool) -> Result<()> {
    let cfg = NorbConfig::load()?;
    let sock_path = cfg.data_dir.join("run").join("docker.sock");

    println!("\x1b[1;36m➜\x1b[0m Configuring Docker CLI context for Norb...");

    // Remove existing if present
    let _ = Command::new("docker")
        .args(["context", "rm", "norb"])
        .output();

    let output = Command::new("docker")
        .args([
            "context",
            "create",
            "norb",
            "--docker",
            &format!("host=unix://{}", sock_path.display()),
            "--description",
            "Norb Linux VM (Apple Silicon)",
        ])
        .output()?;

    if output.status.success() {
        // Switch to norb context
        let _ = Command::new("docker")
            .args(["context", "use", "norb"])
            .output();

        println!("\x1b[1;32m✓\x1b[0m Docker context \x1b[1mnorb\x1b[0m configured and activated!");
        println!("  Endpoint: \x1b[1;34munix://{}\x1b[0m", sock_path.display());
    } else {
        let err = String::from_utf8_lossy(&output.stderr);
        eprintln!("\x1b[1;31m✖\x1b[0m Failed to create Docker context: {}", err.trim());
    }

    if system {
        setup_system_symlink(&sock_path);
    } else {
        let sys_sock = Path::new("/var/run/docker.sock");
        if !sys_sock.exists() {
            println!("\n\x1b[1;33mTip:\x1b[0m To make legacy tools & IDE plugins (VS Code Dev Containers, Testcontainers) work automatically, run:");
            println!("  \x1b[1;32mnorbctl docker setup --system\x1b[0m");
        }
    }

    println!("\nYou can now run standard Docker commands directly:");
    println!("  \x1b[1;32mdocker ps\x1b[0m");
    println!("  \x1b[1;32mdocker run --rm -v $(pwd):/app -p 8080:80 nginx:alpine\x1b[0m");

    Ok(())
}

pub fn setup_system_symlink(sock_path: &Path) {
    let sys_sock = Path::new("/var/run/docker.sock");
    if sys_sock.is_symlink() {
        if let Ok(target) = std::fs::read_link(sys_sock) {
            if target == sock_path {
                println!("\x1b[1;32m✓\x1b[0m System /var/run/docker.sock is already linked to Norb.");
                return;
            }
        }
    }

    println!("\x1b[1;36m➜\x1b[0m Linking /var/run/docker.sock to {}...", sock_path.display());
    let res = Command::new("sudo")
        .args(["ln", "-sf", sock_path.to_str().unwrap(), "/var/run/docker.sock"])
        .status();

    if let Ok(st) = res {
        if st.success() {
            println!("\x1b[1;32m✓\x1b[0m Successfully linked /var/run/docker.sock!");
            return;
        }
    }

    println!("\x1b[1;33m!\x1b[0m Could not create symlink automatically. Run manually:\n");
    println!("  \x1b[1;32msudo ln -sf {} /var/run/docker.sock\x1b[0m\n", sock_path.display());
}

pub async fn status() -> Result<()> {
    let cfg = NorbConfig::load()?;
    let sock_path = cfg.data_dir.join("run").join("docker.sock");

    println!("\x1b[1;36m➜\x1b[0m Checking Norb Docker status...");
    println!("  Socket path: \x1b[1m{}\x1b[0m", sock_path.display());

    if sock_path.exists() {
        println!("  Socket file: \x1b[1;32m● present\x1b[0m");
        let output = Command::new("docker")
            .args(["--context", "norb", "version", "--format", "{{.Server.Version}}"])
            .output();
        if let Ok(out) = output {
            if out.status.success() {
                let ver = String::from_utf8_lossy(&out.stdout).trim().to_string();
                println!("  Engine status: \x1b[1;32m● connected\x1b[0m (version: {ver})");
                return Ok(());
            }
        }
        println!("  Engine status: \x1b[1;33m○ socket exists but daemon not responding\x1b[0m");
    } else {
        println!("  Socket file: \x1b[1;31m✖ not found\x1b[0m");
        println!("\nStart a machine with \x1b[1;34mnorb start\x1b[0m first.");
    }

    Ok(())
}
