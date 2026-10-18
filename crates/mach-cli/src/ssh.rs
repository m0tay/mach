//! SSH identity management for mach machines.

use anyhow::{bail, Context, Result};
use std::fs;
use std::net::IpAddr;
use std::path::PathBuf;
use std::process::Command;

/// Ensure the Mach dedicated SSH key pair exists in `~/.mach/ssh/id_ed25519`.
/// Returns the path to the private key.
pub fn ensure_mach_ssh_key() -> Result<PathBuf> {
    let data_dir = mach_common::config::MachConfig::default().data_dir;
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
                "-C", "mach@macos",
            ])
            .output()
            .with_context(|| "failed to run ssh-keygen to generate Mach SSH key")?;

        if !output.status.success() {
            bail!("ssh-keygen failed: {}", String::from_utf8_lossy(&output.stderr));
        }
    }

    Ok(priv_key)
}

/// Returns standard SSH arguments for non-interactive Mach commands.
pub fn mach_ssh_args() -> Vec<String> {
    let mut args = vec![
        "-o".to_string(), "StrictHostKeyChecking=no".to_string(),
        "-o".to_string(), "UserKnownHostsFile=/dev/null".to_string(),
        "-o".to_string(), "LogLevel=ERROR".to_string(),
    ];
    if let Ok(key) = ensure_mach_ssh_key() {
        args.push("-i".to_string());
        args.push(key.to_string_lossy().to_string());
    }
    if std::env::var("SSH_AUTH_SOCK").is_ok() {
        args.push("-A".to_string());
    }
    args
}

/// Create a pre-configured SSH Command targeting `user@ip`.
/// Configures `-o StrictHostKeyChecking=no`, `-o UserKnownHostsFile=/dev/null`, `-o LogLevel=ERROR`,
/// includes Mach dedicated SSH key, and targets `user@ip`.
pub fn new_ssh_command(ip: &IpAddr, user: &str) -> Command {
    let mut cmd = Command::new("ssh");
    cmd.args(mach_ssh_args());
    cmd.arg(format!("{user}@{ip}"));
    cmd
}

/// Mach managed SSH configuration block.
pub const MACH_SSH_CONFIG_BLOCK: &str = "\
# BEGIN MACH MANAGED BLOCK
Host *.mach.local *.mach mach
    User root
    IdentityFile ~/.mach/ssh/id_ed25519
    StrictHostKeyChecking no
    UserKnownHostsFile /dev/null
    ForwardAgent yes
# END MACH MANAGED BLOCK";

/// Idempotently update or insert the Mach managed block within an SSH config string.
pub fn update_ssh_config_content(content: &str) -> String {
    const BEGIN_MARKER: &str = "# BEGIN MACH MANAGED BLOCK";
    const END_MARKER: &str = "# END MACH MANAGED BLOCK";

    if let (Some(start), Some(end)) = (content.find(BEGIN_MARKER), content.find(END_MARKER)) {
        if start <= end {
            let block_end = end + END_MARKER.len();
            let mut result = String::with_capacity(content.len() + MACH_SSH_CONFIG_BLOCK.len());
            result.push_str(&content[..start]);
            result.push_str(MACH_SSH_CONFIG_BLOCK);
            result.push_str(&content[block_end..]);
            if !result.ends_with('\n') {
                result.push('\n');
            }
            return result;
        }
    }

    if content.trim().is_empty() {
        return format!("{MACH_SSH_CONFIG_BLOCK}\n");
    }

    let mut result = content.to_string();
    if !result.ends_with('\n') {
        result.push('\n');
    }
    if !result.ends_with("\n\n") {
        result.push('\n');
    }
    result.push_str(MACH_SSH_CONFIG_BLOCK);
    result.push('\n');
    result
}

/// Synchronize the Mach managed block in `~/.ssh/config`.
pub async fn sync_ssh_config() -> Result<()> {
    let _ = ensure_mach_ssh_key();

    let home = dirs::home_dir().context("could not determine user home directory")?;
    let ssh_dir = home.join(".ssh");
    sync_ssh_config_to_dir(&ssh_dir)
}

/// Synchronize the mach managed block in the given SSH directory (typically `~/.ssh`).
pub fn sync_ssh_config_to_dir(ssh_dir: &std::path::Path) -> Result<()> {
    if !ssh_dir.exists() {
        fs::create_dir_all(ssh_dir).context("failed to create ssh directory")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(ssh_dir, fs::Permissions::from_mode(0o700));
        }
    }

    let config_path = ssh_dir.join("config");
    let (content, exists) = if config_path.exists() {
        let text = fs::read_to_string(&config_path)
            .with_context(|| format!("failed to read {}", config_path.display()))?;
        (text, true)
    } else {
        (String::new(), false)
    };

    let updated = update_ssh_config_content(&content);

    if !exists {
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            use std::os::unix::fs::PermissionsExt;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&config_path)
                .with_context(|| format!("failed to create {}", config_path.display()))?;
            file.write_all(updated.as_bytes())?;
            let _ = fs::set_permissions(&config_path, fs::Permissions::from_mode(0o600));
        }
        #[cfg(not(unix))]
        {
            fs::write(&config_path, &updated)
                .with_context(|| format!("failed to write {}", config_path.display()))?;
        }
    } else if content != updated {
        fs::write(&config_path, &updated)
            .with_context(|| format!("failed to update {}", config_path.display()))?;
    }

    Ok(())
}

/// Remove any Mach managed block from `~/.ssh/config`.
pub fn remove_mach_ssh_config_from_dir(ssh_dir: &std::path::Path) -> Result<()> {
    let config_path = ssh_dir.join("config");
    if !config_path.exists() {
        return Ok(());
    }

    let text = fs::read_to_string(&config_path)
        .with_context(|| format!("failed to read {}", config_path.display()))?;

    const BEGIN_MARKER: &str = "# BEGIN MACH MANAGED BLOCK";
    const END_MARKER: &str = "# END MACH MANAGED BLOCK";
    const LEGACY_BEGIN_MARKER: &str = "# BEGIN NORB MANAGED BLOCK";
    const LEGACY_END_MARKER: &str = "# END NORB MANAGED BLOCK";

    let mut cleaned = text.clone();
    for (begin, end) in &[(BEGIN_MARKER, END_MARKER), (LEGACY_BEGIN_MARKER, LEGACY_END_MARKER)] {
        if let (Some(start), Some(finish)) = (cleaned.find(begin), cleaned.find(end)) {
            if start <= finish {
                let block_end = finish + end.len();
                let mut res = String::new();
                res.push_str(&cleaned[..start]);
                res.push_str(&cleaned[block_end..]);
                cleaned = res;
            }
        }
    }

    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        let _ = fs::remove_file(&config_path);
    } else {
        let mut final_text = trimmed.to_string();
        final_text.push('\n');
        fs::write(&config_path, final_text.as_bytes())?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_ssh_config_empty() {
        let updated = update_ssh_config_content("");
        assert!(updated.contains("# BEGIN MACH MANAGED BLOCK"));
        assert!(updated.contains("Host *.mach.local *.mach mach"));
        assert!(updated.contains("IdentityFile ~/.mach/ssh/id_ed25519"));
        assert!(updated.contains("ForwardAgent yes"));
        assert!(updated.contains("# END MACH MANAGED BLOCK"));
        assert!(updated.ends_with('\n'));

        // Idempotency check
        let second = update_ssh_config_content(&updated);
        assert_eq!(updated, second);
    }

    #[test]
    fn test_update_ssh_config_append() {
        let original = "Host github.com\n    User git\n";
        let updated = update_ssh_config_content(original);
        assert!(updated.starts_with("Host github.com\n    User git\n\n# BEGIN MACH MANAGED BLOCK"));
        assert!(updated.ends_with("# END MACH MANAGED BLOCK\n"));

        // Idempotency check
        let second = update_ssh_config_content(&updated);
        assert_eq!(updated, second);
    }

    #[test]
    fn test_update_ssh_config_replace_existing() {
        let original = "Host github.com\n    User git\n\n# BEGIN MACH MANAGED BLOCK\nHost old-config\n# END MACH MANAGED BLOCK\n\nHost work\n    User dev\n";
        let updated = update_ssh_config_content(original);
        assert!(updated.contains("Host github.com\n    User git\n"));
        assert!(updated.contains("Host *.mach.local *.mach mach"));
        assert!(updated.contains("Host work\n    User dev\n"));
        assert!(!updated.contains("Host old-config"));

        // Idempotency check
        let second = update_ssh_config_content(&updated);
        assert_eq!(updated, second);
    }

    #[test]
    fn test_mach_ssh_args_ssh_auth_sock() {
        std::env::set_var("SSH_AUTH_SOCK", "/tmp/test-ssh-agent.sock");
        let args = mach_ssh_args();
        assert!(args.contains(&"-A".to_string()));

        std::env::remove_var("SSH_AUTH_SOCK");
        let args_no_sock = mach_ssh_args();
        assert!(!args_no_sock.contains(&"-A".to_string()));
    }

    #[test]
    fn test_sync_ssh_config_to_dir_creates_0600() {
        let temp_dir = std::env::temp_dir().join(format!("mach-ssh-test-{}", uuid::Uuid::new_v4()));
        let res = sync_ssh_config_to_dir(&temp_dir);
        assert!(res.is_ok());

        let config_file = temp_dir.join("config");
        assert!(config_file.exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = fs::metadata(&config_file).unwrap();
            let mode = meta.permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }

        let content = fs::read_to_string(&config_file).unwrap();
        assert!(content.contains("# BEGIN MACH MANAGED BLOCK"));
        assert!(content.contains("# END MACH MANAGED BLOCK"));

        // Second sync should succeed and keep content identical
        let res2 = sync_ssh_config_to_dir(&temp_dir);
        assert!(res2.is_ok());
        let content2 = fs::read_to_string(&config_file).unwrap();
        assert_eq!(content, content2);

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
