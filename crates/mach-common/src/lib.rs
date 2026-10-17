//! `mach-common` — shared types, errors, configuration, and utilities
//! used across every crate in the Mach VM manager workspace.
//!
//! # Modules
//! - [`error`]   — unified [`MachError`](error::MachError) type and [`Result`](error::Result) alias
//! - [`config`]  — [`MachConfig`](config::MachConfig) (load / save from `~/.mach/config.yaml`)
//! - [`machine`] — VM descriptor types ([`Machine`](machine::Machine), states, architecture …)
//! - [`logging`] — one-shot [`init_logging`](logging::init_logging) helper

pub mod config;
pub mod error;
pub mod fs;
pub mod logging;
pub mod machine;
