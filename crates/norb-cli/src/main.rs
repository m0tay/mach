//! Norb CLI binary entry point.

use anyhow::Result;
use clap::Parser;
use std::path::Path;

mod cli;
pub mod bridge;
pub mod cloud_init;
mod commands;
pub mod dns;
pub mod images;
pub mod menu;
pub mod nfs;
pub mod port_forward;
pub mod proxy;
pub mod ssh;
pub mod tls;

use cli::{Cli, Commands, ConfigAction, DaemonAction, DnsAction, DockerAction, K8sAction, TlsAction};
use norb_common::config::NorbConfig;
use norb_common::machine::MachineState;

fn print_norb_help() {
    println!(r#"Norb's short "norb" daily-driver command:

1. Interactive Linux Shell
   Run "norb" with no arguments to drop into your default Linux machine:
   Usage: norb

2. Transparent Linux Command Runner
   Prefix any command with "norb" to execute it directly inside Linux:
   Usage: norb [flags] <command> [args...]
   Examples:
     norb uname -a
     norb apt update
     norb python3 -V
     norb -m debian -u root whoami

3. Machine Lifecycle Verbs
   Start, stop, and list your Linux machines:
     norb start [machine]
     norb stop [machine]
     norb restart [machine]
     norb shell [machine]
     norb create [distro]
     norb delete <machine>
     norb list / norb ls
     norb menu

For the full system control plane, Kubernetes, Docker, activity monitor, and diagnostics, use "norbctl":
  norbctl top         # Live APFS & system monitor
  norbctl info <name> # Detailed machine info
  norbctl ssh <name>  # Display SSH connection details
  norbctl k8s         # Manage local Kubernetes cluster
  norbctl doctor      # System diagnostics
  norbctl --help      # All control plane commands"#);
}

fn print_norbctl_help() {
    println!(r#"Norb Control Plane (norbctl) — System administration and management.

Use "norbctl" for configuring, monitoring, and managing your Norb virtualization environment.
For daily Linux shell and transparent command execution, use "norb".

Usage:
  norbctl [command]

Machine Management:
  create      Create a new Linux machine
  list        List all machines (alias: ls)
  start       Start Norb or a machine
  stop        Stop Norb or a machine
  restart     Restart a machine
  delete      Delete a machine
  info        Get detailed info about a machine
  default     Get or set the default machine
  clone       Clone a machine (APFS instant clone)
  rename      Rename a machine
  export      Export machine disk to a file
  import      Import machine from a raw disk file
  mount       Mount or open machine filesystem in macOS Finder
  code        Open VS Code connected to a machine
  logs        Show boot and system logs for a machine
  push        Copy files to Linux
  pull        Copy files from Linux
  ssh         Show SSH connection details (host, port, keys)

Containers & Kubernetes:
  k8s         Manage local Kubernetes (K3s) cluster (enable, disable, status)
  docker      Manage Docker CLI integration and legacy sockets
  debug       Debug a Docker container with extra tools

System & Infrastructure:
  top         Live activity and resource monitor (CPU, Memory, APFS CoW disk)
  status      Check whether Norb and machines are running
  doctor      Verify hypervisor entitlements, network, and system health
  config      View or modify Norb configuration (~/.norb/config.yaml)
  daemon      Manage background DNS (*.norb.local) and port forwarder
  dns         Configure macOS native DNS resolver
  tls         Manage local wildcard TLS certificates (*.norb.local)
  menu        Launch native macOS menu bar status item ( Norb)
  setup       Run initial onboarding and environment setup
  version     Show Norb version

Flags:
  -h, --help      help for norbctl
  -v, --verbose   enable verbose debug logging"#);
}

#[tokio::main]
async fn main() -> Result<()> {
    let raw_args: Vec<String> = std::env::args().collect();
    let bin_name = raw_args
        .get(0)
        .and_then(|p| Path::new(p).file_name())
        .and_then(|f| f.to_str())
        .unwrap_or("norb");

    // Case 1: Zero arguments
    if raw_args.len() == 1 {
        if bin_name == "norbctl" {
            print_norbctl_help();
            return Ok(());
        }

        // Smart 'norb' zero-args behavior:
        let cfg = NorbConfig::load()?;
        commands::menu_cmd::ensure_menu_bar_running();

        let machines = commands::load_all_machines(&cfg.machines_dir)?;
        if machines.is_empty() {
            println!("No machines found. Welcome to Norb! Starting initial environment setup...\n");
            commands::setup::run(true).await?;
            return Ok(());
        }

        let target_name = if let Some(ref def) = cfg.default_machine {
            if machines.iter().any(|m| &m.config.name == def) {
                def.clone()
            } else if let Some(running) = machines.iter().find(|m| m.state == MachineState::Running) {
                running.config.name.clone()
            } else {
                machines[0].config.name.clone()
            }
        } else if let Some(running) = machines.iter().find(|m| m.state == MachineState::Running) {
            running.config.name.clone()
        } else {
            machines[0].config.name.clone()
        };

        // If stopped, boot it first
        let machine = commands::find_machine(&cfg.machines_dir, &target_name)?.unwrap();
        if machine.state != MachineState::Running {
            commands::start::run(Some(&target_name)).await?;
        }

        return commands::shell::run(Some(&target_name), &[]).await;
    }

    // Custom help handler matching OrbStack
    if raw_args.len() == 2 && (raw_args[1] == "-h" || raw_args[1] == "--help" || raw_args[1] == "help") {
        if bin_name == "norb" {
            print_norb_help();
            return Ok(());
        } else {
            print_norbctl_help();
            return Ok(());
        }
    }

    // Case 2: Subcommand routing
    let mut effective_args = raw_args.clone();
    let first_arg = &raw_args[1];

    if bin_name == "norb" {
        // In `norb`, only basic machine verbs are intercepted locally;
        // everything else (info, top, logs, ps, uname, apt...) transparently runs in Linux!
        let norb_machine_commands = [
            "start", "stop", "restart", "shell", "create", "delete", "list", "ls",
            "menu", "run", "setup", "completion", "boot", "run-vm", "help", "--help", "-h", "--version", "-V",
        ];

        if !norb_machine_commands.contains(&first_arg.as_str())
            && first_arg != "-v"
            && first_arg != "--verbose"
            && !first_arg.starts_with("-m")
            && !first_arg.starts_with("--machine")
            && !first_arg.starts_with("-u")
            && !first_arg.starts_with("--user")
        {
            effective_args.insert(1, "run".to_string());
        }
    } else {
        // In `norbctl`, all control plane commands are recognized
        let norbctl_commands = [
            "clone", "create", "list", "ls", "start", "stop", "restart", "delete",
            "default", "export", "import", "rename", "shell", "run", "info",
            "status", "top", "logs", "ssh", "push", "pull", "docker", "daemon",
            "dns", "doctor", "config", "version", "boot", "run-vm", "debug",
            "mount", "code", "k8s", "link", "tls", "completion", "menu", "setup", "help",
            "--help", "-h", "--version", "-V",
        ];

        if !norbctl_commands.contains(&first_arg.as_str())
            && first_arg != "-v"
            && first_arg != "--verbose"
        {
            effective_args.insert(1, "run".to_string());
        }
    }

    let cli = Cli::parse_from(effective_args);

    // Initialize structured logging
    let log_level = if cli.verbose { "debug" } else { "warn" };
    norb_common::logging::init_logging(log_level);

    match cli.command {
        Commands::Create {
            distro,
            name,
            cpus,
            memory,
            disk,
            arch,
            isolated: _,
        } => {
            commands::create::run(&distro, name.as_deref(), cpus, memory, disk, &arch).await?;
        }
        Commands::List => {
            commands::list::run().await?;
        }
        Commands::Clone { source, target } => {
            commands::clone_cmd::run(&source, &target).await?;
        }
        Commands::Rename { old, new } => {
            commands::rename_cmd::run(&old, &new).await?;
        }
        Commands::Export { machine, file } => {
            commands::export_import::export(&machine, &file).await?;
        }
        Commands::Import { file, name } => {
            commands::export_import::import(&file, &name).await?;
        }
        Commands::Start { name } => {
            commands::start::run(name.as_deref()).await?;
        }
        Commands::Stop { name, force } => {
            commands::stop::run(name.as_deref(), force).await?;
        }
        Commands::Restart { name } => {
            commands::restart::run(name.as_deref()).await?;
        }
        Commands::Default { name } => {
            commands::default_cmd::run(name.as_deref()).await?;
        }
        Commands::Delete { name, yes } => {
            commands::delete::run(&name, yes).await?;
        }
        Commands::Shell { name, command } => {
            commands::shell::run(name.as_deref(), &command).await?;
        }
        Commands::Run {
            machine,
            user,
            command,
        } => {
            commands::run_cmd::run(machine.as_deref(), user.as_deref(), &command).await?;
        }
        Commands::Ssh {
            name,
            details,
            sync,
            args,
        } => {
            if sync {
                commands::ssh::sync().await?;
            } else {
                let show_details = details || bin_name == "norbctl";
                commands::ssh::run(name.as_deref(), &args, show_details).await?;
            }
        }
        Commands::Logs {
            name,
            follow,
            lines,
        } => {
            commands::logs::run(name.as_deref(), follow, lines).await?;
        }
        Commands::Push { machine, src, dest } => {
            commands::push_pull::push(machine.as_deref(), &src, &dest).await?;
        }
        Commands::Pull { machine, src, dest } => {
            commands::push_pull::pull(machine.as_deref(), &src, &dest).await?;
        }
        Commands::Daemon { action } => match action {
            DaemonAction::Start => {
                commands::daemon::start_daemon()?;
            }
            DaemonAction::Stop => {
                commands::daemon::stop_daemon()?;
            }
            DaemonAction::Status => {
                commands::daemon::status_daemon()?;
            }
            DaemonAction::Run => {
                commands::daemon::run_foreground().await?;
            }
        },
        Commands::Dns { action } => match action {
            DnsAction::Setup => {
                commands::dns_cmd::setup()?;
            }
            DnsAction::Status => {
                commands::dns_cmd::status()?;
            }
        },
        Commands::Doctor => {
            commands::doctor::run().await?;
        }
        Commands::Info { name } => {
            commands::info::run(name.as_deref()).await?;
        }
        Commands::Status => {
            commands::status::run().await?;
        }
        Commands::Top { interval } => {
            commands::top::run(interval).await?;
        }
        Commands::Boot {
            kernel,
            initrd,
            cmdline,
            memory,
            cpus,
        } => {
            commands::boot::run(&kernel, initrd.as_deref(), &cmdline, memory, cpus).await?;
        }
        Commands::Config { action } => match action {
            ConfigAction::Show => {
                commands::config::show().await?;
            }
            ConfigAction::Set { key, value } => {
                commands::config::set(&key, &value).await?;
            }
        },
        Commands::Docker { action } => match action {
            DockerAction::Setup { system } => {
                commands::docker::setup(system).await?;
            }
            DockerAction::Status => {
                commands::docker::status().await?;
            }
        },
        Commands::Debug { container } => {
            commands::debug::run(&container).await?;
        }
        Commands::Mount { machine, target } => {
            commands::mount::run_mount(&machine, target.as_deref()).await?;
        }
        Commands::Code { machine, path } => {
            commands::mount::run_code(&machine, path.as_deref()).await?;
        }
        Commands::K8s { action } => match action {
            K8sAction::Enable => {
                commands::k8s::enable().await?;
            }
            K8sAction::Disable => {
                commands::k8s::disable().await?;
            }
            K8sAction::Status => {
                commands::k8s::status().await?;
            }
        },
        Commands::Link { binary, machine } => {
            commands::link::run(&binary, machine.as_deref()).await?;
        }
        Commands::Tls { action } => match action {
            TlsAction::Install { system } => {
                commands::tls_cmd::install(system).await?;
            }
            TlsAction::Uninstall => {
                commands::tls_cmd::uninstall().await?;
            }
            TlsAction::Status => {
                commands::tls_cmd::status().await?;
            }
        },
        Commands::Completion { shell, install } => {
            commands::completion::generate_completion(&shell, install)?;
        }
        Commands::Menu => {
            commands::menu_cmd::run().await?;
        }
        Commands::Setup { welcome } => {
            commands::setup::run(welcome).await?;
        }
        Commands::Version => {
            commands::version::run().await?;
        }
        Commands::RunVm {
            name,
            kernel,
            initrd,
            disk,
            cidata,
            shared_dir,
            cpus,
            memory,
            mac,
            isolated: _,
        } => {
            commands::run_vm::run(
                &name,
                kernel.as_deref(),
                initrd.as_deref(),
                disk.as_deref(),
                cidata.as_deref(),
                shared_dir.as_deref(),
                cpus,
                memory,
                &mac,
            )?;
        }
    }

    Ok(())
}
