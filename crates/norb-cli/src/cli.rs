//! CLI definitions and argument parsing using `clap`.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// Root command-line structure for `norb`.
#[derive(Parser, Debug)]
#[command(
    name = "norb",
    about = "A fast, lightweight macOS VM and container manager in Rust (OrbStack alternative)",
    version,
    propagate_version = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Enable verbose / debug logging output
    #[arg(short, long, global = true)]
    pub verbose: bool,
}

/// Available Norb CLI subcommands.
#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Create a new Linux machine
    Create {
        /// Linux distribution (e.g. ubuntu, debian, fedora, arch, alpine)
        distro: String,

        /// Machine name (defaults to distro name)
        name: Option<String>,

        /// Number of virtual CPUs
        #[arg(short, long, default_value = "2")]
        cpus: u32,

        /// Memory in mebibytes (MiB)
        #[arg(short, long, default_value = "2048")]
        memory: u64,

        /// Disk size in gibibytes (GiB)
        #[arg(short, long, default_value = "20")]
        disk: u64,

        /// CPU architecture (aarch64, x86_64)
        #[arg(long, default_value = "aarch64")]
        arch: String,
    },

    /// List all machines
    #[command(alias = "ls")]
    List,

    /// Clone an existing machine using APFS Copy-on-Write
    Clone {
        /// Source machine name
        source: String,
        /// Target machine name
        target: String,
    },

    /// Rename an existing machine
    Rename {
        /// Current machine name
        old: String,
        /// New machine name
        new: String,
    },

    /// Export a machine disk to a file
    Export {
        /// Machine name
        machine: String,
        /// Destination file path
        file: PathBuf,
    },

    /// Import a machine from a raw disk file
    Import {
        /// Source disk file path
        file: PathBuf,
        /// Target machine name
        name: String,
    },

    /// Start a machine
    Start {
        /// Machine name (defaults to default machine or first available)
        name: Option<String>,
    },

    /// Stop a running machine
    Stop {
        /// Machine name (defaults to default machine or all running)
        name: Option<String>,
        /// Force stop immediately
        #[arg(short, long)]
        force: bool,
    },

    /// Restart a machine
    Restart {
        /// Machine name (defaults to default machine)
        name: Option<String>,
    },

    /// Get or set the default machine
    Default {
        /// Machine name to set as default (or omit to get current default)
        name: Option<String>,
    },

    /// Delete a machine
    Delete {
        /// Machine name
        name: String,
        /// Skip confirmation prompt
        #[arg(short, long)]
        yes: bool,
    },

    /// Open an interactive shell in a machine
    Shell {
        /// Machine name
        name: Option<String>,

        /// Command to run instead of interactive shell
        #[arg(last = true)]
        command: Vec<String>,
    },

    /// Run a command inside a machine (like orbctl run)
    Run {
        /// Target machine (defaults to default machine)
        #[arg(short, long)]
        machine: Option<String>,

        /// User to run command as (defaults to root)
        #[arg(short, long)]
        user: Option<String>,

        /// Command and arguments to execute
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },

    /// Connect to a machine via SSH or show SSH connection details
    Ssh {
        /// Machine name (defaults to default machine)
        name: Option<String>,

        /// Show SSH connection details instead of connecting
        #[arg(short, long)]
        details: bool,

        /// Additional arguments to pass to ssh
        #[arg(last = true)]
        args: Vec<String>,
    },

    /// Show boot and system logs for a machine
    Logs {
        /// Machine name (defaults to default machine)
        name: Option<String>,

        /// Follow log output
        #[arg(short, long)]
        follow: bool,

        /// Number of lines to show
        #[arg(short = 'n', long, default_value = "50")]
        lines: usize,
    },

    /// Copy files to Linux machine
    Push {
        /// Target machine
        #[arg(short, long)]
        machine: Option<String>,

        /// Source path on macOS
        src: String,

        /// Destination path on Linux
        dest: String,
    },

    /// Copy files from Linux machine
    Pull {
        /// Target machine
        #[arg(short, long)]
        machine: Option<String>,

        /// Source path on Linux
        src: String,

        /// Destination path on macOS
        dest: String,
    },

    /// Check if Norb is properly configured and healthy
    Doctor,

    /// Display detailed information about a machine
    Info {
        /// Machine name
        name: Option<String>,
    },

    /// Show overall status of machines and the virtualization engine
    Status,

    /// Live terminal resource monitor (CLI-first)
    Top {
        /// Refresh interval in seconds
        #[arg(short, long, default_value = "1")]
        interval: u64,
    },

    /// Manage Docker engine integration (macOS context and status)
    Docker {
        #[command(subcommand)]
        action: DockerAction,
    },

    /// Boot a Linux kernel in a minimal VM (low-level Hypervisor.framework test)
    Boot {
        /// Path to Linux kernel Image / vmlinux
        #[arg(long)]
        kernel: PathBuf,

        /// Optional path to initrd
        #[arg(long)]
        initrd: Option<PathBuf>,

        /// Kernel command line arguments
        #[arg(long, default_value = "console=hvc0 earlycon=pl011,0x09000000 root=/dev/vda rw")]
        cmdline: String,

        /// Memory in mebibytes (MiB)
        #[arg(long, default_value = "512")]
        memory: u64,

        /// Number of virtual CPUs
        #[arg(long, default_value = "1")]
        cpus: u32,
    },

    /// View or update Norb configuration
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Debug a Docker container with extra diagnostic tools (works on scratch/distroless)
    Debug {
        /// Target container name or ID
        container: String,
    },

    /// Mount or open a machine's filesystem in macOS Finder
    Mount {
        /// Machine name
        machine: String,
        /// Target mount directory (defaults to ~/Norb/<machine>)
        target: Option<PathBuf>,
    },

    /// Open VS Code connected to a machine via SSH
    Code {
        /// Machine name
        machine: String,
        /// Optional directory path in machine (defaults to /root)
        path: Option<String>,
    },

    /// Manage local Kubernetes (K3s) cluster
    K8s {
        #[command(subcommand)]
        action: K8sAction,
    },

    /// Manage background DNS and port forwarding daemon
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },

    /// Configure or show macOS local DNS (*.norb.local)
    Dns {
        #[command(subcommand)]
        action: DnsAction,
    },

    /// Display version and build information
    Version,

    /// Internal command to execute the virtual machine process in the background
    #[command(hide = true)]
    RunVm {
        /// Machine name
        #[arg(long)]
        name: String,

        /// Kernel image path (optional; omitted for UEFI cloud image boot)
        #[arg(long)]
        kernel: Option<PathBuf>,

        /// Initramfs path
        #[arg(long)]
        initrd: Option<PathBuf>,

        /// Persistent root disk image path
        #[arg(long)]
        disk: Option<PathBuf>,

        /// Optional cloud-init seed disk image
        #[arg(long)]
        cidata: Option<PathBuf>,

        /// Shared host directory path for VirtioFS
        #[arg(long)]
        shared_dir: Option<PathBuf>,

        /// Virtual CPU count
        #[arg(long)]
        cpus: u32,

        /// Memory in MiB
        #[arg(long)]
        memory: u64,

        /// MAC address for the guest NIC
        #[arg(long)]
        mac: String,
    },
}

/// Subcommands for `norb config`.
#[derive(Subcommand, Debug)]
pub enum ConfigAction {
    /// Show current configuration
    Show,

    /// Set a configuration key-value pair
    Set {
        /// Configuration key (e.g. default_cpus, default_memory_mb, log_level)
        key: String,
        /// New value
        value: String,
    },
}

/// Subcommands for `norb docker`.
#[derive(Subcommand, Debug)]
pub enum DockerAction {
    /// Configure macOS docker context to connect to Norb
    Setup {
        /// Also symlink /var/run/docker.sock for legacy tools & IDEs
        #[arg(long)]
        system: bool,
    },
    /// Show Docker integration status
    Status,
}

/// Subcommands for `norb k8s`.
#[derive(Subcommand, Debug)]
pub enum K8sAction {
    /// Enable and start local Kubernetes cluster
    Enable,
    /// Stop local Kubernetes cluster
    Disable,
    /// Show Kubernetes cluster status
    Status,
}

/// Subcommands for `norb daemon`.
#[derive(Subcommand, Debug)]
pub enum DaemonAction {
    /// Start the background daemon
    Start,
    /// Stop the background daemon
    Stop,
    /// Show background daemon status
    Status,
    /// Run daemon in the foreground (internal)
    #[command(hide = true)]
    Run,
}

/// Subcommands for `norb dns`.
#[derive(Subcommand, Debug)]
pub enum DnsAction {
    /// Configure or show macOS resolver configuration (/etc/resolver/norb.local)
    Setup,
    /// Show DNS resolver status
    Status,
}

