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

use crate::progress;

pub fn ensure_cached_image(cache_dir: &Path, distro: &DistroImage) -> Result<PathBuf> {
    fs::create_dir_all(cache_dir)?;
    let target_file = cache_dir.join(distro.filename);
    if target_file.exists() && target_file.metadata()?.len() > 1024 * 1024 {
        return Ok(target_file);
    }

    let meta = progress::probe_remote_url(distro.url);
    let mut pb = progress::ProgressBar::new(
        distro.display_name.to_string(),
        Some(meta.provider_tag),
        meta.content_length,
    );

    match distro.format {
        ImageFormat::TarXz { member } => {
            let out_file = fs::File::create(&target_file)
                .with_context(|| format!("failed to create image file at {}", target_file.display()))?;

            let mut curl_proc = Command::new("curl")
                .args(["-f", "-s", "-S", "-L", distro.url])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .with_context(|| "failed to spawn curl")?;

            let mut tar_proc = Command::new("tar")
                .args(["-xJf", "-", "-O", member])
                .stdin(std::process::Stdio::piped())
                .stdout(out_file)
                .stderr(std::process::Stdio::piped())
                .spawn()
                .with_context(|| "failed to spawn tar -xJf")?;

            let mut curl_out = curl_proc.stdout.take().context("failed to pipe curl stdout")?;
            let mut tar_in = tar_proc.stdin.take().context("failed to open tar stdin")?;

            let copy_res = progress::copy_with_progress(&mut curl_out, &mut tar_in, &mut pb);
            drop(tar_in);

            let tar_status = tar_proc.wait().with_context(|| "failed to wait on tar")?;
            let curl_status = curl_proc.wait().with_context(|| "failed to wait on curl")?;

            if copy_res.is_err() || !curl_status.success() || !tar_status.success() {
                let _ = fs::remove_file(&target_file);
                let mut err_msg = String::new();
                if let Some(mut err_pipe) = curl_proc.stderr.take() {
                    let _ = std::io::Read::read_to_string(&mut err_pipe, &mut err_msg);
                }
                bail!(
                    "failed to download/extract image from {} (curl: {:?}, tar: {:?}, err: {})",
                    distro.url,
                    curl_status.code(),
                    tar_status.code(),
                    err_msg.trim()
                );
            }

            let downloaded = copy_res.unwrap_or(0);
            pb.finish_with_message(&format!(
                "{} cloud image ready ({})",
                distro.display_name,
                progress::format_bytes(downloaded)
            ));
        }
        ImageFormat::TarGz { member } => {
            let out_file = fs::File::create(&target_file)
                .with_context(|| format!("failed to create image file at {}", target_file.display()))?;

            let mut curl_proc = Command::new("curl")
                .args(["-f", "-s", "-S", "-L", distro.url])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .with_context(|| "failed to spawn curl")?;

            let mut tar_proc = Command::new("tar")
                .args(["-xzf", "-", "-O", member])
                .stdin(std::process::Stdio::piped())
                .stdout(out_file)
                .stderr(std::process::Stdio::piped())
                .spawn()
                .with_context(|| "failed to spawn tar -xzf")?;

            let mut curl_out = curl_proc.stdout.take().context("failed to pipe curl stdout")?;
            let mut tar_in = tar_proc.stdin.take().context("failed to open tar stdin")?;

            let copy_res = progress::copy_with_progress(&mut curl_out, &mut tar_in, &mut pb);
            drop(tar_in);

            let tar_status = tar_proc.wait().with_context(|| "failed to wait on tar")?;
            let curl_status = curl_proc.wait().with_context(|| "failed to wait on curl")?;

            if copy_res.is_err() || !curl_status.success() || !tar_status.success() {
                let _ = fs::remove_file(&target_file);
                let mut err_msg = String::new();
                if let Some(mut err_pipe) = curl_proc.stderr.take() {
                    let _ = std::io::Read::read_to_string(&mut err_pipe, &mut err_msg);
                }
                bail!(
                    "failed to download/extract image from {} (curl: {:?}, tar: {:?}, err: {})",
                    distro.url,
                    curl_status.code(),
                    tar_status.code(),
                    err_msg.trim()
                );
            }

            let downloaded = copy_res.unwrap_or(0);
            pb.finish_with_message(&format!(
                "{} cloud image ready ({})",
                distro.display_name,
                progress::format_bytes(downloaded)
            ));
        }
        ImageFormat::RawXz => {
            let out_file = fs::File::create(&target_file)
                .with_context(|| format!("failed to create image file at {}", target_file.display()))?;

            let mut curl_proc = Command::new("curl")
                .args(["-f", "-s", "-S", "-L", distro.url])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .with_context(|| "failed to spawn curl")?;

            let has_xz = Command::new("which")
                .arg("xz")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);

            let (mut decomp_proc, mut decomp_in) = if has_xz {
                let mut p = Command::new("xz")
                    .arg("-dc")
                    .stdin(std::process::Stdio::piped())
                    .stdout(out_file)
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .with_context(|| "failed to spawn xz -dc")?;
                let in_pipe = p.stdin.take().context("failed to open xz stdin")?;
                (p, in_pipe)
            } else {
                let mut p = Command::new("/usr/bin/python3")
                    .args([
                        "-c",
                        "import sys, lzma, shutil; shutil.copyfileobj(lzma.LZMAFile(sys.stdin.buffer), sys.stdout.buffer)",
                    ])
                    .stdin(std::process::Stdio::piped())
                    .stdout(out_file)
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .with_context(|| "failed to spawn python3 lzma decompression")?;
                let in_pipe = p.stdin.take().context("failed to open python3 stdin")?;
                (p, in_pipe)
            };

            let mut curl_out = curl_proc.stdout.take().context("failed to pipe curl stdout")?;
            let copy_res = progress::copy_with_progress(&mut curl_out, &mut decomp_in, &mut pb);
            drop(decomp_in);

            let decomp_status = decomp_proc.wait().with_context(|| "failed to wait on decomp")?;
            let curl_status = curl_proc.wait().with_context(|| "failed to wait on curl")?;

            if copy_res.is_err() || !curl_status.success() || !decomp_status.success() {
                let _ = fs::remove_file(&target_file);
                let mut err_msg = String::new();
                if let Some(mut err_pipe) = curl_proc.stderr.take() {
                    let _ = std::io::Read::read_to_string(&mut err_pipe, &mut err_msg);
                }
                bail!(
                    "failed to download/decompress image from {} (curl: {:?}, decomp: {:?}, err: {})",
                    distro.url,
                    curl_status.code(),
                    decomp_status.code(),
                    err_msg.trim()
                );
            }

            let downloaded = copy_res.unwrap_or(0);
            pb.finish_with_message(&format!(
                "{} cloud image ready ({})",
                distro.display_name,
                progress::format_bytes(downloaded)
            ));
        }
        ImageFormat::Raw => {
            let mut out_file = fs::File::create(&target_file)
                .with_context(|| format!("failed to create image file at {}", target_file.display()))?;

            let mut curl_proc = Command::new("curl")
                .args(["-f", "-s", "-S", "-L", distro.url])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .with_context(|| "failed to spawn curl")?;

            let mut curl_out = curl_proc.stdout.take().context("failed to pipe curl stdout")?;
            let copy_res = progress::copy_with_progress(&mut curl_out, &mut out_file, &mut pb);
            drop(out_file);

            let curl_status = curl_proc.wait().with_context(|| "failed to wait on curl")?;

            if copy_res.is_err() || !curl_status.success() {
                let _ = fs::remove_file(&target_file);
                let mut err_msg = String::new();
                if let Some(mut err_pipe) = curl_proc.stderr.take() {
                    let _ = std::io::Read::read_to_string(&mut err_pipe, &mut err_msg);
                }
                bail!(
                    "failed to download image from {} (curl: {:?}, err: {})",
                    distro.url,
                    curl_status.code(),
                    err_msg.trim()
                );
            }

            let downloaded = copy_res.unwrap_or(0);
            pb.finish_with_message(&format!(
                "{} cloud image ready ({})",
                distro.display_name,
                progress::format_bytes(downloaded)
            ));
        }
    }

    Ok(target_file)
}
