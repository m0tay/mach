//! VM descriptor types used throughout the Norb workspace.
//!
//! The central types are:
//! - [`MachineState`]  — lifecycle state machine for a VM
//! - [`Architecture`]  — supported guest CPU architectures
//! - [`MachineConfig`] — static configuration chosen at creation time
//! - [`Machine`]       — runtime descriptor combining config + dynamic state

use std::{fmt, net::IpAddr};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// MachineState
// ---------------------------------------------------------------------------

/// The lifecycle state of a Norb virtual machine.
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

/// Defaults to [`Architecture::Aarch64`] since Norb targets Apple Silicon.
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
}

// ---------------------------------------------------------------------------
// Machine
// ---------------------------------------------------------------------------

/// Runtime descriptor for a Norb virtual machine.
///
/// Instances are created via [`Machine::new`] and persisted as JSON inside
/// `~/.norb/machines/<id>/machine.json`.  The [`state`](Machine::state) and
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
