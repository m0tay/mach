//! Command implementations for `norb`.

pub mod boot;
pub mod clone_cmd;
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
pub mod list;
pub mod logs;
pub mod mount;
pub mod push_pull;
pub mod rename_cmd;
pub mod restart;
pub mod run_cmd;
pub mod run_vm;
pub mod shell;
pub mod ssh;
pub mod start;
pub mod status;
pub mod stop;
pub mod top;
pub mod version;

use anyhow::{Context, Result};
use norb_common::machine::Machine;
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
