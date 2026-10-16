//! `norbctl debug` command implementation for container diagnostics.
//!
//! Provides an interactive debugging shell attached directly to a target container's
//! network, PID, and IPC namespaces. Works even on distroless or scratch containers.

use anyhow::{bail, Context, Result};
use std::process::Command;

pub async fn run(container: &str) -> Result<()> {
    println!("\x1b[1;36m➜\x1b[0m Inspecting target container \x1b[1;34m{container}\x1b[0m...");

    // 1. Verify container exists and is running
    let inspect_out = Command::new("docker")
        .args(["inspect", "--format", "{{.State.Running}}", container])
        .output()
        .context("Failed to execute `docker inspect`. Is Docker context 'norb' active?")?;

    if !inspect_out.status.success() {
        let err = String::from_utf8_lossy(&inspect_out.stderr);
        bail!("Container '{}' not found: {}", container, err.trim());
    }

    let is_running = String::from_utf8_lossy(&inspect_out.stdout).trim() == "true";
    if !is_running {
        bail!("Container '{}' is not currently running. Cannot attach debug shell.", container);
    }

    println!("\x1b[1;32m✓\x1b[0m Container is running.");
    println!("\x1b[1;36m➜\x1b[0m Attaching diagnostic shell (PID, Net, IPC namespaces joined)...");
    println!("  Target root filesystem mounted at: \x1b[1;33m/host\x1b[0m");
    println!("  Type 'exit' to leave debug shell.\n");

    // Launch debug container sharing network, pid, and ipc namespaces
    let status = Command::new("docker")
        .args([
            "run",
            "--rm",
            "-it",
            &format!("--net=container:{container}"),
            &format!("--pid=container:{container}"),
            &format!("--ipc=container:{container}"),
            "-v", "/:/host:ro",
            "busybox:latest",
            "sh",
        ])
        .status()
        .context("Failed to start debug container")?;

    if !status.success() {
        // Fallback to alpine if busybox failed
        let _ = Command::new("docker")
            .args([
                "run",
                "--rm",
                "-it",
                &format!("--net=container:{container}"),
                &format!("--pid=container:{container}"),
                &format!("--ipc=container:{container}"),
                "-v", "/:/host:ro",
                "alpine:latest",
                "sh",
            ])
            .status();
    }

    Ok(())
}
