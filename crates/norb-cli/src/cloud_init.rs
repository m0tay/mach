//! Cloud-init ISO generator for multi-distro UEFI boot.

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;
use std::process::Command;
use uuid::Uuid;

/// Locate the host user's SSH public keys to inject into the VM.
pub fn get_host_ssh_keys() -> Vec<String> {
    let mut keys = Vec::new();
    if let Some(home) = dirs::home_dir() {
        let ssh_dir = home.join(".ssh");
        for key_name in &["id_ed25519.pub", "id_rsa.pub", "id_ecdsa.pub"] {
            let key_file = ssh_dir.join(key_name);
            if let Ok(content) = fs::read_to_string(&key_file) {
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    keys.push(trimmed.to_string());
                }
            }
        }
    }
    keys
}

/// Generate a cloud-init NoCloud ISO image (`cidata.iso`) using macOS native `hdiutil`.
pub fn generate_cidata_iso(
    machine_id: &Uuid,
    machine_name: &str,
    output_iso: &Path,
) -> Result<()> {
    let temp_dir = std::env::temp_dir().join(format!("cidata_{}", machine_id));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir)?;

    let meta_data = format!(
        "instance-id: {}\nlocal-hostname: {}\n",
        machine_id, machine_name
    );
    fs::write(temp_dir.join("meta-data"), meta_data)?;

    let host_keys = get_host_ssh_keys();
    let mut keys_yaml = String::new();
    for k in &host_keys {
        keys_yaml.push_str(&format!("      - \"{}\"\n", k));
    }

    let user_data = format!(
r#"#cloud-config
users:
  - name: root
    ssh_authorized_keys:
{keys}    shell: /bin/bash
  - name: debian
    gecos: Debian User
    sudo: ALL=(ALL) NOPASSWD:ALL
    ssh_authorized_keys:
{keys}    shell: /bin/bash
  - name: norb
    gecos: Norb User
    sudo: ALL=(ALL) NOPASSWD:ALL
    ssh_authorized_keys:
{keys}    shell: /bin/bash
ssh_pwauth: true
disable_root: false
runcmd:
  - mount -o remount,rw / || true
  - sed -i 's/^#*PermitRootLogin.*/PermitRootLogin prohibit-password/' /etc/ssh/sshd_config || true
  - mkdir -p /etc/ssh/sshd_config.d && echo "PermitRootLogin prohibit-password" > /etc/ssh/sshd_config.d/norb.conf || true
  - systemctl restart ssh || systemctl restart sshd || true
  - mkdir -p /Users
  - mount -t virtiofs norb_share /Users || true
  - echo "norb_share /Users virtiofs defaults 0 0" >> /etc/fstab
"#,
        keys = keys_yaml
    );
    fs::write(temp_dir.join("user-data"), user_data)?;

    let _ = fs::remove_file(output_iso);

    let status = Command::new("hdiutil")
        .args([
            "makehybrid",
            "-iso",
            "-joliet",
            "-default-volume-name",
            "cidata",
            "-o",
            output_iso.to_str().unwrap(),
            temp_dir.to_str().unwrap(),
        ])
        .output()
        .with_context(|| "failed to execute hdiutil to create cidata.iso")?;

    let _ = fs::remove_dir_all(&temp_dir);

    if !status.status.success() {
        let err = String::from_utf8_lossy(&status.stderr);
        anyhow::bail!("failed to create cidata.iso: {}", err.trim());
    }

    Ok(())
}
