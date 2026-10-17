//! Command implementations for `norb`.

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
pub mod export_import;
pub mod info;
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
pub mod version;

use anyhow::{bail, Context, Result};
use mach_common::config::NorbConfig;
use mach_common::machine::{Machine, MachineState};
use std::fs;
use std::path::{Path, PathBuf};

/// Helper to get the path to a machine's metadata file.
#[allow(dead_code)]
pub fn machine_file_path(machines_dir: &Path, name: &str) -> PathBuf {
    machines_dir.join(name).join("machine.json")
}

/// Load all machines from the machines directory.
pub fn load_all_machines(machines_dir: &Path) -> Result<Vec<Machine>> {
    if !machines_dir.exists() {
        return Ok(Vec::new());
    }

    let mut machines = Vec::new();
    let entries = fs::read_dir(machines_dir)
        .with_context(|| format!("failed to read machines directory: {}", machines_dir.display()))?;

    for entry in entries.flatten() {
        let meta_file = entry.path().join("machine.json");
        if meta_file.exists() {
            if let Ok(content) = fs::read_to_string(&meta_file) {
                if let Ok(m) = serde_json::from_str::<Machine>(&content) {
                    machines.push(m);
                }
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

/// Persist a machine descriptor to disk.
pub fn save_machine(machines_dir: &Path, machine: &Machine) -> Result<()> {
    let dir = machines_dir.join(&machine.config.name);
    fs::create_dir_all(&dir)?;
    let path = dir.join("machine.json");
    let content = serde_json::to_string_pretty(machine)?;
    fs::write(path, content)?;
    Ok(())
}

/// Resolve the target machine using standard fallback priority:
/// 1. Explicit name (`name_opt`) -> if specified, must exist or error.
/// 2. `cfg.default_machine` -> if configured and exists on disk.
/// 3. Single running user machine -> if exactly one user machine is currently running.
/// 4. Single existing user machine -> if exactly one user machine exists overall.
/// 5. Error -> if none or ambiguous.
pub fn resolve_target_machine(cfg: &NorbConfig, name_opt: Option<&str>) -> Result<Machine> {
    resolve_target_machine_ext(cfg, name_opt, false)
}

pub fn resolve_target_machine_ext(cfg: &NorbConfig, name_opt: Option<&str>, allow_system: bool) -> Result<Machine> {
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
