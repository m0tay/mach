//! SSH identity management for Norb machines.

use anyhow::{bail, Context, Result};
use std::fs;
use std::net::IpAddr;
use std::path::PathBuf;
use std::process::Command;

/// Ensure the Norb dedicated SSH key pair exists in `~/.norb/ssh/id_ed25519`.
/// Returns the path to the private key.
pub fn ensure_norb_ssh_key() -> Result<PathBuf> {
    let data_dir = norb_common::config::NorbConfig::default().data_dir;
    let ssh_dir = data_dir.join("ssh");
    fs::create_dir_all(&ssh_dir)?;

    let priv_key = ssh_dir.join("id_ed25519");
    let pub_key = ssh_dir.join("id_ed25519.pub");

    if !priv_key.exists() || !pub_key.exists() {
        let _ = fs::remove_file(&priv_key);
        let _ = fs::remove_file(&pub_key);

        let output = Command::new("ssh-keygen")
            .args([
                "-t", "ed25519",
                "-f", priv_key.to_str().unwrap(),
                "-N", "",
                "-C", "norb@macos",
            ])
            .output()
            .with_context(|| "failed to run ssh-keygen to generate Norb SSH key")?;

        if !output.status.success() {
            bail!("ssh-keygen failed: {}", String::from_utf8_lossy(&output.stderr));
        }
    }

    Ok(priv_key)
}

/// Returns standard SSH arguments for non-interactive Norb commands.
pub fn norb_ssh_args() -> Vec<String> {
    let mut args = vec![
        "-o".to_string(), "StrictHostKeyChecking=no".to_string(),
        "-o".to_string(), "UserKnownHostsFile=/dev/null".to_string(),
        "-o".to_string(), "LogLevel=ERROR".to_string(),
    ];
    if let Ok(key) = ensure_norb_ssh_key() {
        args.push("-i".to_string());
        args.push(key.to_string_lossy().to_string());
    }
    args
}

/// Create a pre-configured SSH Command targeting `user@ip`.
/// Configures `-o StrictHostKeyChecking=no`, `-o UserKnownHostsFile=/dev/null`, `-o LogLevel=ERROR`,
/// includes Norb dedicated SSH key, and targets `user@ip`.
pub fn new_ssh_command(ip: &IpAddr, user: &str) -> Command {
    let mut cmd = Command::new("ssh");
    cmd.args(norb_ssh_args());
    cmd.arg(format!("{user}@{ip}"));
    cmd
}

/// Synchronize the Norb managed block in `~/.ssh/config`.
pub async fn sync_ssh_config() -> Result<()> {
    Ok(())
}
