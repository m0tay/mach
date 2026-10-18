//! Command implementations for `mach`.

pub mod boot;
pub mod clone_cmd;
pub mod completion;
pub mod config;
pub mod create;
pub mod daemon;
pub mod debug;
pub mod default_cmd;
pub mod delete;
pub mod dns_cmd;
pub mod docker;
pub mod doctor;
pub mod dump;
pub mod export_import;
pub mod info;
pub mod init;
pub mod inspect;
pub mod k8s;
pub mod link;
pub mod list;
pub mod logs;
pub mod menu_cmd;
pub mod mount;
pub mod push_pull;
pub mod rename_cmd;
pub mod restart;
pub mod run_cmd;
pub mod run_vm;
pub mod setup;
pub mod shell;
pub mod ssh;
pub mod start;
pub mod status;
pub mod stop;
pub mod tls_cmd;
pub mod top;
pub mod update;
pub mod uninstall;
pub mod version;

use anyhow::{bail, Context, Result};
use mach_common::config::MachConfig;
use mach_common::machine::{Machine, MachineState};
use std::fs;
use std::path::{Path, PathBuf};

/// Helper to get the path to a machine's metadata file.
#[allow(dead_code)]
pub fn machine_file_path(machines_dir: &Path, name: &str) -> PathBuf {
    let yaml = machines_dir.join(name).join("machine.yaml");
    if yaml.exists() {
        yaml
    } else {
        machines_dir.join(name).join("machine.json")
    }
}

/// Locate a binary on macOS looking in PATH, Homebrew, /usr/local, and common user tool locations.
pub fn find_binary(name: &str) -> Option<PathBuf> {
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let p = dir.join(name);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    let mut candidates = vec![
        PathBuf::from(format!("/opt/homebrew/bin/{name}")),
        PathBuf::from(format!("/usr/local/bin/{name}")),
        PathBuf::from(format!("/usr/bin/{name}")),
    ];
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(format!(".rd/bin/{name}")));
        candidates.push(home.join(format!(".orbstack/bin/{name}")));
        candidates.push(home.join(format!(".colima/bin/{name}")));
        candidates.push(home.join(format!(".docker/bin/{name}")));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Load all machines from the machines directory (supports both YAML and JSON with auto-migration).
pub fn load_all_machines(machines_dir: &Path) -> Result<Vec<Machine>> {
    if !machines_dir.exists() {
        return Ok(Vec::new());
    }

    let mut machines = Vec::new();
    let entries = fs::read_dir(machines_dir)
        .with_context(|| format!("failed to read machines directory: {}", machines_dir.display()))?;

    for entry in entries.flatten() {
        if entry.path().is_dir() {
            if let Ok(m) = mach_common::machine::load_machine(&entry.path()) {
                machines.push(m);
            }
        }
    }

    machines.sort_by(|a, b| a.config.name.cmp(&b.config.name));
    Ok(machines)
}

/// Find a machine by name or partial UUID prefix.
pub fn find_machine(machines_dir: &Path, query: &str) -> Result<Option<Machine>> {
    let all = load_all_machines(machines_dir)?;
    for m in all {
        if m.config.name.eq_ignore_ascii_case(query)
            || m.short_id().eq_ignore_ascii_case(query)
            || m.id.to_string().starts_with(query)
        {
            return Ok(Some(m));
        }
    }
    Ok(None)
}

/// Persist a machine descriptor to disk in YAML format.
pub fn save_machine(machines_dir: &Path, machine: &Machine) -> Result<()> {
    mach_common::machine::save_machine(machines_dir, machine)
        .map_err(|e| anyhow::anyhow!("{e}"))
}
/// Returns the visible character width of a string in terminal cells, ignoring ANSI escape sequences.
pub fn visible_width(s: &str) -> usize {
    let mut width = 0;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                while let Some(esc_c) = chars.next() {
                    if ('@'..='~').contains(&esc_c) {
                        break;
                    }
                }
            } else if chars.peek() == Some(&']') {
                chars.next(); // consume ']'
                while let Some(esc_c) = chars.next() {
                    if esc_c == '\x07' || esc_c == '\x1b' {
                        break;
                    }
                }
            }
        } else {
            width += 1;
        }
    }
    width
}

/// Pads a string with trailing spaces so that its visible width in terminal cells equals `target_width`.
pub fn pad_to_width(s: &str, target_width: usize) -> String {
    let v_width = visible_width(s);
    let pad = target_width.saturating_sub(v_width);
    format!("{}{}", s, " ".repeat(pad))
}

/// Resolve the target machine using standard fallback priority:
/// 1. Explicit name (`name_opt`) -> if specified, must exist or error.
/// 2. `cfg.default_machine` -> if configured and exists on disk.
/// 3. Single running user machine -> if exactly one user machine is currently running.
/// 4. Single existing user machine -> if exactly one user machine exists overall.
/// 5. Error -> if none or ambiguous.
pub fn resolve_target_machine(cfg: &MachConfig, name_opt: Option<&str>) -> Result<Machine> {
    resolve_target_machine_ext(cfg, name_opt, false)
}

pub fn resolve_target_machine_ext(cfg: &MachConfig, name_opt: Option<&str>, allow_system: bool) -> Result<Machine> {
    if let Some(name) = name_opt {
        let m = find_machine(&cfg.machines_dir, name)?
            .with_context(|| format!("machine '{name}' not found"))?;

        if m.is_system() && !allow_system {
            let svc = m.system_service_name().unwrap_or("system");
            bail!(
                "'{}' is a protected system infrastructure machine powering {svc}.\n\
                To interact with this service, use `machctl {} shell` or pass `--system`.",
                m.config.name,
                svc.to_lowercase()
            );
        }

        return Ok(m);
    }

    if let Some(ref def) = cfg.default_machine {
        if let Some(m) = find_machine(&cfg.machines_dir, def)? {
            if allow_system || !m.is_system() {
                return Ok(m);
            }
        }
    }

    let machines = load_all_machines(&cfg.machines_dir)?;
    let user_machines: Vec<_> = machines.into_iter().filter(|m| allow_system || !m.is_system()).collect();

    if user_machines.is_empty() {
        bail!("no user machines found. Run `mach create <distro>` first, or use `docker` / `kubectl`.");
    }

    let running: Vec<_> = user_machines
        .iter()
        .filter(|m| m.state == MachineState::Running)
        .collect();

    if running.len() == 1 {
        return Ok(running[0].clone());
    }

    if user_machines.len() == 1 {
        return Ok(user_machines[0].clone());
    }

    if running.len() > 1 {
        bail!("multiple running machines found; please specify a machine name or set a default with `mach default <name>`");
    }

    bail!("multiple machines found; please specify a machine name or set a default with `mach default <name>`");
}

/// Find any active hypervisor process running for a machine by scanning the macOS process table.
pub fn find_running_vm_pid(machine_name: &str) -> Option<u32> {
    if let Ok(output) = std::process::Command::new("ps").args(["-eo", "pid,command"]).output() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let is_mach_vm = line.contains("run-vm") && (line.contains("mach") || line.contains("machctl"));
            if is_mach_vm && (line.contains(&format!("--name {}", machine_name)) || line.contains(&format!("--name={}", machine_name))) {
                if let Some(pid_str) = line.split_whitespace().next() {
                    if let Ok(pid) = pid_str.parse::<u32>() {
                        return Some(pid);
                    }
                }
            }
        }
    }
    None
}

/// Kill any running hypervisor processes for a machine (or all mach VMs if machine_name_opt is None).
pub fn kill_running_vm_processes(machine_name_opt: Option<&str>) {
    if let Ok(output) = std::process::Command::new("ps").args(["-eo", "pid,command"]).output() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let is_mach_vm = line.contains("run-vm") && (line.contains("mach") || line.contains("machctl"));
            if is_mach_vm {
                let matches_target = match machine_name_opt {
                    Some(name) => line.contains(&format!("--name {}", name)) || line.contains(&format!("--name={}", name)),
                    None => true,
                };
                if matches_target {
                    if let Some(pid_str) = line.split_whitespace().next() {
                        if let Ok(pid) = pid_str.parse::<i32>() {
                            let _ = nix::sys::signal::kill(
                                nix::unistd::Pid::from_raw(pid),
                                nix::sys::signal::Signal::SIGKILL,
                            );
                        }
                    }
                }
            }
        }
    }
}

/// Restore Docker and Kubernetes in the background if they were running in the previous session.
pub fn restore_services_background(cfg: &MachConfig) {
    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("mach"));

    // 1. Restore Docker engine if active in previous session
    if cfg.docker.was_running {
        let is_running = find_machine(&cfg.machines_dir, docker::DOCKER_VM_NAME)
            .ok()
            .flatten()
            .map(|m| {
                (m.state == MachineState::Running || find_running_vm_pid(docker::DOCKER_VM_NAME).is_some())
                    && m.pid.map(|p| nix::sys::signal::kill(nix::unistd::Pid::from_raw(p as i32), None).is_ok()).unwrap_or(false)
            })
            .unwrap_or(false);

        if !is_running {
            let _ = std::process::Command::new(&exe)
                .args(["docker", "start"])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
        }
    }

    // 2. Restore Kubernetes cluster if active in previous session
    if cfg.k8s.was_running {
        let vm_name = k8s::resolve_k8s_machine_name(&cfg.machines_dir).unwrap_or_else(|_| k8s::K8S_VM_NAME.to_string());
        let is_running = find_machine(&cfg.machines_dir, &vm_name)
            .ok()
            .flatten()
            .map(|m| {
                (m.state == MachineState::Running || find_running_vm_pid(&vm_name).is_some())
                    && m.pid.map(|p| nix::sys::signal::kill(nix::unistd::Pid::from_raw(p as i32), None).is_ok()).unwrap_or(false)
            })
            .unwrap_or(false);

        if !is_running {
            let _ = std::process::Command::new(&exe)
                .args(["k8s", "start"])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
        }
    }
}

/// Check if Docker or Kubernetes were running previously and restore them on startup.
pub async fn restore_services_if_needed(cfg: &MachConfig) -> Result<()> {
    if cfg.docker.was_running || cfg.k8s.was_running {
        let cfg_clone = cfg.clone();
        std::thread::spawn(move || {
            restore_services_background(&cfg_clone);
        });
    }
    Ok(())
}
