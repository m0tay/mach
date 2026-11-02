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

/// Available mach CLI subcommands.
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
        /// Show all machines including stopped
        #[arg(short, long)]
        all: bool,

        /// Show protected system infrastructure machines (mach-system)
        #[arg(long)]
        system: bool,
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

        /// Working directory in Linux (defaults to shared host working directory when available)
        #[arg(short = 'w', long)]
        workdir: Option<String>,

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

        /// Synchronize mach managed block in ~/.ssh/config
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

    /// Check if mach is properly configured and healthy
    Doctor,

    /// Display detailed information about a machine
    Info {
        /// Machine name
        name: Option<String>,
    },

    /// Inspect machine configuration and runtime status (YAML by default, or JSON)
    Inspect {
        /// Machine name (defaults to default machine)
        name: Option<String>,

        /// Output in YAML format (default)
        #[arg(long)]
        yaml: bool,

        /// Output in JSON format
        #[arg(long)]
        json: bool,
    },

    /// Dump raw machine or system configuration (YAML by default, or JSON)
    Dump {
        /// Machine name (defaults to default machine or all if --all)
        name: Option<String>,

        /// Dump all machines and global configuration
        #[arg(long)]
        all: bool,

        /// Output in YAML format (default)
        #[arg(long)]
        yaml: bool,

        /// Output in JSON format
        #[arg(long)]
        json: bool,
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

    /// View or update mach configuration
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Debug a Docker container with extra diagnostic tools (works on scratch/distroless)
    Debug {
        /// Target container name or ID
        container: String,

        /// Custom diagnostic image to use (defaults to alpine, busybox, or netshoot)
        #[arg(short, long)]
        image: Option<String>,
    },

    /// Mount or open a machine's filesystem in macOS Finder
    Mount {
        /// Machine name
        machine: String,
        /// Target mount directory (defaults to ~/Mach/<machine>)
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

    /// Initialize Mach configuration, keys, Docker context, and run diagnostics (internal/automated)
    #[command(hide = true)]
    Init {
        /// Non-interactive mode (for automated scripts and package managers)
        #[arg(long)]
        non_interactive: bool,
    },

    /// Initial environment setup and onboarding tour (internal/automatic)
    #[command(hide = true)]
    Setup {
        /// Display interactive welcome tour in plain English
        #[arg(long, default_value_t = true)]
        welcome: bool,

        /// Non-interactive mode
        #[arg(long)]
        non_interactive: bool,
    },

    /// Display version and build information
    Version,

    /// Check for and apply Mach updates (Homebrew or binary)
    Update,

    /// Completely uninstall Mach, stop all VMs, and clean up system integration
    Uninstall {
        /// Delete all stored virtual machine disks, images, and ~/.mach data directory
        #[arg(short, long)]
        all: bool,

        /// Non-interactive mode (do not prompt for confirmation)
        #[arg(short, long)]
        force: bool,

        /// Keep stored machine disk images and data directory
        #[arg(long)]
        keep_data: bool,
    },

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

    /// Get a configuration key value
    Get {
        /// Configuration key (e.g. system_memory_mb, default_memory_mb, default_cpus)
        key: String,
    },

    /// Set a configuration key-value pair
    Set {
        /// Configuration key (e.g. default_cpus, default_memory_mb, system_memory_mb, log_level)
        key: String,
        /// New value
        value: String,
    },
}

/// Subcommands for `mach docker`.
#[derive(Subcommand, Debug)]
pub enum DockerAction {
    /// Start Docker engine and configure CLI context
    Start {
        /// Also symlink /var/run/docker.sock for legacy tools & IDEs
        #[arg(long)]
        system: bool,
    },
    /// Stop Docker engine
    Stop,
    /// Restart Docker engine
    Restart,
    /// Show Docker integration status
    Status,
    /// View Docker daemon system logs
    Logs {
        /// Follow log output
        #[arg(short, long)]
        follow: bool,

        /// Number of lines to show
        #[arg(short = 'n', long, default_value = "50")]
        lines: usize,
    },
    /// Open an interactive shell inside the Docker infrastructure machine
    Shell,
    /// Destroy Docker infrastructure machine and its volumes
    Destroy {
        /// Skip confirmation prompt
        #[arg(short, long)]
        force: bool,
    },
    /// Enable and start Docker engine (legacy alias for start)
    #[command(hide = true)]
    Enable {
        /// Also symlink /var/run/docker.sock for legacy tools & IDEs
        #[arg(long)]
        system: bool,
    },
    /// Disable and stop Docker engine (legacy alias for stop)
    #[command(hide = true)]
    Disable,
    /// Configure and ensure macOS docker context is connected to Mach (legacy)
    #[command(hide = true)]
    Setup {
        /// Also symlink /var/run/docker.sock for legacy tools & IDEs
        #[arg(long)]
        system: bool,
    },
}

/// Subcommands for `mach k8s`.
#[derive(Subcommand, Debug)]
pub enum K8sAction {
    /// Enable and start local Kubernetes (K3s) cluster
    Enable,
    /// Disable local Kubernetes cluster
    Disable,
    /// Restart local Kubernetes cluster
    Restart,
    /// Show Kubernetes cluster status
    Status,
    /// View Kubernetes (K3s) daemon system logs
    Logs {
        /// Follow log output
        #[arg(short, long)]
        follow: bool,

        /// Number of lines to show
        #[arg(short = 'n', long, default_value = "50")]
        lines: usize,
    },
    /// Open an interactive shell inside the Kubernetes infrastructure machine
    Shell,
    /// Start local Kubernetes cluster node (legacy alias for enable)
    #[command(hide = true)]
    Start,
    /// Stop local Kubernetes cluster node (legacy alias for disable)
    #[command(hide = true)]
    Stop,
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
    /// Enable macOS native DNS resolver for *.mach.local
    Enable,
    /// Disable macOS native DNS resolver
    Disable,
    /// Show DNS resolver status
    Status,
    /// Configure macOS resolver (/etc/resolver/mach.local) (legacy)
    #[command(hide = true)]
    Setup,
    /// Remove macOS resolver (legacy)
    #[command(hide = true)]
    Remove,
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


/// Public daily-driver surface, shared by help and completion generation.
pub fn mach_command() -> clap::Command {
    use clap::CommandFactory;
    let full = Cli::command();
    let daily = ["start", "stop", "restart", "shell", "create", "delete", "list", "menu", "version", "run"];
    let mut cmd = clap::Command::new("mach")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Linux shell and command runner. Use machctl for administration.")
        .override_usage("mach [OPTIONS] [COMMAND] [ARGS]...")
        .after_help("Run mach with no arguments for a Linux login shell. Other commands run in Linux.
Use mach -- COMMAND to run a guest command that matches a reserved lifecycle verb.
Examples: mach uname -a; mach -m debian -u root -w /tmp pwd; mach docker ps");
    if let Some(run) = full.find_subcommand("run") {
        cmd = cmd.args(run.get_arguments().filter(|arg| matches!(arg.get_id().as_str(), "machine" | "user" | "workdir" | "system")).cloned());
    }
    cmd = cmd.args(full.get_arguments().cloned());
    cmd.subcommands(full.get_subcommands().filter(|sub| daily.contains(&sub.get_name())).cloned())
}

/// Parse only Mach's prefix. Everything from the guest command onward stays untouched.
pub fn route_mach_args(args: &[String]) -> anyhow::Result<Vec<String>> {
    let mut idx = 1;
    let mut guest_options = false;
    while let Some(arg) = args.get(idx) {
        match arg.as_str() {
            "-v" | "--verbose" => idx += 1,
            "-m" | "--machine" | "-u" | "--user" | "-w" | "--workdir" => {
                guest_options = true;
                anyhow::ensure!(args.get(idx + 1).is_some(), "missing value for {arg}");
                idx += 2;
            }
            "--system" => { guest_options = true; idx += 1; }
            "--" => { guest_options = true; break; }
            _ if arg.starts_with("--machine=") || arg.starts_with("--user=") || arg.starts_with("--workdir=")
                || (arg.starts_with("-m") || arg.starts_with("-u") || arg.starts_with("-w")) && arg.len() > 2 => {
                guest_options = true; idx += 1;
            }
            _ => break,
        }
    }
    let first = args.get(idx).map(String::as_str);
    let daily = ["start", "stop", "restart", "shell", "create", "delete", "list", "ls", "menu", "run", "version", "help", "-h", "--help", "-V", "--version", "run-vm"];
    let admin = ["doctor", "inspect", "dump", "top", "config", "daemon", "dns", "tls", "mount", "code", "link", "debug", "export", "import", "clone", "rename", "uninstall", "info", "status", "logs", "ssh", "push", "pull", "default", "k8s", "update", "completion", "init", "setup"];
    if !guest_options {
        if let Some(command) = first {
            anyhow::ensure!(!admin.contains(&command), "'{command}' is a control plane command. Run 'machctl {command}' instead. Use 'mach -- {command}' to execute a Linux command with that name.");
            if daily.contains(&command) { return Ok(args.to_vec()); }
            anyhow::ensure!(!command.starts_with('-'), "unknown Mach option '{command}'; use 'mach -- COMMAND' for a guest command beginning with '-'");
        }
    }
    let mut routed = args.to_vec();
    routed.insert(1, "run".to_string());
    Ok(routed)
}

#[cfg(test)]
mod routing_tests {
    use super::*;
    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(route_mach_args(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap()).unwrap()
    }
    #[test]
    fn guest_arguments_are_not_mach_options() {
        let parsed = parse(&["mach", "-m", "debian", "-u", "nobody", "-w", "/tmp", "printf", "%s", "", "a b", "--help", "-u", "root"]);
        match parsed.command {
            Commands::Run { machine, user, workdir, command, .. } => {
                assert_eq!(machine.as_deref(), Some("debian"));
                assert_eq!(user.as_deref(), Some("nobody"));
                assert_eq!(workdir.as_deref(), Some("/tmp"));
                assert_eq!(command, ["printf", "%s", "", "a b", "--help", "-u", "root"]);
            }
            _ => panic!("expected guest command"),
        }
    }
    #[test]
    fn docker_verbs_and_escape_are_guest_commands() {
        for args in [vec!["mach", "docker", "stop", "container"], vec!["mach", "--", "start", "--", "-x"], vec!["mach", "-m", "debian", "top"]] {
            assert!(matches!(parse(&args).command, Commands::Run { .. }));
        }
        assert!(route_mach_args(&["mach".into(), "inspect".into()]).is_err());
        assert!(route_mach_args(&["mach".into(), "--bad".into()]).is_err());
    }
    #[test]
    fn user_only_opens_selected_user_shell() {
        match parse(&["mach", "-u", "nobody"]).command {
            Commands::Run { user, command, .. } => { assert_eq!(user.as_deref(), Some("nobody")); assert!(command.is_empty()); }
            _ => panic!("expected guest shell"),
        }
    }
}
