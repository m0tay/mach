//! `norb docker` command implementation.

use anyhow::Result;
use norb_common::config::NorbConfig;
use std::process::Command;

pub async fn setup() -> Result<()> {
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
        println!("\nYou can now run standard Docker commands directly:");
        println!("  \x1b[1;32mdocker ps\x1b[0m");
        println!("  \x1b[1;32mdocker run --rm -v $(pwd):/app -p 8080:80 nginx:alpine\x1b[0m");
    } else {
        let err = String::from_utf8_lossy(&output.stderr);
        eprintln!("\x1b[1;31m✖\x1b[0m Failed to create Docker context: {}", err.trim());
    }

    Ok(())
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
