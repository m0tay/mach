//! CLI definitions and argument parsing using `clap`.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// Root command-line structure for `mach`.
#[derive(Parser, Debug)]
#[command(
    name = "mach",
    about = "A fast, lightweight macOS VM and container manager in Rust",
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
        #[arg(default_value = "ubuntu")]
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

        /// Run in isolated sandbox mode (disables /Users file sharing and host bridge)
        #[arg(long)]
        isolated: bool,
    },

    /// List all machines
    #[command(alias = "ls")]
    List {
        /// Show all machines including system infrastructure machines
        #[arg(short, long)]
        all: bool,
    },

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
        /// Also stop protected system infrastructure machines
        #[arg(long)]
        system: bool,
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
        /// Allow deleting protected system infrastructure machines
        #[arg(long)]
        system: bool,
    },

    /// Open an interactive shell in a machine
    Shell {
        /// Machine name
        name: Option<String>,

        /// Allow connecting to protected system infrastructure machines
        #[arg(long)]
        system: bool,

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

        /// Allow running on protected system infrastructure machines
        #[arg(long)]
        system: bool,

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

        /// Synchronize Norb managed block in ~/.ssh/config
        #[arg(long)]
        sync: bool,

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

    /// Configure or show macOS local DNS (*.mach.local)
    Dns {
        #[command(subcommand)]
        action: DnsAction,
    },

    /// Link a host command to run seamlessly inside a Linux machine
    Link {
        /// Host binary name (e.g. gh, aws, op)
        binary: String,
        /// Target machine (defaults to default machine)
        #[arg(short, long)]
        machine: Option<String>,
    },

    /// Manage local wildcard TLS certificate authority (*.mach.local)
    Tls {
        #[command(subcommand)]
        action: TlsAction,
    },

    /// Generate shell autocompletion script (zsh, bash, fish)
    Completion {
        /// Shell type (zsh, bash, fish)
        #[arg(default_value = "zsh")]
        shell: String,
        /// Install autocompletion directly into shell directory
        #[arg(long)]
        install: bool,
    },

    /// Launch minimal Apple menu bar status item
    Menu,

    /// Initial environment setup and onboarding tour
    Setup {
        /// Display interactive welcome tour in plain English
        #[arg(long, default_value_t = true)]
        welcome: bool,
    },

    /// Display version and build information
    Version,

    /// Check for and apply Mach updates (Homebrew or binary)
    Update,

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

        /// Run in isolated sandbox mode (disables /Users file sharing)
        #[arg(long)]
        isolated: bool,
    },
}

/// Subcommands for `mach config`.
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

/// Subcommands for `mach docker`.
#[derive(Subcommand, Debug)]
pub enum DockerAction {
    /// Configure and ensure macOS docker context is connected to Mach
    Setup {
        /// Also symlink /var/run/docker.sock for legacy tools & IDEs
        #[arg(long)]
        system: bool,
    },
    /// Start the dedicated Docker infrastructure machine
    Start,
    /// Stop the dedicated Docker infrastructure machine
    Stop,
    /// Restart the dedicated Docker infrastructure machine
    Restart,
    /// Open an interactive shell inside the Docker infrastructure machine
    Shell,
    /// Destroy the dedicated Docker infrastructure machine and its volumes
    Destroy {
        /// Skip confirmation prompt
        #[arg(short, long)]
        force: bool,
    },
    /// Show Docker integration status
    Status,
}

/// Subcommands for `mach k8s`.
#[derive(Subcommand, Debug)]
pub enum K8sAction {
    /// Enable and start local Kubernetes cluster
    Enable,
    /// Stop local Kubernetes cluster
    Disable,
    /// Start local Kubernetes cluster node
    Start,
    /// Stop local Kubernetes cluster node
    Stop,
    /// Restart local Kubernetes cluster node
    Restart,
    /// Open an interactive shell inside the Kubernetes infrastructure machine
    Shell,
    /// Show Kubernetes cluster status
    Status,
}

/// Subcommands for `mach daemon`.
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

/// Subcommands for `mach dns`.
#[derive(Subcommand, Debug)]
pub enum DnsAction {
    /// Configure or show macOS resolver configuration (/etc/resolver/mach.local)
    Setup,
    /// Show DNS resolver status
    Status,
}

/// Subcommands for `mach tls`.
#[derive(Subcommand, Debug)]
pub enum TlsAction {
    /// Install Mach Root CA into macOS Keychain trust store
    Install {
        /// Install into System keychain instead of user login keychain
        #[arg(long)]
        system: bool,
    },
    /// Uninstall Mach Root CA from macOS Keychain
    Uninstall,
    /// Check TLS CA and certificate status
    Status,
}

