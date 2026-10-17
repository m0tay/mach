//! Cloud-init ISO generator for multi-distro UEFI boot.

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;
use std::process::Command;
use uuid::Uuid;

/// Locate the host user's SSH public keys to inject into the VM, including Mach's dedicated key.
pub fn get_host_ssh_keys() -> Vec<String> {
    let mut keys = Vec::new();

    // Include Mach's dedicated key
    if let Ok(_) = crate::ssh::ensure_mach_ssh_key() {
        let cfg = mach_common::config::MachConfig::load().unwrap_or_default();
        let pub_key_path = cfg.data_dir.join("ssh").join("id_ed25519.pub");
        if let Ok(content) = fs::read_to_string(&pub_key_path) {
            let trimmed = content.trim();
            if !trimmed.is_empty() {
                keys.push(trimmed.to_string());
            }
        }
    }

    if let Some(home) = dirs::home_dir() {
        let ssh_dir = home.join(".ssh");
        for key_name in &["id_ed25519.pub", "id_rsa.pub", "id_ecdsa.pub"] {
            let key_file = ssh_dir.join(key_name);
            if let Ok(content) = fs::read_to_string(&key_file) {
                let trimmed = content.trim();
                if !trimmed.is_empty() && !keys.contains(&trimmed.to_string()) {
                    keys.push(trimmed.to_string());
                }
            }
        }
    }
    keys
}

/// RAII guard to ensure temporary cloud-init directory is always cleaned up on exit.
struct TempDirGuard {
    path: std::path::PathBuf,
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
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
    let _guard = TempDirGuard { path: temp_dir.clone() };

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
bootcmd:
  - mkdir -p /etc/systemd/network
  - printf '[Match]\nName=e*\n\n[Network]\nDHCP=ipv4\n\n[DHCPv4]\nClientIdentifier=mac\n' > /etc/systemd/network/99-mach-mac.network
chpasswd:
  list: |
    root:root
    debian:debian
    ubuntu:ubuntu
    fedora:fedora
  expire: False
users:
  - name: root
    ssh_authorized_keys:
{keys}    shell: /bin/bash
  - name: debian
    gecos: Debian User
    sudo: ALL=(ALL) NOPASSWD:ALL
    ssh_authorized_keys:
{keys}    shell: /bin/bash
  - name: ubuntu
    gecos: Ubuntu User
    sudo: ALL=(ALL) NOPASSWD:ALL
    ssh_authorized_keys:
{keys}    shell: /bin/bash
  - name: fedora
    gecos: Fedora User
    sudo: ALL=(ALL) NOPASSWD:ALL
    ssh_authorized_keys:
{keys}    shell: /bin/bash
  - name: mach
    gecos: mach user
    sudo: ALL=(ALL) NOPASSWD:ALL
    ssh_authorized_keys:
{keys}    shell: /bin/bash
ssh_pwauth: true
disable_root: false
runcmd:
  - mount -o remount,rw / || true
  - sed -i 's/^#*PermitRootLogin.*/PermitRootLogin yes/' /etc/ssh/sshd_config || true
  - mkdir -p /etc/ssh/sshd_config.d && echo "PermitRootLogin yes" > /etc/ssh/sshd_config.d/mach.conf || true
  - systemctl restart ssh || systemctl restart sshd || true
  - mkdir -p /Users
  - mount -t virtiofs mach_share /Users || true
  - echo "mach_share /Users virtiofs defaults 0 0" >> /etc/fstab
  - mkdir -p /media/rosetta
  - mount -t virtiofs rosetta /media/rosetta || true
  - echo "rosetta /media/rosetta virtiofs defaults 0 0" >> /etc/fstab
  - echo ':rosetta:M::\x7fELF\x02\x01\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x02\x00\x3e\x00:\xff\xff\xff\xff\xff\xfe\xfe\x00\xff\xff\xff\xff\xff\xff\xff\xff\xfe\xff\xff\xff:/media/rosetta/rosetta:CF' > /proc/sys/fs/binfmt_misc/register || true
"#,
        keys = keys_yaml
    );
    fs::write(temp_dir.join("user-data"), user_data)?;

    let network_config = r#"version: 2
ethernets:
  all-eth:
    match:
      name: "e*"
    dhcp4: true
    dhcp-identifier: mac
"#;
    fs::write(temp_dir.join("network-config"), network_config)?;

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

    if !status.status.success() {
        let err = String::from_utf8_lossy(&status.stderr);
        anyhow::bail!("failed to create cidata.iso: {}", err.trim());
    }

    Ok(())
}
