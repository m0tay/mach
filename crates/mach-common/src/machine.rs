//! VM descriptor types used throughout the Mach workspace.
//!
//! The central types are:
//! - [`MachineState`]  — lifecycle state machine for a VM
//! - [`Architecture`]  — supported guest CPU architectures
//! - [`MachineConfig`] — static configuration chosen at creation time
//! - [`Machine`]       — runtime descriptor combining config + dynamic state

use std::{fmt, net::IpAddr, path::Path};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use crate::config::atomic_write;
use crate::error::{MachError, Result};

// ---------------------------------------------------------------------------
// MachineState
// ---------------------------------------------------------------------------

/// The lifecycle state of a Mach virtual machine.
///
/// The valid transitions are:
///
/// ```text
/// Creating → Stopped → Starting → Running → Stopping → Stopped
///     │                                                     │
///     └──────────────────── Error ──────────────────────────┘
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "message")]
pub enum MachineState {
    /// The machine record and associated disk image are being created.
    Creating,
    /// The machine exists on disk but no VMM process is running.
    Stopped,
    /// A VMM process has been spawned and the guest is booting.
    Starting,
    /// The guest OS is fully up and accepting connections.
    Running,
    /// A graceful shutdown has been requested; waiting for the guest to halt.
    Stopping,
    /// The machine encountered an unrecoverable error.  The inner string
    /// carries a human-readable explanation.
    Error(String),
}

impl fmt::Display for MachineState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Creating  => write!(f, "creating"),
            Self::Stopped   => write!(f, "stopped"),
            Self::Starting  => write!(f, "starting"),
            Self::Running   => write!(f, "running"),
            Self::Stopping  => write!(f, "stopping"),
            Self::Error(msg) => write!(f, "error: {msg}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Architecture
// ---------------------------------------------------------------------------

/// Guest CPU architecture.
///
/// On Apple Silicon hosts only `Aarch64` can run with full HW acceleration via
/// the macOS Hypervisor framework.  `X86_64` support is planned for a future
/// release (emulated via a software back-end).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Architecture {
    /// 64-bit ARM (AArch64 / ARM64) — native on Apple Silicon.
    Aarch64,
    /// x86-64 / AMD64 — currently emulated.
    X86_64,
}

/// Defaults to [`Architecture::Aarch64`] since Mach targets Apple Silicon.
impl Default for Architecture {
    fn default() -> Self {
        Self::Aarch64
    }
}

impl fmt::Display for Architecture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Aarch64 => write!(f, "aarch64"),
            Self::X86_64  => write!(f, "x86_64"),
        }
    }
}

// ---------------------------------------------------------------------------
// MachineConfig
// ---------------------------------------------------------------------------

/// Static configuration chosen when a machine is first created.
///
/// Once the machine is created this struct should be treated as immutable by
/// management code — any mutation requires recreating the machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineConfig {
    /// Human-readable name, unique within the user's machine collection.
    pub name: String,

    /// Linux distribution identifier, e.g. `"ubuntu-24.04"` or `"alpine-3.20"`.
    pub distro: String,

    /// Guest CPU architecture.
    pub arch: Architecture,

    /// Number of virtual CPUs exposed to the guest.
    pub cpus: u32,

    /// Guest RAM in mebibytes (MiB).
    pub memory_mb: u64,

    /// Root disk size in gibibytes (GiB).
    pub disk_gb: u64,

    /// Run in isolated sandbox mode (disables /Users file sharing and host bridge).
    #[serde(default)]
    pub isolated: bool,

    /// Whether this is a protected Mach system infrastructure machine (e.g. mach-docker, mach-k8s).
    #[serde(default)]
    pub is_system: bool,
}

// ---------------------------------------------------------------------------
// Machine
// ---------------------------------------------------------------------------

/// Runtime descriptor for a Mach virtual machine.
///
/// Instances are created via [`Machine::new`] and persisted as JSON inside
/// `~/.mach/machines/<id>/machine.json`.  The [`state`](Machine::state) and
/// [`pid`](Machine::pid) fields are updated dynamically by the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Machine {
    /// Stable, opaque identifier.  Generated once at creation and never
    /// changed.
    pub id: Uuid,

    /// The immutable configuration this machine was created with.
    pub config: MachineConfig,

    /// Current lifecycle state.
    pub state: MachineState,

    /// Wall-clock time when the machine record was first created (UTC).
    pub created_at: DateTime<Utc>,

    /// IPv4 or IPv6 address assigned to the guest's primary interface.
    /// `None` until the guest has fully booted and obtained an address.
    pub ip_address: Option<IpAddr>,

    /// Operating system PID of the VMM process while the machine is in the
    /// [`Running`](MachineState::Running) or [`Starting`](MachineState::Starting)
    /// state.  `None` at all other times.
    pub pid: Option<u32>,
}

impl Machine {
    /// Create a new [`Machine`] from the given [`MachineConfig`].
    ///
    /// The machine is initialised in the [`MachineState::Creating`] state with
    /// a fresh UUID and the current UTC timestamp.  Callers should transition
    /// the state to [`MachineState::Stopped`] once the disk image has been
    /// provisioned successfully.
    pub fn new(config: MachineConfig) -> Self {
        Self {
            id: Uuid::new_v4(),
            config,
            state: MachineState::Creating,
            created_at: Utc::now(),
            ip_address: None,
            pid: None,
        }
    }

    /// Returns the short (8-hex-character) form of the machine's UUID,
    /// suitable for display in terminal output.
    pub fn short_id(&self) -> String {
        self.id.to_string()[..8].to_string()
    }

    /// Returns `true` if the machine has a live VMM process (i.e. is
    /// [`Starting`](MachineState::Starting) or [`Running`](MachineState::Running)).
    pub fn is_live(&self) -> bool {
        matches!(self.state, MachineState::Starting | MachineState::Running)
    }

    /// Returns `true` if this is a protected Mach system infrastructure machine
    /// (e.g. mach-docker, mach-k8s, or explicitly marked is_system).
    pub fn is_system(&self) -> bool {
        self.config.is_system
            || self.config.name == "mach-docker"
            || self.config.name == "mach-k8s"
            || self.config.name == "mach-kubernetes"
    }

    /// Returns human-readable system service name if this is an infrastructure machine.
    pub fn system_service_name(&self) -> Option<&'static str> {
        if self.config.name == "mach-docker" {
            Some("Docker & K8s")
        } else if self.config.name == "mach-k8s" || self.config.name == "mach-kubernetes" {
            Some("Kubernetes")
        } else if self.config.is_system {
            Some("System")
        } else {
            None
        }
    }

    /// Persist this machine descriptor to disk under `machines_dir` atomically.
    pub fn save(&self, machines_dir: &Path) -> Result<()> {
        save_machine(machines_dir, self)
    }
}

/// Load a machine descriptor from disk inside `<machine_dir>/machine.yaml` (or fallback `machine.json`).
/// If a legacy `machine.json` is encountered, it is automatically migrated to `machine.yaml`.
pub fn load_machine(machine_dir: &Path) -> Result<Machine> {
    let yaml_path = machine_dir.join("machine.yaml");
    let json_path = machine_dir.join("machine.json");

    if yaml_path.exists() {
        let content = std::fs::read_to_string(&yaml_path)?;
        let m: Machine = serde_yaml::from_str(&content)
            .map_err(|e| MachError::Machine(format!("failed to parse machine.yaml in {}: {e}", machine_dir.display())))?;
        return Ok(m);
    }

    if json_path.exists() {
        let content = std::fs::read_to_string(&json_path)?;
        let m: Machine = serde_json::from_str(&content)
            .map_err(|e| MachError::Machine(format!("failed to parse machine.json in {}: {e}", machine_dir.display())))?;
        // Auto-migrate to machine.yaml
        if let Ok(yaml_str) = serde_yaml::to_string(&m) {
            let _ = atomic_write(&yaml_path, yaml_str.as_bytes());
            let _ = std::fs::remove_file(&json_path);
        }
        return Ok(m);
    }

    Err(MachError::NotFound {
        resource: "machine configuration".to_string(),
        name: machine_dir.display().to_string(),
    })
}

/// Persist a machine descriptor to disk inside `<machines_dir>/<machine.config.name>/machine.yaml`
/// using atomic file write semantics to prevent 0-byte corruptions.
pub fn save_machine(machines_dir: &Path, machine: &Machine) -> Result<()> {
    let dir = machines_dir.join(&machine.config.name);
    std::fs::create_dir_all(&dir)?;
    let yaml_path = dir.join("machine.yaml");
    let content = serde_yaml::to_string(machine)
        .map_err(|e| MachError::Machine(format!("failed to serialize machine {}: {e}", machine.config.name)))?;
    atomic_write(&yaml_path, content.as_bytes())?;
    let _ = std::fs::remove_file(dir.join("machine.json"));
    Ok(())
}

impl fmt::Display for Machine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ({}) [{}/{}vCPU/{}MiB] — {}",
            self.config.name,
            self.short_id(),
            self.config.arch,
            self.config.cpus,
            self.config.memory_mb,
            self.state,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_machine_config_isolated_default() {
        let json_without_isolated = r#"{
            "name": "test-vm",
            "distro": "ubuntu",
            "arch": "Aarch64",
            "cpus": 2,
            "memory_mb": 2048,
            "disk_gb": 20
        }"#;
        let cfg: MachineConfig = serde_json::from_str(json_without_isolated).unwrap();
        assert!(!cfg.isolated);
    }

    #[test]
    fn test_machine_config_isolated_explicit() {
        let json_with_isolated = r#"{
            "name": "test-isolated-vm",
            "distro": "ubuntu",
            "arch": "Aarch64",
            "cpus": 4,
            "memory_mb": 4096,
            "disk_gb": 40,
            "isolated": true
        }"#;
        let cfg: MachineConfig = serde_json::from_str(json_with_isolated).unwrap();
        assert!(cfg.isolated);

        let serialized = serde_json::to_string(&cfg).unwrap();
        assert!(serialized.contains(r#""isolated":true"#));
    }

    #[test]
    fn test_machine_yaml_serialization_roundtrip() {
        let machine = Machine::new(MachineConfig {
            name: "yaml-vm".to_string(),
            distro: "debian".to_string(),
            arch: Architecture::Aarch64,
            cpus: 4,
            memory_mb: 4096,
            disk_gb: 30,
            isolated: false,
            is_system: false,
        });

        let yaml = serde_yaml::to_string(&machine).unwrap();
        assert!(yaml.contains("name: yaml-vm"));
        assert!(yaml.contains("cpus: 4"));
        assert!(yaml.contains("memory_mb: 4096"));

        let deserialized: Machine = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(deserialized.config.name, "yaml-vm");
        assert_eq!(deserialized.config.cpus, 4);
        assert_eq!(deserialized.config.memory_mb, 4096);
    }

    #[test]
    fn test_machine_auto_migration_from_json_to_yaml() {
        let temp_dir = std::env::temp_dir().join(format!("mach_test_mig_{}", uuid::Uuid::new_v4()));
        let machine_dir = temp_dir.join("test-mig");
        std::fs::create_dir_all(&machine_dir).unwrap();

        let machine = Machine::new(MachineConfig {
            name: "test-mig".to_string(),
            distro: "alpine".to_string(),
            arch: Architecture::Aarch64,
            cpus: 2,
            memory_mb: 2048,
            disk_gb: 10,
            isolated: false,
            is_system: false,
        });

        // Write legacy machine.json
        let json_content = serde_json::to_string_pretty(&machine).unwrap();
        let json_path = machine_dir.join("machine.json");
        std::fs::write(&json_path, json_content).unwrap();

        // Verify machine.yaml doesn't exist yet
        let yaml_path = machine_dir.join("machine.yaml");
        assert!(!yaml_path.exists());

        // load_machine should parse JSON and automatically migrate to YAML
        let loaded = load_machine(&machine_dir).unwrap();
        assert_eq!(loaded.config.name, "test-mig");
        assert!(yaml_path.exists(), "machine.yaml should have been created on migration");
        assert!(!json_path.exists(), "machine.json should have been cleaned up");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}

