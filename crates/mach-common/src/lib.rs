//! `norb-common` — shared types, errors, configuration, and utilities
//! used across every crate in the Norb VM manager workspace.
//!
//! # Modules
//! - [`error`]   — unified [`NorbError`](error::NorbError) type and [`Result`](error::Result) alias
//! - [`config`]  — [`NorbConfig`](config::NorbConfig) (load / save from `~/.norb/config.json`)
//! - [`machine`] — VM descriptor types ([`Machine`](machine::Machine), states, architecture …)
//! - [`logging`] — one-shot [`init_logging`](logging::init_logging) helper

pub mod config;
pub mod error;
pub mod fs;
pub mod logging;
pub mod machine;
