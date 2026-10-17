//! Unified error type for the Norb VM manager.
//!
//! All public APIs in the Norb workspace return [`Result<T>`], which is an
//! alias for `std::result::Result<T, NorbError>`.  Downstream crates extend
//! this with their own `From` impls where necessary, but they always convert
//! into `NorbError` at the boundary so callers only ever see one error type.

use thiserror::Error;

/// The top-level error enum used across the entire Norb workspace.
///
/// Each variant corresponds to a distinct subsystem.  Variants that wrap a
/// plain `String` do so intentionally: the subsystem owning that error is
/// responsible for producing a human-readable message before converting.
#[derive(Debug, Error)]
pub enum MachError {
    /// An error originating from the macOS Hypervisor framework or the
    /// low-level hypervisor abstraction layer.
    #[error("hypervisor error: {0}")]
    Hypervisor(String),

    /// An error produced by the Virtual Machine Monitor (VMM) layer, e.g.
    /// failed to start or stop a guest vCPU.
    #[error("vmm error: {0}")]
    Vmm(String),

    /// An error related to machine lifecycle management (create, delete, list …).
    #[error("machine error: {0}")]
    Machine(String),

    /// An error in disk / image / storage operations.
    #[error("storage error: {0}")]
    Storage(String),

    /// An error in virtual or host networking (TAP interfaces, DHCP, NAT …).
    #[error("network error: {0}")]
    Network(String),

    /// A transparent wrapper around [`std::io::Error`]; converted automatically
    /// via the `#[from]` attribute so `?` just works on I/O operations.
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    /// An error caused by a missing, malformed, or unsatisfiable configuration
    /// value.
    #[error("config error: {0}")]
    Config(String),

    /// The requested resource does not exist.
    ///
    /// # Fields
    /// * `resource` — the kind of resource (e.g. `"machine"`, `"disk"`)
    /// * `name`     — the name / identifier that was looked up
    #[error("{resource} '{name}' not found")]
    NotFound { resource: String, name: String },

    /// Attempt to create a resource that already exists.
    ///
    /// # Fields
    /// * `resource` — the kind of resource
    /// * `name`     — the name / identifier that collided
    #[error("{resource} '{name}' already exists")]
    AlreadyExists { resource: String, name: String },

    /// The resource is in the wrong state for the requested operation.
    ///
    /// # Fields
    /// * `current`  — the state the resource is currently in
    /// * `expected` — the state(s) that would have been acceptable
    #[error("invalid state: resource is in '{current}' but expected '{expected}'")]
    InvalidState { current: String, expected: String },
}

pub type NorbError = MachError;

/// Convenience alias so every module can write `Result<T>` instead of
/// `Result<T, MachError>`.
pub type Result<T> = std::result::Result<T, MachError>;
