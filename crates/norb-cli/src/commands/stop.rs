use anyhow::{Context, Result};
use norb_common::config::NorbConfig;
use norb_common::machine::MachineState;

use super::{find_machine, load_all_machines, save_machine};

fn terminate_docker_ssh_tunnel(ip_opt: Option<&str>) {
    if let Ok(output) = std::process::Command::new("ps").args(["-eo", "pid,command"]).output() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.contains("ssh") && line.contains("docker.sock:/var/run/docker.sock") {
                let matches_ip = match ip_opt {
                    Some(ip) => line.contains(ip),
                    None => true,
                };
                if matches_ip {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if let Some(pid_str) = parts.first() {
                        if let Ok(pid) = pid_str.parse::<i32>() {
                            let _ = nix::sys::signal::kill(
                                nix::unistd::Pid::from_raw(pid),
                                nix::sys::signal::Signal::SIGTERM,
                            );
                        }
                    }
                }
            }
        }
    }
}

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
                    terminate_docker_ssh_tunnel(Some(&ip.to_string()));
                    let mut cmd = std::process::Command::new("ssh");
                    cmd.args(crate::ssh::norb_ssh_args());
                    cmd.args([
                        "-o", "ConnectTimeout=1",
                        &format!("root@{ip}"),
                        "sync",
                    ]);
                    let _ = cmd.output();
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

            terminate_docker_ssh_tunnel(None);
            let _ = std::fs::remove_file(cfg.data_dir.join("run").join("docker.sock"));
            return Ok(());
        }
    };

    let mut machine = find_machine(&cfg.machines_dir, &target_name)?
        .with_context(|| format!("machine '{target_name}' not found"))?;

    println!("\x1b[1;36m➜\x1b[0m Stopping machine \x1b[1m{}\x1b[0m...", machine.config.name);

    if let Some(ip) = machine.ip_address {
        terminate_docker_ssh_tunnel(Some(&ip.to_string()));
        let mut cmd = std::process::Command::new("ssh");
        cmd.args(crate::ssh::norb_ssh_args());
        cmd.args([
            "-o", "ConnectTimeout=1",
            &format!("root@{ip}"),
            "sync",
        ]);
        let _ = cmd.output();
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
        terminate_docker_ssh_tunnel(None);
        let _ = std::fs::remove_file(cfg.data_dir.join("run").join("docker.sock"));
    }

    println!(
        "\x1b[1;32m✓\x1b[0m Machine \x1b[1m{}\x1b[0m stopped successfully.",
        machine.config.name
    );

    Ok(())
}
