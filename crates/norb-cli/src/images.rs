//! Multi-distro image manager and cache for Norb.

use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageFormat {
    TarXz { member: &'static str },
    TarGz { member: &'static str },
    RawXz,
    Raw,
}

pub struct DistroImage {
    pub name: &'static str,
    pub display_name: &'static str,
    pub url: &'static str,
    pub filename: &'static str,
    pub format: ImageFormat,
}

pub const DISTROS: &[DistroImage] = &[
    DistroImage {
        name: "debian",
        display_name: "Debian 12 (Bookworm)",
        url: "https://cloud.debian.org/images/cloud/bookworm/latest/debian-12-genericcloud-arm64.tar.xz",
        filename: "debian-12-genericcloud-arm64.raw",
        format: ImageFormat::TarXz { member: "disk.raw" },
    },
    DistroImage {
        name: "ubuntu",
        display_name: "Ubuntu 24.04 LTS (Noble)",
        url: "https://cloud-images.ubuntu.com/releases/noble/release/ubuntu-24.04-server-cloudimg-arm64.tar.gz",
        filename: "ubuntu-24.04-server-cloudimg-arm64.raw",
        format: ImageFormat::TarGz { member: "noble-server-cloudimg-arm64.img" },
    },
    DistroImage {
        name: "fedora",
        display_name: "Fedora 40 Cloud",
        url: "https://archives.fedoraproject.org/pub/archive/fedora/linux/releases/40/Cloud/aarch64/images/Fedora-Cloud-Base-AmazonEC2.aarch64-40-1.14.raw.xz",
        filename: "fedora-40-cloud-base-arm64.raw",
        format: ImageFormat::RawXz,
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

    match distro.format {
        ImageFormat::TarXz { member } => {
            let status = Command::new("sh")
                .arg("-c")
                .arg(format!(
                    "curl -fSL --progress-bar \"{}\" | tar -xJf - -O \"{}\" > \"{}\"",
                    distro.url,
                    member,
                    target_file.display()
                ))
                .status()
                .with_context(|| "failed to download and extract tar.xz disk image")?;

            if !status.success() {
                let _ = fs::remove_file(&target_file);
                bail!("failed to download/extract image from {}", distro.url);
            }
        }
        ImageFormat::TarGz { member } => {
            let status = Command::new("sh")
                .arg("-c")
                .arg(format!(
                    "curl -fSL --progress-bar \"{}\" | tar -xzf - -O \"{}\" > \"{}\"",
                    distro.url,
                    member,
                    target_file.display()
                ))
                .status()
                .with_context(|| "failed to download and extract tar.gz disk image")?;

            if !status.success() {
                let _ = fs::remove_file(&target_file);
                bail!("failed to download/extract image from {}", distro.url);
            }
        }
        ImageFormat::RawXz => {
            // Check if xz CLI exists, else fallback to python3 lzma streaming
            let decompress_cmd = if Command::new("which").arg("xz").output().map(|o| o.status.success()).unwrap_or(false) {
                "xz -dc"
            } else {
                "/usr/bin/python3 -c 'import sys, lzma, shutil; shutil.copyfileobj(lzma.LZMAFile(sys.stdin.buffer), sys.stdout.buffer)'"
            };

            let status = Command::new("sh")
                .arg("-c")
                .arg(format!(
                    "curl -fSL --progress-bar \"{}\" | {} > \"{}\"",
                    distro.url,
                    decompress_cmd,
                    target_file.display()
                ))
                .status()
                .with_context(|| "failed to download and decompress xz image")?;

            if !status.success() {
                let _ = fs::remove_file(&target_file);
                bail!("failed to download/decompress image from {}", distro.url);
            }
        }
        ImageFormat::Raw => {
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
    }

    println!("  \x1b[1;32m✓\x1b[0m Cached \x1b[1m{}\x1b[0m to {}", distro.display_name, target_file.display());
    Ok(target_file)
}
