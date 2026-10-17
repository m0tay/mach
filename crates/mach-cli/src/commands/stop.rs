use anyhow::{Context, Result};
use mach_common::config::MachConfig;
use mach_common::machine::MachineState;

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

pub fn stop_single_machine(
    machine: &mut mach_common::machine::Machine,
    cfg: &MachConfig,
    force: bool,
) -> Result<()> {
    println!("\x1b[1;36m➜\x1b[0m Stopping machine \x1b[1m{}\x1b[0m...", machine.config.name);

    let pid_file = cfg.machines_dir.join(&machine.config.name).join("pid");
    let target_pid = machine.pid
        .or_else(|| {
            std::fs::read_to_string(&pid_file)
                .ok()
                .and_then(|s| s.trim().parse::<u32>().ok())
        })
        .or_else(|| super::find_running_vm_pid(&machine.config.name));

    if let Some(ip) = machine.ip_address {
        terminate_docker_ssh_tunnel(Some(&ip.to_string()));

        if !force {
            let mut cmd = std::process::Command::new("ssh");
            cmd.args(crate::ssh::mach_ssh_args());
            cmd.args([
                "-o", "ConnectTimeout=2",
                &format!("root@{ip}"),
                "sync; (systemctl poweroff || poweroff || halt) >/dev/null 2>&1 &",
            ]);
            let _ = cmd.output();
        }
    }

    if let Some(pid) = target_pid {
        let raw_pid = nix::unistd::Pid::from_raw(pid as i32);
        if force {
            let _ = nix::sys::signal::kill(raw_pid, nix::sys::signal::Signal::SIGKILL);
        } else {
            // Wait up to 3.5s for graceful guest shutdown to stop the VMM
            let mut exited = false;
            for _ in 0..35 {
                if nix::sys::signal::kill(raw_pid, None).is_err() {
                    exited = true;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }

            if !exited {
                let _ = nix::sys::signal::kill(raw_pid, nix::sys::signal::Signal::SIGTERM);
                for _ in 0..15 {
                    if nix::sys::signal::kill(raw_pid, None).is_err() {
                        exited = true;
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                if !exited {
                    let _ = nix::sys::signal::kill(raw_pid, nix::sys::signal::Signal::SIGKILL);
                }
            }
        }
        let _ = std::fs::remove_file(&pid_file);
    }

    // Clean up any stray child process for this machine
    super::kill_running_vm_processes(Some(&machine.config.name));

    machine.state = MachineState::Stopped;
    machine.ip_address = None;
    machine.pid = None;
    save_machine(&cfg.machines_dir, machine)?;

    println!(
        "\x1b[1;32m✓\x1b[0m Machine \x1b[1m{}\x1b[0m stopped successfully.",
        machine.config.name
    );

    Ok(())
}

pub async fn run(name: Option<&str>, force: bool, system: bool) -> Result<()> {
    let cfg = MachConfig::load()?;

    let target_name = match name {
        Some(n) => n.to_string(),
        None => {
            let machines = load_all_machines(&cfg.machines_dir)?;
            let mut running = Vec::new();
            for m in machines {
                let is_running_state = m.state == MachineState::Running;
                let has_proc = super::find_running_vm_pid(&m.config.name).is_some();
                if (is_running_state || has_proc) && (system || !m.is_system()) {
                    running.push(m);
                }
            }

            if running.is_empty() {
                if system {
                    super::kill_running_vm_processes(None);
                }
                println!("No user machines currently running.");
                return Ok(());
            }

            for mut m in running {
                stop_single_machine(&mut m, &cfg, force)?;
            }

            if system {
                terminate_docker_ssh_tunnel(None);
                let _ = std::fs::remove_file(cfg.data_dir.join("run").join("docker.sock"));
            }

            let remaining = load_all_machines(&cfg.machines_dir)?
                .into_iter()
                .filter(|m| m.state == MachineState::Running)
                .count();
            if remaining == 0 {
                crate::commands::menu_cmd::stop_menu_bar();
            }

            return Ok(());
        }
    };

    let mut machine = find_machine(&cfg.machines_dir, &target_name)?
        .with_context(|| format!("machine '{target_name}' not found"))?;

    if machine.is_system() && !system {
        let svc = machine.system_service_name().unwrap_or("system");
        anyhow::bail!(
            "'{}' is a protected system infrastructure machine powering {svc}.\n\
            To stop this service, run:\n  machctl {} stop\n\
            Or pass `--system`:\n  machctl stop {} --system",
            machine.config.name,
            svc.to_lowercase(),
            machine.config.name
        );
    }

    stop_single_machine(&mut machine, &cfg, force)?;

    let running_count = load_all_machines(&cfg.machines_dir)?
        .into_iter()
        .filter(|m| m.state == MachineState::Running)
        .count();
    if running_count == 0 {
        terminate_docker_ssh_tunnel(None);
        let _ = std::fs::remove_file(cfg.data_dir.join("run").join("docker.sock"));
        crate::commands::menu_cmd::stop_menu_bar();
    }

    Ok(())
}
