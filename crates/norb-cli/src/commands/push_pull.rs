//! `norb push` and `norb pull` commands (bidirectional file copy).

use anyhow::{bail, Context, Result};
use norb_common::config::NorbConfig;
use norb_common::machine::MachineState;
use std::process::Command;

use super::{find_machine, load_all_machines};

fn resolve_ip(cfg: &NorbConfig, machine_opt: Option<&str>) -> Result<String> {
    let target_name = match machine_opt {
        Some(n) => n.to_string(),
        None => {
            if let Some(ref def) = cfg.default_machine {
                def.clone()
            } else {
                let machines = load_all_machines(&cfg.machines_dir)?;
                let running: Vec<_> = machines
                    .into_iter()
                    .filter(|m| m.state == MachineState::Running)
                    .collect();
                if running.is_empty() {
                    bail!("no running machines found. Run `norb start <machine>` first.");
                }
                running[0].config.name.clone()
            }
        }
    };

    let machine = find_machine(&cfg.machines_dir, &target_name)?
        .with_context(|| format!("machine '{target_name}' not found"))?;

    if machine.state != MachineState::Running {
        bail!("machine '{}' is not running", machine.config.name);
    }

    let ip = machine
        .ip_address
        .with_context(|| format!("machine '{}' has no IP address", machine.config.name))?;

    Ok(ip.to_string())
}

/// Safely quote an argument for POSIX shell execution to prevent command injection.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

pub async fn push(machine_opt: Option<&str>, src: &str, dest: &str) -> Result<()> {
    let cfg = NorbConfig::load()?;
    let ip = resolve_ip(&cfg, machine_opt)?;
    let src_path = std::path::Path::new(src);

    if !src_path.exists() {
        bail!("local source path '{}' does not exist", src);
    }

    println!("\x1b[1;36m➜\x1b[0m Copying \x1b[1m{}\x1b[0m to guest \x1b[1m{}\x1b[0m...", src, dest);

    let q_dest = shell_quote(dest);

    if src_path.is_file() {
        let file = std::fs::File::open(src_path)?;
        let remote_cmd = format!("mkdir -p \"$(dirname -- {q_dest})\" && cat > {q_dest}");
        let mut ssh_cmd = Command::new("ssh");
        ssh_cmd.args(crate::ssh::norb_ssh_args());
        ssh_cmd.arg(format!("root@{ip}"));
        ssh_cmd.arg(&remote_cmd);
        ssh_cmd.stdin(file);
        let status = ssh_cmd.status()?;

        if !status.success() {
            bail!("failed to copy file to guest");
        }
    } else {
        // Directory copy via tar stream
        let parent = src_path.parent().unwrap_or(std::path::Path::new("."));
        let base = src_path.file_name().unwrap();
        let tar_cmd = format!("mkdir -p -- {q_dest} && tar -xzf - -C {q_dest}");

        let mut tar_proc = Command::new("tar")
            .env("COPYFILE_DISABLE", "1")
            .args(["-czf", "-", "-C", parent.to_str().unwrap(), base.to_str().unwrap()])
            .stdout(std::process::Stdio::piped())
            .spawn()?;

        let tar_out = tar_proc.stdout.take().unwrap();

        let mut ssh_cmd = Command::new("ssh");
        ssh_cmd.args(crate::ssh::norb_ssh_args());
        ssh_cmd.arg(format!("root@{ip}"));
        ssh_cmd.arg(&tar_cmd);
        ssh_cmd.stdin(tar_out);
        let status = ssh_cmd.status()?;

        let _ = tar_proc.wait();
        if !status.success() {
            bail!("failed to copy directory to guest");
        }
    }

    println!("\x1b[1;32m✓\x1b[0m Copied successfully.");
    Ok(())
}

pub async fn pull(machine_opt: Option<&str>, src: &str, dest: &str) -> Result<()> {
    let cfg = NorbConfig::load()?;
    let ip = resolve_ip(&cfg, machine_opt)?;
    let dest_path = std::path::Path::new(dest);

    println!("\x1b[1;36m➜\x1b[0m Copying guest \x1b[1m{}\x1b[0m to \x1b[1m{}\x1b[0m...", src, dest);

    let q_src = shell_quote(src);
    let test_cmd = format!("test -d {q_src}");
    let mut check_cmd = Command::new("ssh");
    check_cmd.arg("-n");
    check_cmd.args(crate::ssh::norb_ssh_args());
    check_cmd.arg(format!("root@{ip}"));
    check_cmd.arg(&test_cmd);
    let check_is_dir = check_cmd.status()?;

    if check_is_dir.success() {
        // Remote is a directory: pull via tar
        std::fs::create_dir_all(dest_path)?;
        let remote_tar = format!("tar -czf - -C {q_src} .");
        let mut ssh_cmd = Command::new("ssh");
        ssh_cmd.arg("-n");
        ssh_cmd.args(crate::ssh::norb_ssh_args());
        ssh_cmd.arg(format!("root@{ip}"));
        ssh_cmd.arg(&remote_tar);
        ssh_cmd.stdout(std::process::Stdio::piped());
        let mut ssh_proc = ssh_cmd.spawn()?;

        let ssh_out = ssh_proc.stdout.take().unwrap();
        let status = Command::new("tar")
            .args(["-xzf", "-", "-C", dest_path.to_str().unwrap()])
            .stdin(ssh_out)
            .status()?;

        let _ = ssh_proc.wait();
        if !status.success() {
            bail!("failed to copy directory from guest");
        }
    } else {
        // Remote is a file: pull via cat
        if let Some(parent) = dest_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let out_file = std::fs::File::create(dest_path)?;
        let cat_cmd = format!("cat -- {q_src}");
        let mut ssh_cmd = Command::new("ssh");
        ssh_cmd.arg("-n");
        ssh_cmd.args(crate::ssh::norb_ssh_args());
        ssh_cmd.arg(format!("root@{ip}"));
        ssh_cmd.arg(&cat_cmd);
        ssh_cmd.stdout(out_file);
        let status = ssh_cmd.status()?;

        if !status.success() {
            bail!("failed to copy file from guest");
        }
    }

    println!("\x1b[1;32m✓\x1b[0m Copied successfully.");
    Ok(())
}
