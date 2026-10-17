//! `machctl inspect` command implementation.
//!
//! Outputs detailed machine metadata, hypervisor runtime status, hardware specs,
//! network configuration, features, and storage paths.
//! Output is formatted as YAML by default, with `--json` support.

use anyhow::{bail, Context, Result};
use mach_common::config::MachConfig;
use mach_common::machine::MachineState;
use serde::Serialize;

use super::find_machine;

#[derive(Debug, Serialize)]
pub struct HardwareInspection {
    pub cpus: u32,
    pub memory_mb: u64,
    pub memory_human: String,
    pub disk_gb: u64,
    pub disk_human: String,
}

#[derive(Debug, Serialize)]
pub struct NetworkInspection {
    pub ip_address: Option<String>,
    pub mac_address: String,
    pub domain: String,
    pub ssh_command: String,
    pub ssh_port: u16,
}

#[derive(Debug, Serialize)]
pub struct FeaturesInspection {
    pub rosetta: String,
    pub virtiofs_users: String,
    pub docker_socket: Option<String>,
    pub isolated: bool,
    pub is_system: bool,
}

#[derive(Debug, Serialize)]
pub struct StorageInspection {
    pub machine_dir: String,
    pub disk_path: String,
    pub cidata_path: Option<String>,
    pub efi_vars_path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MachineInspection {
    pub name: String,
    pub id: String,
    pub distro: String,
    pub arch: String,
    pub state: String,
    pub pid: Option<u32>,
    pub created_at: String,
    pub hardware: HardwareInspection,
    pub network: NetworkInspection,
    pub features: FeaturesInspection,
    pub storage: StorageInspection,
}

pub async fn run(name: Option<&str>, _yaml: bool, json: bool) -> Result<()> {
    let cfg = MachConfig::load()?;

    let target_name = match name {
        Some(n) => n.to_string(),
        None => {
            if let Some(ref def) = cfg.default_machine {
                def.clone()
            } else {
                let machines = super::load_all_machines(&cfg.machines_dir)?;
                if machines.is_empty() {
                    bail!("no machines found. Run `mach create <distro>` first.");
                }
                machines[0].config.name.clone()
            }
        }
    };

    let machine = find_machine(&cfg.machines_dir, &target_name)?
        .with_context(|| format!("machine '{target_name}' not found"))?;

    let machine_dir = cfg.machines_dir.join(&machine.config.name);
    let disk_path = machine_dir.join("disk.raw");
    let cidata_path = machine_dir.join("cidata.iso");
    let efi_path = machine_dir.join("efi_vars.bin");

    let bytes = machine.id.as_bytes();
    let mac = format!("52:54:00:12:{:02x}:{:02x}", bytes[14], bytes[15]);

    let ip_str = machine.ip_address.map(|ip| ip.to_string());
    let ssh_cmd = match &ip_str {
        Some(ip) => format!("ssh root@{ip}"),
        None => format!("ssh root@{}.mach.local", machine.config.name),
    };

    let docker_sock = cfg.data_dir.join("run").join("docker.sock");
    let docker_sock_str = if docker_sock.exists() {
        Some(format!("unix://{}", docker_sock.display()))
    } else {
        None
    };

    let mem_gb = machine.config.memory_mb as f64 / 1024.0;
    let mem_human = format!("{:.1} GiB", mem_gb);
    let disk_human = format!("{} GiB", machine.config.disk_gb);

    let rosetta_status = if machine.state == MachineState::Running {
        "active (x86_64 emulation enabled)".to_string()
    } else {
        "supported".to_string()
    };

    let virtiofs_status = if machine.config.isolated {
        "disabled (isolated sandbox)".to_string()
    } else {
        "mounted (/Users -> 1:1 host sharing)".to_string()
    };

    let inspection = MachineInspection {
        name: machine.config.name.clone(),
        id: machine.id.to_string(),
        distro: machine.config.distro.clone(),
        arch: machine.config.arch.to_string(),
        state: machine.state.to_string(),
        pid: machine.pid,
        created_at: machine.created_at.to_rfc3339(),
        hardware: HardwareInspection {
            cpus: machine.config.cpus,
            memory_mb: machine.config.memory_mb,
            memory_human: mem_human,
            disk_gb: machine.config.disk_gb,
            disk_human,
        },
        network: NetworkInspection {
            ip_address: ip_str,
            mac_address: mac,
            domain: format!("{}.mach.local", machine.config.name),
            ssh_command: ssh_cmd,
            ssh_port: 22,
        },
        features: FeaturesInspection {
            rosetta: rosetta_status,
            virtiofs_users: virtiofs_status,
            docker_socket: docker_sock_str,
            isolated: machine.config.isolated,
            is_system: machine.is_system(),
        },
        storage: StorageInspection {
            machine_dir: machine_dir.display().to_string(),
            disk_path: disk_path.display().to_string(),
            cidata_path: if cidata_path.exists() { Some(cidata_path.display().to_string()) } else { None },
            efi_vars_path: if efi_path.exists() { Some(efi_path.display().to_string()) } else { None },
        },
    };

    if json {
        let json_output = serde_json::to_string_pretty(&inspection)?;
        println!("{json_output}");
    } else {
        let yaml_output = serde_yaml::to_string(&inspection)?;
        print!("{yaml_output}");
    }

    Ok(())
}
