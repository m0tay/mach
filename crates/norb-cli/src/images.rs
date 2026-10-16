//! Multi-distro image manager and cache for Norb.

use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Raw,
    TarXz,
    RawXz,
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
        format: ImageFormat::TarXz,
    },
    DistroImage {
        name: "ubuntu",
        display_name: "Ubuntu 24.04 LTS (Noble)",
        url: "https://cloud-images.ubuntu.com/minimal/releases/noble/release/ubuntu-24.04-minimal-cloudimg-arm64.img",
        filename: "ubuntu-24.04-minimal-cloudimg-arm64.img",
        format: ImageFormat::Raw,
    },
    DistroImage {
        name: "fedora",
        display_name: "Fedora 40 Cloud",
        url: "https://download.fedoraproject.org/pub/fedora/linux/releases/40/Cloud/aarch64/images/Fedora-Cloud-Base-Generic.aarch64-40-1.14.raw.xz",
        filename: "fedora-40-cloud-base-arm64.raw",
        format: ImageFormat::RawXz,
    },
    DistroImage {
        name: "alpine",
        display_name: "Alpine Linux 3.20",
        url: "https://dl-cdn.alpinelinux.org/alpine/v3.20/releases/aarch64/alpine-virt-3.20.3-aarch64.iso",
        filename: "alpine-3.20-virt-arm64.iso",
        format: ImageFormat::Raw,
    },
    DistroImage {
        name: "arch",
        display_name: "Arch Linux ARM",
        url: "http://os.archlinuxarm.org/os/ArchLinuxARM-aarch64-latest.tar.gz",
        filename: "archlinux-aarch64-latest.tar.gz",
        format: ImageFormat::Raw,
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
        ImageFormat::TarXz => {
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
        }
        ImageFormat::RawXz => {
            // Stream download through xz decompression
            let status = Command::new("sh")
                .arg("-c")
                .arg(format!(
                    "curl -fSL --progress-bar \"{}\" | xz -dc > \"{}\"",
                    distro.url,
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
