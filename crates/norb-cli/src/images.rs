//! Multi-distro image manager and cache for Norb.

use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct DistroImage {
    pub name: &'static str,
    pub display_name: &'static str,
    pub url: &'static str,
    pub filename: &'static str,
    pub is_tar_xz: bool,
}

pub const DISTROS: &[DistroImage] = &[
    DistroImage {
        name: "debian",
        display_name: "Debian 12 (Bookworm)",
        url: "https://cloud.debian.org/images/cloud/bookworm/latest/debian-12-genericcloud-arm64.tar.xz",
        filename: "debian-12-genericcloud-arm64.raw",
        is_tar_xz: true,
    },
    DistroImage {
        name: "ubuntu",
        display_name: "Ubuntu 24.04 LTS (Noble)",
        url: "https://cloud-images.ubuntu.com/minimal/releases/noble/release/ubuntu-24.04-minimal-cloudimg-arm64.img",
        filename: "ubuntu-24.04-minimal-cloudimg-arm64.img",
        is_tar_xz: false,
    },
];

pub fn get_distro(name: &str) -> Option<&'static DistroImage> {
    DISTROS.iter().find(|d| d.name.eq_ignore_ascii_case(name))
}

pub fn ensure_cached_image(cache_dir: &Path, distro: &DistroImage) -> Result<PathBuf> {
    fs::create_dir_all(cache_dir)?;
    let target_file = cache_dir.join(distro.filename);
    if target_file.exists() && target_file.metadata()?.len() > 1024 * 1024 {
        return Ok(target_file);
    }

    println!("  \x1b[1;36m➜\x1b[0m Downloading \x1b[1m{}\x1b[0m cloud image...", distro.display_name);

    if distro.is_tar_xz {
        // Stream download directly into tar extraction so disk.raw is written directly
        let status = Command::new("sh")
            .arg("-c")
            .arg(format!(
                "curl -fSL --progress-bar \"{}\" | tar -xJf - -O disk.raw > \"{}\"",
                distro.url,
                target_file.display()
            ))
            .status()
            .with_context(|| "failed to download and extract disk image")?;

        if !status.success() {
            let _ = fs::remove_file(&target_file);
            bail!("failed to download/extract image from {}", distro.url);
        }
    } else {
        let status = Command::new("curl")
            .args([
                "-fSL",
                "--progress-bar",
                "-o",
                target_file.to_str().unwrap(),
                distro.url,
            ])
            .status()
            .with_context(|| "failed to download disk image")?;

        if !status.success() {
            let _ = fs::remove_file(&target_file);
            bail!("failed to download image from {}", distro.url);
        }
    }

    println!("  \x1b[1;32m✓\x1b[0m Cached \x1b[1m{}\x1b[0m to {}", distro.display_name, target_file.display());
    Ok(target_file)
}
