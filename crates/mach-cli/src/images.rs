//! Multi-distro image manager and cache for mach.

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
    pub mirrors: &'static [crate::mirrors::MirrorEntry],
}

pub const DISTROS: &[DistroImage] = &[
    DistroImage {
        name: "debian",
        display_name: "Debian 12 (Bookworm)",
        url: "https://cloud.debian.org/images/cloud/bookworm/latest/debian-12-genericcloud-arm64.tar.xz",
        filename: "debian-12-genericcloud-arm64.raw",
        format: ImageFormat::TarXz { member: "disk.raw" },
        mirrors: crate::mirrors::DEBIAN_MIRRORS,
    },
    DistroImage {
        name: "ubuntu",
        display_name: "Ubuntu 24.04 LTS (Noble)",
        url: "https://cloud-images.ubuntu.com/releases/noble/release/ubuntu-24.04-server-cloudimg-arm64.tar.gz",
        filename: "ubuntu-24.04-server-cloudimg-arm64.raw",
        format: ImageFormat::TarGz { member: "noble-server-cloudimg-arm64.img" },
        mirrors: crate::mirrors::UBUNTU_MIRRORS,
    },
    DistroImage {
        name: "fedora",
        display_name: "Fedora 40 Cloud",
        url: "https://archives.fedoraproject.org/pub/archive/fedora/linux/releases/40/Cloud/aarch64/images/Fedora-Cloud-Base-AmazonEC2.aarch64-40-1.14.raw.xz",
        filename: "fedora-40-cloud-base-arm64.raw",
        format: ImageFormat::RawXz,
        mirrors: crate::mirrors::FEDORA_MIRRORS,
    },
];

pub fn get_distro(name: &str) -> Option<&'static DistroImage> {
    DISTROS.iter().find(|d| d.name.eq_ignore_ascii_case(name))
}

fn download_and_extract(
    url: &str,
    target_file: &Path,
    format: &ImageFormat,
) -> Result<()> {
    match format {
        ImageFormat::TarXz { member } => {
            let out_file = fs::File::create(target_file)
                .with_context(|| format!("failed to create image file at {}", target_file.display()))?;

            let mut curl_proc = Command::new("curl")
                .args(["-fSL", "--progress-bar", url])
                .stdout(std::process::Stdio::piped())
                .spawn()
                .with_context(|| "failed to spawn curl")?;

            let curl_out = curl_proc.stdout.take().context("failed to pipe curl stdout")?;

            let tar_status = Command::new("tar")
                .args(["-xJf", "-", "-O", member])
                .stdin(curl_out)
                .stdout(out_file)
                .status()
                .with_context(|| "failed to execute tar -xJf")?;

            let curl_status = curl_proc.wait().with_context(|| "failed to wait on curl")?;

            if !curl_status.success() || !tar_status.success() {
                let _ = fs::remove_file(target_file);
                bail!(
                    "failed to download/extract image from {} (curl: {:?}, tar: {:?})",
                    url,
                    curl_status.code(),
                    tar_status.code()
                );
            }
        }
        ImageFormat::TarGz { member } => {
            let out_file = fs::File::create(target_file)
                .with_context(|| format!("failed to create image file at {}", target_file.display()))?;

            let mut curl_proc = Command::new("curl")
                .args(["-fSL", "--progress-bar", url])
                .stdout(std::process::Stdio::piped())
                .spawn()
                .with_context(|| "failed to spawn curl")?;

            let curl_out = curl_proc.stdout.take().context("failed to pipe curl stdout")?;

            let tar_status = Command::new("tar")
                .args(["-xzf", "-", "-O", member])
                .stdin(curl_out)
                .stdout(out_file)
                .status()
                .with_context(|| "failed to execute tar -xzf")?;

            let curl_status = curl_proc.wait().with_context(|| "failed to wait on curl")?;

            if !curl_status.success() || !tar_status.success() {
                let _ = fs::remove_file(target_file);
                bail!(
                    "failed to download/extract image from {} (curl: {:?}, tar: {:?})",
                    url,
                    curl_status.code(),
                    tar_status.code()
                );
            }
        }
        ImageFormat::RawXz => {
            let out_file = fs::File::create(target_file)
                .with_context(|| format!("failed to create image file at {}", target_file.display()))?;

            let mut curl_proc = Command::new("curl")
                .args(["-fSL", "--progress-bar", url])
                .stdout(std::process::Stdio::piped())
                .spawn()
                .with_context(|| "failed to spawn curl")?;

            let curl_out = curl_proc.stdout.take().context("failed to pipe curl stdout")?;

            let has_xz = Command::new("which")
                .arg("xz")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);

            let decomp_status = if has_xz {
                Command::new("xz")
                    .arg("-dc")
                    .stdin(curl_out)
                    .stdout(out_file)
                    .status()
                    .with_context(|| "failed to execute xz -dc")?
            } else {
                Command::new("/usr/bin/python3")
                    .args([
                        "-c",
                        "import sys, lzma, shutil; shutil.copyfileobj(lzma.LZMAFile(sys.stdin.buffer), sys.stdout.buffer)",
                    ])
                    .stdin(curl_out)
                    .stdout(out_file)
                    .status()
                    .with_context(|| "failed to execute python3 lzma decompression")?
            };

            let curl_status = curl_proc.wait().with_context(|| "failed to wait on curl")?;

            if !curl_status.success() || !decomp_status.success() {
                let _ = fs::remove_file(target_file);
                bail!(
                    "failed to download/decompress image from {} (curl: {:?}, decomp: {:?})",
                    url,
                    curl_status.code(),
                    decomp_status.code()
                );
            }
        }
        ImageFormat::Raw => {
            let status = Command::new("curl")
                .args([
                    "-fSL",
                    "--progress-bar",
                    "-o",
                    target_file.to_str().unwrap(),
                    url,
                ])
                .status()
                .with_context(|| "failed to download disk image")?;

            if !status.success() {
                let _ = fs::remove_file(target_file);
                bail!("failed to download image from {} (status: {:?})", url, status.code());
            }
        }
    }

    Ok(())
}

pub async fn ensure_cached_image(cache_dir: &Path, distro: &DistroImage) -> Result<PathBuf> {
    fs::create_dir_all(cache_dir)?;
    let target_file = cache_dir.join(distro.filename);
    if target_file.exists() && target_file.metadata()?.len() > 1024 * 1024 {
        return Ok(target_file);
    }

    println!("  \x1b[1;36m➜\x1b[0m Probing mirrors for \x1b[1m{}\x1b[0m...", distro.display_name);

    let ranked_mirrors = if !distro.mirrors.is_empty() {
        crate::mirrors::rank_mirrors(distro.mirrors).await
    } else {
        vec![]
    };

    if let Some((best, lat)) = ranked_mirrors.first() {
        if let Some(duration) = lat {
            println!(
                "  \x1b[1;32m✓\x1b[0m Selected fastest mirror: \x1b[1m{}\x1b[0m ({}) [{}ms]",
                best.name,
                best.region,
                duration.as_millis()
            );
        } else {
            println!(
                "  \x1b[1;33m⚠\x1b[0m All mirror probes timed out; falling back to primary mirror: \x1b[1m{}\x1b[0m ({})",
                best.name,
                best.region
            );
        }
    }

    // Build the ordered download candidate URLs (fastest first, fallback next)
    let mut candidates: Vec<(&str, &str)> = ranked_mirrors.iter().map(|(m, _)| (m.name, m.url)).collect();
    if candidates.is_empty() {
        candidates.push(("Upstream URL", distro.url));
    }

    let mut last_error = None;
    for (idx, (mirror_name, url)) in candidates.iter().enumerate() {
        if idx > 0 {
            eprintln!(
                "  \x1b[1;33m⚠\x1b[0m Retrying with fallback mirror \x1b[1m{}\x1b[0m...",
                mirror_name
            );
        }
        println!("  \x1b[1;36m➜\x1b[0m Downloading \x1b[1m{}\x1b[0m cloud image from {}...", distro.display_name, mirror_name);

        match download_and_extract(url, &target_file, &distro.format) {
            Ok(()) => {
                println!("  \x1b[1;32m✓\x1b[0m Cached \x1b[1m{}\x1b[0m to {}", distro.display_name, target_file.display());
                return Ok(target_file);
            }
            Err(e) => {
                eprintln!("  \x1b[1;31m✗\x1b[0m Download failed from {} ({}): {:#}", mirror_name, url, e);
                let _ = fs::remove_file(&target_file);
                last_error = Some(e);
            }
        }
    }

    bail!(
        "Failed to download {} from all available mirrors. Last error: {:?}",
        distro.display_name,
        last_error
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_distros_mirror_integration() {
        for distro in DISTROS {
            assert!(!distro.mirrors.is_empty(), "Distro {} should have mirrors attached", distro.name);
            assert_eq!(distro.mirrors[0].url, distro.url, "First mirror URL should match distro.url for {}", distro.name);
        }
    }

    #[test]
    fn test_get_distro_cases() {
        assert!(get_distro("debian").is_some());
        assert!(get_distro("DEBIAN").is_some());
        assert!(get_distro("Ubuntu").is_some());
        assert!(get_distro("fedora").is_some());
        assert!(get_distro("nonexistent").is_none());
    }
}
