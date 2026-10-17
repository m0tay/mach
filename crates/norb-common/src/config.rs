//! Global Norb configuration — loaded from `~/.norb/config.json` at start-up.
//!
//! The configuration file is optional; if it does not exist [`NorbConfig::load`]
//! returns sensible defaults and does **not** create the file automatically.
//! Call [`NorbConfig::save`] explicitly when you want to persist changes.
//!
//! # Example
//! ```no_run
//! use norb_common::config::NorbConfig;
//!
//! let mut cfg = NorbConfig::load().expect("failed to load config");
//! cfg.log_level = "debug".to_string();
//! cfg.save().expect("failed to save config");
//! ```

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{NorbError, Result};

// ---------------------------------------------------------------------------
// NorbConfig
// ---------------------------------------------------------------------------

/// Runtime configuration for the Norb daemon and CLI.
///
/// All path fields are fully expanded (no `~` tilde) by the time they reach
/// callers — the tilde expansion happens inside [`NorbConfig::default`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NorbConfig {
    /// Root data directory.  Every piece of persistent state lives under here.
    ///
    /// Default: `~/.norb`
    pub data_dir: PathBuf,

    /// Directory that contains one sub-directory per machine.
    ///
    /// Default: `~/.norb/machines`
    pub machines_dir: PathBuf,

    /// Unix-domain socket path used by the Norb daemon for IPC.
    ///
    /// Default: `~/.norb/run/norb.sock`
    pub socket_path: PathBuf,

    /// Minimum tracing log level (`"trace"`, `"debug"`, `"info"`, `"warn"`,
    /// `"error"`).
    ///
    /// Default: `"info"`
    pub log_level: String,

    /// Default number of vCPUs assigned to a new machine when not explicitly
    /// specified by the caller.
    ///
    /// Default: `2`
    pub default_cpus: u32,

    /// Default RAM in mebibytes assigned to a new machine.
    ///
    /// Default: `2048`
    pub default_memory_mb: u64,

    /// Name of the machine to use when a command omits an explicit target.
    /// `None` means "no default — always require an explicit name".
    ///
    /// Default: `None`
    pub default_machine: Option<String>,
}

impl Default for NorbConfig {
    fn default() -> Self {
        // Resolve the home directory at construction time so paths are always
        // absolute.  If dirs::home_dir() returns None (highly unusual on macOS)
        // we fall back to `/tmp/norb` so the process can at least start.
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
        let data_dir = home.join(".norb");
        let machines_dir = data_dir.join("machines");
        let socket_path = data_dir.join("run").join("norb.sock");

        Self {
            data_dir,
            machines_dir,
            socket_path,
            log_level: "info".to_string(),
            default_cpus: 2,
            default_memory_mb: 2048,
            default_machine: None,
        }
    }
}

impl NorbConfig {
    /// Returns the canonical path to the configuration file.
    ///
    /// This is always `<data_dir>/config.json`, which resolves to
    /// `~/.norb/config.json` under a default configuration.
    pub fn config_path() -> PathBuf {
        // We need the data_dir to build the path, so we instantiate a default
        // just to grab the directory; no I/O is performed.
        Self::default().data_dir.join("config.json")
    }

    /// Load configuration from `~/.norb/config.json`.
    ///
    /// # Behaviour
    /// - If the file **does not exist**, returns [`NorbConfig::default`].
    /// - If the file **exists but is malformed**, returns
    ///   [`NorbError::Config`] with a descriptive message.
    /// - Unknown JSON keys are silently ignored (forward-compatibility).
    ///
    /// # Errors
    /// - [`NorbError::Io`]     — file exists but could not be read.
    /// - [`NorbError::Config`] — file exists but could not be parsed.
    pub fn load() -> Result<Self> {
        let path = Self::config_path();

        if !path.exists() {
            tracing::debug!(
                path = %path.display(),
                "config file not found, using defaults"
            );
            return Ok(Self::default());
        }

        tracing::debug!(path = %path.display(), "loading config");

        let raw = std::fs::read_to_string(&path)?;

        let cfg: Self = serde_json::from_str(&raw).map_err(|e| {
            NorbError::Config(format!(
                "failed to parse {}: {e}",
                path.display()
            ))
        })?;

        Ok(cfg)
    }

    /// Persist the current configuration to `~/.norb/config.json`.
    ///
    /// Parent directories are created automatically if they do not exist.
    ///
    /// # Errors
    /// - [`NorbError::Io`]     — directory creation or file write failed.
    /// - [`NorbError::Config`] — serialisation failed (should never happen for
    ///   a well-formed [`NorbConfig`]).
    pub fn save(&self) -> Result<()> {
        let path = Self::config_path();

        // Ensure the parent directory exists.
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let json = serde_json::to_string_pretty(self).map_err(|e| {
            NorbError::Config(format!("failed to serialise config: {e}"))
        })?;

        atomic_write(&path, json.as_bytes())?;

        tracing::debug!(path = %path.display(), "config saved");

        Ok(())
    }
}

/// Write data to a temporary file `<path>.tmp.<pid>` and atomically rename it to
/// `<path>` to prevent 0-byte file corruptions.
pub fn atomic_write(path: impl AsRef<std::path::Path>, content: impl AsRef<[u8]>) -> std::io::Result<()> {
    let path = path.as_ref();
    let pid = std::process::id();
    let tmp_path = PathBuf::from(format!("{}.tmp.{}", path.display(), pid));

    std::fs::write(&tmp_path, content)?;
    if let Err(e) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }
    Ok(())
}
