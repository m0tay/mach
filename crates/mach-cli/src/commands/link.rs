//! Host binary link implementation for mach.

use anyhow::{bail, Context, Result};
use mach_common::config::MachConfig;
use mach_common::machine::MachineState;
use std::process::Command;

/// Link a host command to run seamlessly inside a Linux machine.
///
/// Verifies that `binary` exists on macOS, resolves the target machine (auto-starting if needed),
/// and generates an executable wrapper script at `/usr/local/bin/<binary>` inside the guest
/// that routes execution through `mac exec <binary>` over the host bridge (192.168.64.1:32222).
pub async fn run(binary: &str, machine_opt: Option<&str>) -> Result<()> {
    if binary.contains('/') || binary.contains('\\') || binary.trim().is_empty() {
        bail!("invalid binary name '{binary}': path separators not allowed");
    }

    // 1. Check if binary exists on macOS host using `which <binary>`
    let which_out = Command::new("which")
        .arg(binary)
        .output()
        .with_context(|| format!("failed to check binary '{binary}' on host"))?;

    if !which_out.status.success() {
        bail!("binary '{binary}' not found on macOS host");
    }

    let host_path = String::from_utf8_lossy(&which_out.stdout).trim().to_string();
    if host_path.is_empty() {
        bail!("binary '{binary}' not found on macOS host");
    }

    // 2. Resolve target machine
    let cfg = MachConfig::load()?;
    let mut machine = super::resolve_target_machine(&cfg, machine_opt)?;

    // Auto-start machine if stopped
    if machine.state != MachineState::Running {
        println!("\x1b[1;36m➜\x1b[0m Starting machine \x1b[1m{}\x1b[0m...", machine.config.name);
        super::start::run(Some(&machine.config.name)).await?;
        machine = super::find_machine(&cfg.machines_dir, &machine.config.name)?
            .with_context(|| format!("machine '{}' not found after start", machine.config.name))?;
    }

    let ip = machine
        .ip_address
        .with_context(|| format!("machine '{}' has no IP address assigned", machine.config.name))?;

    // 3. Ensure host bridge mac CLI is installed in guest
    crate::bridge::install_guest_mac_cli(ip)?;

    // 4. Generate executable wrapper script in guest
    crate::bridge::install_guest_link(ip, binary)?;

    println!(
        "\x1b[1;32m✓\x1b[0m Linked host command \x1b[1m{}\x1b[0m ({}) to \x1b[1m/usr/local/bin/{}\x1b[0m inside \x1b[1m{}\x1b[0m",
        binary, host_path, binary, machine.config.name
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_link_rejects_unsafe_binary() {
        let res = run("../bin/sh", None).await;
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("path separators not allowed"));
    }

    #[tokio::test]
    async fn test_link_rejects_nonexistent_binary() {
        let res = run("mach_test_nonexistent_bin_12345", None).await;
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("not found on macOS host"));
    }
}
