use anyhow::{Context, Result};
use norb_common::config::NorbConfig;
use norb_common::machine::MachineState;

use super::{find_machine, load_all_machines, save_machine};

pub async fn run(name: Option<&str>, _force: bool) -> Result<()> {
    let cfg = NorbConfig::load()?;

    let target_name = match name {
        Some(n) => n.to_string(),
        None => {
            let running: Vec<_> = load_all_machines(&cfg.machines_dir)?
                .into_iter()
                .filter(|m| m.state == MachineState::Running)
                .collect();

            if running.is_empty() {
                println!("No machines currently running.");
                return Ok(());
            }

            for mut m in running {
                println!("\x1b[1;36m➜\x1b[0m Stopping machine \x1b[1m{}\x1b[0m...", m.config.name);
                if let Some(ip) = m.ip_address {
                    let _ = std::process::Command::new("ssh")
                        .args([
                            "-o", "StrictHostKeyChecking=no",
                            "-o", "UserKnownHostsFile=/dev/null",
                            "-o", "LogLevel=ERROR",
                            "-o", "ConnectTimeout=1",
                            &format!("root@{ip}"),
                            "sync",
                        ])
                        .output();
                }
                if let Some(pid) = m.pid {
                    let _ = nix::sys::signal::kill(
                        nix::unistd::Pid::from_raw(pid as i32),
                        nix::sys::signal::Signal::SIGTERM,
                    );
                }
                m.state = MachineState::Stopped;
                m.ip_address = None;
                m.pid = None;
                save_machine(&cfg.machines_dir, &m)?;
                println!("\x1b[1;32m✓\x1b[0m Machine \x1b[1m{}\x1b[0m stopped.", m.config.name);
            }
            return Ok(());
        }
    };

    let mut machine = find_machine(&cfg.machines_dir, &target_name)?
        .with_context(|| format!("machine '{target_name}' not found"))?;

    println!("\x1b[1;36m➜\x1b[0m Stopping machine \x1b[1m{}\x1b[0m...", machine.config.name);

    if let Some(ip) = machine.ip_address {
        let _ = std::process::Command::new("ssh")
            .args([
                "-o", "StrictHostKeyChecking=no",
                "-o", "UserKnownHostsFile=/dev/null",
                "-o", "LogLevel=ERROR",
                "-o", "ConnectTimeout=1",
                &format!("root@{ip}"),
                "sync",
            ])
            .output();
    }

    let pid_file = cfg.machines_dir.join(&machine.config.name).join("pid");
    let target_pid = machine.pid.or_else(|| {
        std::fs::read_to_string(&pid_file)
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
    });

    if let Some(pid) = target_pid {
        let sig = if _force {
            nix::sys::signal::Signal::SIGKILL
        } else {
            nix::sys::signal::Signal::SIGTERM
        };
        let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), sig);
        let _ = std::fs::remove_file(&pid_file);
    }

    machine.state = MachineState::Stopped;
    machine.ip_address = None;
    machine.pid = None;

    save_machine(&cfg.machines_dir, &machine)?;

    let running_count = load_all_machines(&cfg.machines_dir)?
        .into_iter()
        .filter(|m| m.state == MachineState::Running)
        .count();
    if running_count == 0 {
        let _ = std::fs::remove_file(cfg.data_dir.join("run").join("docker.sock"));
    }

    println!(
        "\x1b[1;32m✓\x1b[0m Machine \x1b[1m{}\x1b[0m stopped successfully.",
        machine.config.name
    );

    Ok(())
}
