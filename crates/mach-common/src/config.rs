//! Global Mach configuration — loaded from `~/.mach/config.yaml` (or fallback `config.json`) at start-up.
//!
//! The configuration file is optional; if it does not exist [`MachConfig::load`]
//! returns sensible defaults and does **not** create the file automatically.
//! Call [`MachConfig::save`] explicitly when you want to persist changes.
//!
//! # Example
//! ```no_run
//! use mach_common::config::MachConfig;
//!
//! let mut cfg = MachConfig::load().expect("failed to load config");
//! cfg.log_level = "debug".to_string();
//! cfg.save().expect("failed to save config");
//! ```

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{MachError, Result};

// ---------------------------------------------------------------------------
// NetworkConfig
// ---------------------------------------------------------------------------

/// Network configuration for Mach virtual machines and local services.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkConfig {
    /// Enable NAT networking for virtual machines.
    ///
    /// Default: `true`
    pub nat: bool,

    /// Local DNS responder port on 127.0.0.1.
    ///
    /// Default: `5354`
    pub dns_port: u16,

    /// Local domain suffix for machines and containers.
    ///
    /// Default: `"mach.local"`
    pub domain: String,

    /// Port for the host bridge daemon (e.g. `mac open`, `mac pbcopy`).
    ///
    /// Default: `32222`
    pub host_bridge_port: u16,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            nat: true,
            dns_port: 5354,
            domain: "mach.local".to_string(),
            host_bridge_port: 32222,
        }
    }
}

// ---------------------------------------------------------------------------
// DockerConfig
// ---------------------------------------------------------------------------

/// Docker integration configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DockerConfig {
    /// Enable Docker socket forwarding and integration.
    ///
    /// Default: `true`
    pub enabled: bool,

    /// Path to the local Docker socket.
    ///
    /// Default: `"~/.mach/run/docker.sock"`
    pub socket_path: PathBuf,

    /// Whether to create a system symlink at `/var/run/docker.sock`.
    ///
    /// Default: `false`
    pub system_symlink: bool,

    /// Whether Docker was running last time and should be automatically restored on Mach startup.
    ///
    /// Default: `false`
    #[serde(default, alias = "autostart", alias = "auto_start")]
    pub was_running: bool,
}

impl Default for DockerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            socket_path: PathBuf::from("~/.mach/run/docker.sock"),
            system_symlink: false,
            was_running: false,
        }
    }
}

impl DockerConfig {
    /// Returns the resolved socket path, expanding `~` to the user's home directory.
    pub fn resolved_socket_path(&self) -> PathBuf {
        let s = self.socket_path.to_string_lossy();
        if let Some(stripped) = s.strip_prefix("~/") {
            if let Some(home) = dirs::home_dir() {
                return home.join(stripped);
            }
        }
        self.socket_path.clone()
    }
}

// ---------------------------------------------------------------------------
// K8sConfig
// ---------------------------------------------------------------------------

/// Kubernetes (K3s) integration configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct K8sConfig {
    /// Port for the Kubernetes API server endpoint.
    ///
    /// Default: `26443`
    pub port: u16,

    /// Whether Kubernetes was running last time and should be automatically restored on Mach startup.
    ///
    /// Default: `false`
    #[serde(default, alias = "autostart", alias = "auto_start")]
    pub was_running: bool,
}

impl Default for K8sConfig {
    fn default() -> Self {
        Self {
            port: 26443,
            was_running: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Helper default functions for serde deserialization
// ---------------------------------------------------------------------------

fn default_distro_val() -> String {
    "ubuntu".to_string()
}

fn default_cpus_val() -> u32 {
    2
}

fn default_memory_val() -> u64 {
    2048
}

fn default_disk_gib_val() -> u64 {
    20
}

fn default_rosetta_val() -> bool {
    true
}

// ---------------------------------------------------------------------------
// MachConfig
// ---------------------------------------------------------------------------

/// Runtime configuration for the Mach daemon and CLI.
///
/// All path fields are fully expanded (no `~` tilde) by the time they reach
/// callers — the tilde expansion happens inside [`MachConfig::default`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MachConfig {
    /// Default Linux distribution for newly created machines.
    ///
    /// Default: `"ubuntu"`
    #[serde(default = "default_distro_val")]
    pub default_distro: String,

    /// Default number of vCPUs assigned to a new machine when not explicitly
    /// specified by the caller.
    ///
    /// Default: `2`
    #[serde(default = "default_cpus_val")]
    pub default_cpus: u32,

    /// Default RAM in mebibytes assigned to a new machine.
    ///
    /// Default: `2048`
    #[serde(default = "default_memory_val")]
    pub default_memory_mib: u64,

    /// Deprecated legacy alias for `default_memory_mib`. Kept for backward compatibility
    /// with existing callers and older config formats.
    #[serde(default = "default_memory_val", skip_serializing)]
    pub default_memory_mb: u64,

    /// Default disk size in gibibytes assigned to a new machine.
    ///
    /// Default: `20`
    #[serde(default = "default_disk_gib_val", alias = "default_disk_gb")]
    pub default_disk_gib: u64,

    /// Enable Apple Silicon Rosetta 2 for Linux x86_64 emulation by default.
    ///
    /// Default: `true`
    #[serde(default = "default_rosetta_val")]
    pub rosetta: bool,

    /// Network configuration.
    pub network: NetworkConfig,

    /// Docker integration configuration.
    pub docker: DockerConfig,

    /// Kubernetes (K3s) configuration.
    pub k8s: K8sConfig,

    /// Name of the machine to use when a command omits an explicit target.
    /// `None` means "no default — always require an explicit name".
    ///
    /// Default: `None`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_machine: Option<String>,

    /// Minimum tracing log level (`"trace"`, `"debug"`, `"info"`, `"warn"`,
    /// `"error"`).
    ///
    /// Default: `"info"`
    pub log_level: String,

    /// Root data directory. Every piece of persistent state lives under here.
    ///
    /// Default: `~/.mach`
    pub data_dir: PathBuf,

    /// Directory that contains one sub-directory per machine.
    ///
    /// Default: `~/.mach/machines`
    pub machines_dir: PathBuf,

    /// Unix-domain socket path used by the Mach daemon for IPC.
    ///
    /// Default: `~/.mach/run/mach.sock`
    pub socket_path: PathBuf,
}

impl Default for MachConfig {
    fn default() -> Self {
        // Resolve the home directory at construction time so paths are always
        // absolute. If dirs::home_dir() returns None (highly unusual on macOS)
        // we fall back to `/tmp/mach` so the process can at least start.
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
        let data_dir = if let Ok(val) = std::env::var("MACH_HOME") {
            PathBuf::from(val)
        } else {
            home.join(".mach")
        };
        let machines_dir = data_dir.join("machines");
        let socket_path = data_dir.join("run").join("mach.sock");

        Self {
            default_distro: "ubuntu".to_string(),
            default_cpus: 2,
            default_memory_mib: 2048,
            default_memory_mb: 2048,
            default_disk_gib: 20,
            rosetta: true,
            network: NetworkConfig::default(),
            docker: DockerConfig::default(),
            k8s: K8sConfig::default(),
            default_machine: None,
            log_level: "info".to_string(),
            data_dir,
            machines_dir,
            socket_path,
        }
    }
}

impl MachConfig {
    /// Returns the canonical path to the primary YAML configuration file.
    ///
    /// This is `<data_dir>/config.yaml`, which resolves to
    /// `~/.mach/config.yaml` under default settings.
    pub fn config_path() -> PathBuf {
        Self::yaml_path()
    }

    /// Returns the path to `~/.mach/config.yaml`.
    pub fn yaml_path() -> PathBuf {
        Self::default().data_dir.join("config.yaml")
    }

    /// Returns the path to legacy `~/.mach/config.json`.
    pub fn json_path() -> PathBuf {
        Self::default().data_dir.join("config.json")
    }

    /// Synchronize memory fields between `default_memory_mb` and `default_memory_mib` and normalize paths.
    pub fn normalize(&mut self) {
        if self.default_memory_mb != 2048 && self.default_memory_mib == 2048 {
            self.default_memory_mib = self.default_memory_mb;
        } else if self.default_memory_mib != 2048 && self.default_memory_mb == 2048 {
            self.default_memory_mb = self.default_memory_mib;
        } else if self.default_memory_mib != self.default_memory_mb {
            self.default_memory_mb = self.default_memory_mib;
        }
    }

    /// Sets both `default_memory_mib` and legacy `default_memory_mb`.
    pub fn set_memory_mib(&mut self, mib: u64) {
        self.default_memory_mib = mib;
        self.default_memory_mb = mib;
    }

    /// Load configuration from `~/.mach/config.yaml` if it exists, falling back
    /// to `~/.mach/config.json`.
    ///
    /// # Behaviour
    /// - If `config.yaml` exists, it is parsed via [`serde_yaml`].
    /// - If `config.yaml` does not exist but `config.json` exists, it is parsed via [`serde_json`].
    /// - If neither exists, returns [`MachConfig::default`].
    /// - Unknown keys are silently ignored for forward-compatibility.
    ///
    /// # Errors
    /// - [`MachError::Io`]     — file exists but could not be read.
    /// - [`MachError::Config`] — file exists but could not be parsed.
    pub fn load() -> Result<Self> {
        let default_cfg = Self::default();
        Self::load_from_dir(&default_cfg.data_dir)
    }

    /// Load configuration from a specific base directory.
    pub fn load_from_dir(data_dir: &Path) -> Result<Self> {
        let yaml_path = data_dir.join("config.yaml");
        let json_path = data_dir.join("config.json");

        if yaml_path.exists() {
            tracing::debug!(path = %yaml_path.display(), "loading YAML config");
            let raw = std::fs::read_to_string(&yaml_path)?;
            if raw.trim().is_empty() {
                return Ok(Self::default());
            }
            let mut cfg: Self = serde_yaml::from_str(&raw).map_err(|e| {
                MachError::Config(format!(
                    "failed to parse {}: {e}",
                    yaml_path.display()
                ))
            })?;
            cfg.normalize();
            return Ok(cfg);
        }

        if json_path.exists() {
            tracing::debug!(path = %json_path.display(), "YAML config not found, loading fallback JSON config");
            let raw = std::fs::read_to_string(&json_path)?;
            if raw.trim().is_empty() {
                return Ok(Self::default());
            }
            let mut cfg: Self = serde_json::from_str(&raw).map_err(|e| {
                MachError::Config(format!(
                    "failed to parse {}: {e}",
                    json_path.display()
                ))
            })?;
            cfg.normalize();
            return Ok(cfg);
        }

        tracing::debug!(
            yaml_path = %yaml_path.display(),
            json_path = %json_path.display(),
            "config file not found, using defaults"
        );
        Ok(Self::default())
    }

    /// Persist the current configuration to `~/.mach/config.yaml` using YAML format.
    ///
    /// Parent directories are created automatically if they do not exist.
    ///
    /// # Errors
    /// - [`MachError::Io`]     — directory creation or file write failed.
    /// - [`MachError::Config`] — serialisation failed.
    pub fn save(&self) -> Result<()> {
        Self::save_to_dir(self, &self.data_dir)
    }

    /// Persist configuration to `<data_dir>/config.yaml`.
    pub fn save_to_dir(&self, data_dir: &Path) -> Result<()> {
        let path = data_dir.join("config.yaml");

        // Ensure the parent directory exists.
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut cfg = self.clone();
        if cfg.default_memory_mib != 2048 && cfg.default_memory_mb == 2048 {
            cfg.default_memory_mb = cfg.default_memory_mib;
        } else if cfg.default_memory_mb != cfg.default_memory_mib {
            cfg.default_memory_mib = cfg.default_memory_mb;
        }

        let yaml = serde_yaml::to_string(&cfg).map_err(|e| {
            MachError::Config(format!("failed to serialise config: {e}"))
        })?;

        atomic_write(&path, yaml.as_bytes())?;

        tracing::debug!(path = %path.display(), "config saved");

        Ok(())
    }

    /// Persist configuration to `~/.mach/config.json` (for legacy JSON compatibility).
    pub fn save_json(&self) -> Result<()> {
        Self::save_json_to_dir(self, &self.data_dir)
    }

    /// Persist configuration to `<data_dir>/config.json`.
    pub fn save_json_to_dir(&self, data_dir: &Path) -> Result<()> {
        let path = data_dir.join("config.json");

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut cfg = self.clone();
        if cfg.default_memory_mib != 2048 && cfg.default_memory_mb == 2048 {
            cfg.default_memory_mb = cfg.default_memory_mib;
        } else if cfg.default_memory_mb != cfg.default_memory_mib {
            cfg.default_memory_mib = cfg.default_memory_mb;
        }

        let json = serde_json::to_string_pretty(&cfg).map_err(|e| {
            MachError::Config(format!("failed to serialise config to JSON: {e}"))
        })?;

        atomic_write(&path, json.as_bytes())?;

        tracing::debug!(path = %path.display(), "config saved to JSON");

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let cfg = MachConfig::default();
        assert_eq!(cfg.default_distro, "ubuntu");
        assert_eq!(cfg.default_cpus, 2);
        assert_eq!(cfg.default_memory_mib, 2048);
        assert_eq!(cfg.default_memory_mb, 2048);
        assert_eq!(cfg.default_disk_gib, 20);
        assert!(cfg.rosetta);
        assert_eq!(cfg.network.nat, true);
        assert_eq!(cfg.network.dns_port, 5354);
        assert_eq!(cfg.network.domain, "mach.local");
        assert_eq!(cfg.network.host_bridge_port, 32222);
        assert_eq!(cfg.docker.enabled, true);
        assert_eq!(cfg.docker.socket_path, PathBuf::from("~/.mach/run/docker.sock"));
        assert_eq!(cfg.docker.system_symlink, false);
        assert_eq!(cfg.docker.was_running, false);
        assert_eq!(cfg.k8s.port, 26443);
        assert_eq!(cfg.k8s.was_running, false);
        assert_eq!(cfg.log_level, "info");
        assert_eq!(cfg.default_machine, None);
    }

    #[test]
    fn test_yaml_serialization_roundtrip() {
        let dir = std::env::temp_dir().join(format!("mach-test-yaml-{}", uuid::Uuid::new_v4()));
        let mut cfg = MachConfig::default();
        cfg.default_distro = "debian".to_string();
        cfg.default_cpus = 4;
        cfg.default_memory_mib = 4096;
        cfg.default_disk_gib = 50;

        cfg.save_to_dir(&dir).expect("save YAML failed");

        let loaded = MachConfig::load_from_dir(&dir).expect("load YAML failed");
        assert_eq!(loaded.default_distro, "debian");
        assert_eq!(loaded.default_cpus, 4);
        assert_eq!(loaded.default_memory_mib, 4096);
        assert_eq!(loaded.default_memory_mb, 4096);
        assert_eq!(loaded.default_disk_gib, 50);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_fallback_to_json_when_yaml_absent() {
        let dir = std::env::temp_dir().join(format!("mach-test-fallback-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let json_content = r#"{
            "default_cpus": 6,
            "default_memory_mb": 8192,
            "log_level": "debug"
        }"#;
        std::fs::write(dir.join("config.json"), json_content).unwrap();

        let loaded = MachConfig::load_from_dir(&dir).expect("load fallback JSON failed");
        assert_eq!(loaded.default_cpus, 6);
        assert_eq!(loaded.default_memory_mb, 8192);
        assert_eq!(loaded.default_memory_mib, 8192); // Synced
        assert_eq!(loaded.log_level, "debug");
        assert_eq!(loaded.default_distro, "ubuntu"); // Defaults preserved

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_yaml_precedence_over_json() {
        let dir = std::env::temp_dir().join(format!("mach-test-prec-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let yaml_content = "default_cpus: 8\ndefault_distro: fedora\n";
        let json_content = r#"{"default_cpus": 4, "default_distro": "debian"}"#;

        std::fs::write(dir.join("config.yaml"), yaml_content).unwrap();
        std::fs::write(dir.join("config.json"), json_content).unwrap();

        let loaded = MachConfig::load_from_dir(&dir).expect("load failed");
        assert_eq!(loaded.default_cpus, 8);
        assert_eq!(loaded.default_distro, "fedora");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_atomic_write() {
        let dir = std::env::temp_dir().join(format!("mach-test-atomic-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("test.txt");

        atomic_write(&target, b"hello world").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "hello world");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_docker_socket_resolution() {
        let docker_cfg = DockerConfig::default();
        let resolved = docker_cfg.resolved_socket_path();
        if let Some(home) = dirs::home_dir() {
            assert_eq!(resolved, home.join(".mach/run/docker.sock"));
        }
    }

    #[test]
    fn test_legacy_config_set_memory() {
        let dir = std::env::temp_dir().join(format!("mach-test-legacy-set-{}", uuid::Uuid::new_v4()));
        let mut cfg = MachConfig::default();
        // Emulate `mach config set default_memory_mb 8192`
        cfg.default_memory_mb = 8192;
        cfg.save_to_dir(&dir).unwrap();

        let loaded = MachConfig::load_from_dir(&dir).unwrap();
        assert_eq!(loaded.default_memory_mib, 8192);
        assert_eq!(loaded.default_memory_mb, 8192);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_empty_files_load_defaults() {
        let dir = std::env::temp_dir().join(format!("mach-test-empty-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        std::fs::write(dir.join("config.yaml"), "").unwrap();
        let loaded = MachConfig::load_from_dir(&dir).unwrap();
        assert_eq!(loaded.default_distro, "ubuntu");

        std::fs::remove_file(dir.join("config.yaml")).unwrap();
        std::fs::write(dir.join("config.json"), "   \n").unwrap();
        let loaded = MachConfig::load_from_dir(&dir).unwrap();
        assert_eq!(loaded.default_distro, "ubuntu");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_unknown_keys_ignored() {
        let dir = std::env::temp_dir().join(format!("mach-test-unknown-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let yaml = "unknown_future_key: 12345\ndefault_cpus: 4\n";
        std::fs::write(dir.join("config.yaml"), yaml).unwrap();

        let loaded = MachConfig::load_from_dir(&dir).unwrap();
        assert_eq!(loaded.default_cpus, 4);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_corrupt_files_error() {
        let dir = std::env::temp_dir().join(format!("mach-test-corrupt-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        std::fs::write(dir.join("config.yaml"), ": bad yaml :: [[").unwrap();
        assert!(MachConfig::load_from_dir(&dir).is_err());

        std::fs::remove_file(dir.join("config.yaml")).unwrap();
        std::fs::write(dir.join("config.json"), "{ bad json").unwrap();
        assert!(MachConfig::load_from_dir(&dir).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_was_running_and_autostart_parsing() {
        let dir = std::env::temp_dir().join(format!("mach-test-autostart-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let yaml = r#"
docker:
  was_running: true
k8s:
  autostart: true
"#;
        std::fs::write(dir.join("config.yaml"), yaml).unwrap();

        let loaded = MachConfig::load_from_dir(&dir).unwrap();
        assert!(loaded.docker.was_running);
        assert!(loaded.k8s.was_running);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
