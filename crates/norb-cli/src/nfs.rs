//! Loopback NFS / file sharing integration for Norb.

use anyhow::{bail, Context, Result};
use norb_common::config::NorbConfig;
use norb_common::machine::MachineState;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Name of the Norb top-level directory in user home.
pub const NORB_ROOT_DIR: &str = "Norb";
/// Subdirectory holding VM machine filesystem mounts.
pub const MACHINES_SUBDIR: &str = "machines";
/// Subdirectory holding container / persistent volumes.
pub const VOLUMES_SUBDIR: &str = "volumes";

/// Descriptor for the Unified Finder Hierarchy paths.
#[derive(Debug, Clone)]
pub struct FinderHierarchy {
    pub root: PathBuf,
    pub machines: PathBuf,
    pub volumes: PathBuf,
}

/// Result of a mount operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MountMethod {
    Nfs,
    Sshfs,
    Sftp,
    AlreadyMounted,
}

/// Summary of a machine mount action.
#[derive(Debug, Clone)]
pub struct MountSummary {
    pub name: String,
    pub ip: String,
    pub primary_path: PathBuf,
    pub alias_path: Option<PathBuf>,
    pub method: MountMethod,
}

/// Returns the standard root path `~/Norb`.
pub fn norb_root() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/Users/Shared"))
        .join(NORB_ROOT_DIR)
}

/// Returns the standard machines directory `~/Norb/machines`.
pub fn machines_root() -> PathBuf {
    norb_root().join(MACHINES_SUBDIR)
}

/// Returns the standard volumes directory `~/Norb/volumes`.
pub fn volumes_root() -> PathBuf {
    norb_root().join(VOLUMES_SUBDIR)
}

/// Ensures the Unified Finder Hierarchy (`~/Norb/machines` and `~/Norb/volumes`) exists.
pub fn ensure_finder_hierarchy() -> Result<FinderHierarchy> {
    ensure_finder_hierarchy_at(&norb_root())
}

/// Ensures the Unified Finder Hierarchy exists at a custom root directory.
pub fn ensure_finder_hierarchy_at(root: &Path) -> Result<FinderHierarchy> {
    let machines = root.join(MACHINES_SUBDIR);
    let volumes = root.join(VOLUMES_SUBDIR);

    fs::create_dir_all(root)
        .with_context(|| format!("failed to create Finder root directory: {}", root.display()))?;
    fs::create_dir_all(&machines)
        .with_context(|| format!("failed to create machines directory: {}", machines.display()))?;
    fs::create_dir_all(&volumes)
        .with_context(|| format!("failed to create volumes directory: {}", volumes.display()))?;

    Ok(FinderHierarchy {
        root: root.to_path_buf(),
        machines,
        volumes,
    })
}

/// Resolve the primary mount destination and optional alias symlink for a machine.
/// Supports both canonical `~/Norb/machines/<name>` and direct `~/Norb/<name>`.
pub fn resolve_mount_paths(name: &str, target: Option<&Path>) -> (PathBuf, Option<PathBuf>) {
    resolve_mount_paths_at(&norb_root(), name, target)
}

/// Resolve mount paths at a specified root directory.
pub fn resolve_mount_paths_at(root: &Path, name: &str, target: Option<&Path>) -> (PathBuf, Option<PathBuf>) {
    let machines_dir = root.join(MACHINES_SUBDIR);
    let legacy_dir = root.join(name);
    let canonical_machine_dir = machines_dir.join(name);

    if let Some(t) = target {
        let t_buf = t.to_path_buf();
        if t_buf == legacy_dir {
            // Target explicitly set to ~/Norb/<name> -> alias is machines/<name>
            (t_buf, Some(canonical_machine_dir))
        } else if t_buf == canonical_machine_dir {
            // Target explicitly set to ~/Norb/machines/<name> -> alias is ~/Norb/<name>
            (t_buf, Some(legacy_dir))
        } else {
            // Custom target path -> alias link in machines/<name>
            (t_buf, Some(canonical_machine_dir))
        }
    } else {
        // Default: canonical path is ~/Norb/machines/<name>, alias is ~/Norb/<name>
        (canonical_machine_dir, Some(legacy_dir))
    }
}

/// Ensure an alias symlink points to `primary_dir`.
pub fn setup_alias_symlink(primary_dir: &Path, alias_path: &Path) -> Result<()> {
    if let Ok(meta) = fs::symlink_metadata(alias_path) {
        if meta.file_type().is_symlink() {
            if let Ok(dest) = fs::read_link(alias_path) {
                if dest == primary_dir {
                    return Ok(());
                }
            }
            let _ = fs::remove_file(alias_path);
        } else if meta.is_dir() {
            if let Ok(mut entries) = fs::read_dir(alias_path) {
                if entries.next().is_none() {
                    let _ = fs::remove_dir(alias_path);
                } else {
                    // Do not remove non-empty directory
                    return Ok(());
                }
            }
        }
    }

    if !alias_path.exists() {
        #[cfg(unix)]
        {
            let _ = std::os::unix::fs::symlink(primary_dir, alias_path);
        }
    }
    Ok(())
}

/// Check if a directory is currently an active mount point on macOS.
pub fn is_mounted(mount_dir: &Path) -> bool {
    let target = mount_dir.canonicalize().unwrap_or_else(|_| mount_dir.to_path_buf());
    let target_str = target.to_string_lossy();
    let original_str = mount_dir.to_string_lossy();

    let output = match Command::new("/sbin/mount").output() {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).to_string(),
        _ => return false,
    };

    for line in output.lines() {
        if let Some((_, rest)) = line.split_once(" on ") {
            if let Some((mp, _)) = rest.split_once(" (") {
                let mp_path = Path::new(mp);
                let mp_canonical = mp_path.canonicalize().unwrap_or_else(|_| mp_path.to_path_buf());
                if mp == original_str || mp == target_str || mp_canonical == target {
                    return true;
                }
            }
        }
    }
    false
}

/// Mount a remote NFS export using native macOS `/sbin/mount_nfs -o nolocks,resvport,soft,intr`.
pub fn mount_nfs(ip: &str, export: &str, mount_dir: &Path) -> Result<()> {
    let export_spec = format!("{ip}:{export}");
    let mount_str = mount_dir
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("invalid mount directory path"))?;

    let bin = if Path::new("/sbin/mount_nfs").exists() {
        "/sbin/mount_nfs"
    } else {
        "mount_nfs"
    };

    let output = Command::new(bin)
        .args([
            "-o",
            "nolocks,resvport,soft,intr",
            &export_spec,
            mount_str,
        ])
        .output()
        .with_context(|| format!("failed to execute {bin}"))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        bail!("mount_nfs failed (exit code {:?}): {}", output.status.code(), stderr);
    }
}

/// Mount a remote filesystem via sshfs.
pub fn mount_sshfs(ip: &str, remote_path: &str, mount_dir: &Path) -> Result<()> {
    let which_sshfs = Command::new("which").arg("sshfs").output();
    let has_sshfs = which_sshfs.map(|o| o.status.success()).unwrap_or(false);
    if !has_sshfs {
        bail!("sshfs is not installed on host");
    }

    let remote_spec = format!("root@{ip}:{remote_path}");
    let mount_str = mount_dir
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("invalid mount directory path"))?;

    let mut args = vec![
        remote_spec,
        mount_str.to_string(),
        "-o".to_string(),
        "StrictHostKeyChecking=no".to_string(),
        "-o".to_string(),
        "UserKnownHostsFile=/dev/null".to_string(),
        "-o".to_string(),
        "reconnect".to_string(),
        "-o".to_string(),
        "follow_symlinks".to_string(),
    ];

    if let Ok(key) = crate::ssh::ensure_norb_ssh_key() {
        args.push("-o".to_string());
        args.push(format!("IdentityFile={}", key.display()));
    }

    let output = Command::new("sshfs")
        .args(&args)
        .output()
        .with_context(|| "failed to execute sshfs")?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        bail!("sshfs failed: {}", stderr);
    }
}

/// Open native macOS Network Browser connection via SFTP (`open sftp://root@<ip>/`).
pub fn open_sftp(ip: &str) -> Result<()> {
    let sftp_url = format!("sftp://root@{ip}/");
    let status = Command::new("open")
        .arg(&sftp_url)
        .status()
        .with_context(|| "failed to run 'open' for SFTP URL")?;

    if status.success() {
        Ok(())
    } else {
        bail!("'open {}' exited with non-zero status", sftp_url);
    }
}

/// Open a path in macOS Finder (`open <path>`).
pub fn open_in_finder(path: &Path) -> Result<()> {
    let status = Command::new("open")
        .arg(path)
        .status()
        .with_context(|| format!("failed to run 'open' for {}", path.display()))?;

    if status.success() {
        Ok(())
    } else {
        bail!("'open {}' exited with non-zero status", path.display());
    }
}

/// Unmount a mounted directory using `/sbin/umount` with fallback to `diskutil unmount`.
pub fn unmount(mount_dir: &Path, force: bool) -> Result<()> {
    if !mount_dir.exists() {
        return Ok(());
    }

    let bin = if Path::new("/sbin/umount").exists() {
        "/sbin/umount"
    } else {
        "umount"
    };

    let mut cmd = Command::new(bin);
    if force {
        cmd.arg("-f");
    }
    cmd.arg(mount_dir);

    let output = cmd.output().with_context(|| format!("failed to execute {bin}"))?;
    if output.status.success() {
        return Ok(());
    }

    // Fallback to diskutil unmount
    let mut dcmd = Command::new("diskutil");
    dcmd.arg("unmount");
    if force {
        dcmd.arg("force");
    }
    dcmd.arg(mount_dir);

    let d_out = dcmd.output().with_context(|| "failed to execute diskutil unmount")?;
    if d_out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        bail!("failed to unmount {}: {}", mount_dir.display(), err);
    }
}

/// Mount a running machine into the Unified Finder Hierarchy.
/// Tries native loopback NFS (`/sbin/mount_nfs -o nolocks,resvport,soft,intr`),
/// falls back to sshfs if present, and falls back to native Finder SFTP (`open sftp://root@<ip>/`).
pub async fn mount_machine(
    name: &str,
    ip: &str,
    target: Option<&Path>,
) -> Result<MountSummary> {
    let hierarchy = ensure_finder_hierarchy()?;
    let (primary_dir, alias_dir) = resolve_mount_paths_at(&hierarchy.root, name, target);

    fs::create_dir_all(&primary_dir)
        .with_context(|| format!("failed to create directory {}", primary_dir.display()))?;

    // Check if already mounted
    if is_mounted(&primary_dir) {
        println!("  \x1b[1;32m✓\x1b[0m Filesystem for \x1b[1m{}\x1b[0m is already mounted at {}", name, primary_dir.display());
        if let Some(ref alias) = alias_dir {
            let _ = setup_alias_symlink(&primary_dir, alias);
        }
        let _ = open_in_finder(&primary_dir);
        return Ok(MountSummary {
            name: name.to_string(),
            ip: ip.to_string(),
            primary_path: primary_dir,
            alias_path: alias_dir,
            method: MountMethod::AlreadyMounted,
        });
    }

    // 1. Try native macOS NFS loopback mount
    println!("  \x1b[1;36m➜\x1b[0m Attempting native NFS loopback mount for \x1b[1m{}\x1b[0m at {}...", name, primary_dir.display());
    match mount_nfs(ip, "/", &primary_dir) {
        Ok(()) => {
            println!("\x1b[1;32m✓\x1b[0m Successfully mounted \x1b[1m{}\x1b[0m via native NFS at {}", name, primary_dir.display());
            if let Some(ref alias) = alias_dir {
                let _ = setup_alias_symlink(&primary_dir, alias);
            }
            let _ = open_in_finder(&primary_dir);
            return Ok(MountSummary {
                name: name.to_string(),
                ip: ip.to_string(),
                primary_path: primary_dir,
                alias_path: alias_dir,
                method: MountMethod::Nfs,
            });
        }
        Err(e) => {
            tracing::debug!("native mount_nfs failed: {}", e);
        }
    }

    // 2. Try sshfs if installed
    let which_sshfs = Command::new("which").arg("sshfs").output();
    let has_sshfs = which_sshfs.map(|o| o.status.success()).unwrap_or(false);
    if has_sshfs {
        println!("  \x1b[1;36m➜\x1b[0m Mounting \x1b[1m{}\x1b[0m filesystem at {} via sshfs...", name, primary_dir.display());
        if let Ok(()) = mount_sshfs(ip, "/", &primary_dir) {
            println!("\x1b[1;32m✓\x1b[0m Successfully mounted \x1b[1m{}\x1b[0m via sshfs to {}", name, primary_dir.display());
            if let Some(ref alias) = alias_dir {
                let _ = setup_alias_symlink(&primary_dir, alias);
            }
            let _ = open_in_finder(&primary_dir);
            return Ok(MountSummary {
                name: name.to_string(),
                ip: ip.to_string(),
                primary_path: primary_dir,
                alias_path: alias_dir,
                method: MountMethod::Sshfs,
            });
        }
    }

    // 3. Fallback to native Finder SFTP / Network browser
    println!("  \x1b[1;36m➜\x1b[0m Opening \x1b[1m{}\x1b[0m ({}) in macOS Network Browser (SFTP)...", name, ip);
    let sftp_url = format!("sftp://root@{ip}/");
    let open_ok = open_sftp(ip).is_ok();

    if open_ok {
        println!("\x1b[1;32m✓\x1b[0m Opened network share in macOS Finder!");
    } else {
        println!("  To browse guest files from Finder, connect via SFTP:");
        println!("  \x1b[1;34m{}\x1b[0m", sftp_url);
    }

    if let Some(ref alias) = alias_dir {
        let _ = setup_alias_symlink(&primary_dir, alias);
    }

    Ok(MountSummary {
        name: name.to_string(),
        ip: ip.to_string(),
        primary_path: primary_dir,
        alias_path: alias_dir,
        method: MountMethod::Sftp,
    })
}

/// Mount all currently running machines into the Unified Finder Hierarchy.
pub async fn mount_all() -> Result<()> {
    let hierarchy = ensure_finder_hierarchy()?;
    let cfg = NorbConfig::load()?;
    let machines = crate::commands::load_all_machines(&cfg.machines_dir)?;

    let running: Vec<_> = machines
        .into_iter()
        .filter(|m| m.state == MachineState::Running && m.ip_address.is_some())
        .collect();

    if running.is_empty() {
        println!("  No running machines to mount.");
        return Ok(());
    }

    println!("  \x1b[1;36m➜\x1b[0m Ensuring mounts in Finder hierarchy at {}...", hierarchy.root.display());
    for m in running {
        if let Some(ref ip) = m.ip_address {
            let _ = mount_machine(&m.config.name, &ip.to_string(), None).await;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_finder_hierarchy_creation() {
        let temp_dir = std::env::temp_dir().join(format!("norb-test-hierarchy-{}", uuid::Uuid::new_v4()));
        let hierarchy = ensure_finder_hierarchy_at(&temp_dir).unwrap();

        assert_eq!(hierarchy.root, temp_dir);
        assert_eq!(hierarchy.machines, temp_dir.join("machines"));
        assert_eq!(hierarchy.volumes, temp_dir.join("volumes"));

        assert!(hierarchy.root.is_dir());
        assert!(hierarchy.machines.is_dir());
        assert!(hierarchy.volumes.is_dir());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_resolve_mount_paths_default() {
        let root = Path::new("/mock/home/Norb");
        let (primary, alias) = resolve_mount_paths_at(root, "ubuntu", None);

        assert_eq!(primary, PathBuf::from("/mock/home/Norb/machines/ubuntu"));
        assert_eq!(alias, Some(PathBuf::from("/mock/home/Norb/ubuntu")));
    }

    #[test]
    fn test_resolve_mount_paths_explicit_legacy() {
        let root = Path::new("/mock/home/Norb");
        let target = Path::new("/mock/home/Norb/ubuntu");
        let (primary, alias) = resolve_mount_paths_at(root, "ubuntu", Some(target));

        assert_eq!(primary, PathBuf::from("/mock/home/Norb/ubuntu"));
        assert_eq!(alias, Some(PathBuf::from("/mock/home/Norb/machines/ubuntu")));
    }

    #[test]
    fn test_setup_alias_symlink() {
        let temp_dir = std::env::temp_dir().join(format!("norb-test-symlink-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&temp_dir).unwrap();
        let target_dir = temp_dir.join("target");
        fs::create_dir_all(&target_dir).unwrap();
        let link_path = temp_dir.join("link");

        setup_alias_symlink(&target_dir, &link_path).unwrap();
        assert!(link_path.exists());
        assert_eq!(fs::read_link(&link_path).unwrap(), target_dir);

        // Idempotency: calling again with same parameters succeeds
        setup_alias_symlink(&target_dir, &link_path).unwrap();
        assert_eq!(fs::read_link(&link_path).unwrap(), target_dir);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_is_mounted_root() {
        // Root filesystem "/" is always mounted on macOS
        assert!(is_mounted(Path::new("/")));
        // Nonexistent test path should not be mounted
        assert!(!is_mounted(Path::new("/tmp/norb_nonexistent_mount_test_12345")));
    }
}
