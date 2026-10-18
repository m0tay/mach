//! Mach CLI binary entry point.

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
pub mod mdns;
pub mod nfs;
pub mod port_forward;
pub mod proxy;
pub mod ssh;
pub mod tls;

use cli::{Cli, Commands, ConfigAction, DaemonAction, DnsAction, DockerAction, K8sAction, TlsAction};
use mach_common::config::MachConfig;
use mach_common::machine::MachineState;

fn print_mach_help() {
    println!(r#"mach's short "mach" daily-driver command:

1. Interactive Linux Shell
   Run "mach" with no arguments to drop into your default Linux machine:
   Usage: mach

2. Transparent Linux Command Runner
   Prefix any command with "mach" to execute it directly inside Linux:
   Usage: mach [flags] <command> [args...]
   Examples:
     mach uname -a
     mach apt update
     mach python3 -V
     mach -m debian -u root whoami

3. Machine Lifecycle Verbs
   Start, stop, and list your Linux machines:
     mach start [machine]
     mach stop [machine]
     mach restart [machine]
     mach shell [machine]
     mach create [distro]
     mach delete <machine>
     mach list / mach ls
     mach menu

For the full system control plane, Kubernetes, Docker, activity monitor, and diagnostics, use "machctl":
  machctl top            # Live APFS & system monitor
  machctl info <name>    # Detailed machine info
  machctl inspect <name> # Deep inspection of machine specs (YAML/JSON)
  machctl dump <name>    # Dump configuration (YAML/JSON)
  machctl ssh <name>     # Display SSH connection details
  machctl k8s            # Manage local Kubernetes cluster
  machctl doctor         # System diagnostics
  machctl --help         # All control plane commands"#);
}

fn print_machctl_help() {
    println!(r#"mach control plane (machctl) — System administration and management.

Use "machctl" for configuring, monitoring, and managing your mach virtualization environment.
For daily Linux shell and transparent command execution, use "mach".

Usage:
  machctl [command]

Linux machines:
  create      Create a new Linux machine
  list        List all machines (alias: ls)
  start       Start mach or a machine
  stop        Stop mach or a machine
  restart     Restart a machine
  delete      Delete a machine
  info        Get detailed info about a machine
  inspect     Deep inspection of machine specs, network, and features (YAML/JSON)
  dump        Dump raw machine or system configuration (YAML/JSON)
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

Containers:
  k8s         Manage local Kubernetes (K3s) cluster (enable, disable, status)
  docker      Manage Docker CLI integration and legacy sockets
  debug       Debug a Docker container with extra tools

System & Infrastructure:
  top         Live activity and resource monitor (CPU, Memory, APFS CoW disk)
  status      Check whether mach and machines are running
  doctor      Verify hypervisor entitlements, network, and system health
  config      View or modify mach configuration (~/.mach/config.yaml)
  daemon      Manage background DNS (*.mach.local) and port forwarder
  dns         Configure macOS native DNS resolver
  tls         Manage local wildcard TLS certificates (*.mach.local)
  menu        Launch native macOS menu bar status item ( mach)
  setup       Run initial onboarding and environment setup
  version     Show mach version

Flags:
  -h, --help      help for machctl
  -v, --verbose   enable verbose debug logging"#);
}

#[tokio::main]
async fn main() -> Result<()> {
    let raw_args: Vec<String> = std::env::args().collect();
    let bin_name = raw_args
        .get(0)
        .and_then(|p| Path::new(p).file_name())
        .and_then(|f| f.to_str())
        .unwrap_or("mach");

    let is_ctl = bin_name == "machctl";

    // Case 1: Zero arguments
    if raw_args.len() == 1 {
        if is_ctl {
            print_machctl_help();
            return Ok(());
        }

        // Smart 'mach' zero-args behavior:
        let cfg = MachConfig::load()?;

        let machines = commands::load_all_machines(&cfg.machines_dir)?;
        let user_machines: Vec<_> = machines.iter().filter(|m| !m.is_system()).collect();

        if user_machines.is_empty() {
            println!("No user machines found. Welcome to mach! Starting initial environment setup...\n");
            commands::setup::run(true).await?;
            return Ok(());
        }

        let target_name = if let Some(ref def) = cfg.default_machine {
            if user_machines.iter().any(|m| &m.config.name == def) {
                def.clone()
            } else if let Some(running) = user_machines.iter().find(|m| m.state == MachineState::Running) {
                running.config.name.clone()
            } else {
                user_machines[0].config.name.clone()
            }
        } else if let Some(running) = user_machines.iter().find(|m| m.state == MachineState::Running) {
            running.config.name.clone()
        } else {
            user_machines[0].config.name.clone()
        };

        // If stopped, boot it first
        let machine = commands::find_machine(&cfg.machines_dir, &target_name)?.unwrap();
        if machine.state != MachineState::Running {
            commands::start::run(Some(&target_name)).await?;
        } else {
            commands::menu_cmd::ensure_menu_bar_running();
        }

        // Restore services if active in last session
        let _ = commands::restore_services_if_needed(&cfg).await;

        return commands::shell::run(Some(&target_name), false, &[]).await;
    }

    // Custom help handler matching OrbStack
    if raw_args.len() == 2 && (raw_args[1] == "-h" || raw_args[1] == "--help" || raw_args[1] == "help") {
        if !is_ctl {
            print_mach_help();
            return Ok(());
        } else {
            print_machctl_help();
            return Ok(());
        }
    }

    // Case 2: Subcommand routing
    let mut effective_args = raw_args.clone();

    if !is_ctl {
        let first_cmd_idx = raw_args
            .iter()
            .skip(1)
            .position(|arg| arg != "-v" && arg != "--verbose")
            .map(|i| i + 1);

        if let Some(idx) = first_cmd_idx {
            let first_arg = &raw_args[idx];
            // If a user types `mach docker status` or `mach k8s status`, or an administrative action,
            // or if the guest VM is not running: provide clear, polite guidance pointing to machctl.
            if first_arg == "docker" || first_arg == "k8s" {
                let sub = raw_args.get(idx + 1).map(|s| s.as_str());
                let is_admin = match first_arg.as_str() {
                    "docker" => matches!(
                        sub,
                        Some("status")
                            | Some("setup")
                            | Some("start")
                            | Some("stop")
                            | Some("restart")
                            | Some("destroy")
                            | Some("shell")
                            | None
                    ),
                    "k8s" => matches!(
                        sub,
                        Some("status")
                            | Some("enable")
                            | Some("disable")
                            | Some("start")
                            | Some("stop")
                            | Some("restart")
                            | Some("shell")
                            | None
                    ),
                    _ => false,
                };

                let cfg_res = MachConfig::load();
                let vm_not_running = if let Ok(ref cfg) = cfg_res {
                    let vm_name = if first_arg == "docker" {
                        commands::docker::DOCKER_VM_NAME.to_string()
                    } else {
                        commands::k8s::resolve_k8s_machine_name(&cfg.machines_dir)
                            .unwrap_or_else(|_| commands::k8s::K8S_VM_NAME.to_string())
                    };
                    commands::find_machine(&cfg.machines_dir, &vm_name)
                        .ok()
                        .flatten()
                        .map(|m| m.state != MachineState::Running)
                        .unwrap_or(true)
                } else {
                    true
                };

                if is_admin || vm_not_running {
                    println!("In mach, 'machctl' manages the macOS virtualization control plane. Use: machctl docker status (or machctl k8s status). 'mach' executes commands directly inside Linux (e.g. 'mach docker ps').\n");

                    if first_arg == "docker" {
                        match sub {
                            Some("status") | None => return commands::docker::status().await,
                            Some("start") => return commands::docker::start().await,
                            Some("stop") => return commands::docker::stop().await,
                            Some("restart") => return commands::docker::restart().await,
                            Some("setup") => {
                                let system = raw_args.iter().any(|a| a == "--system");
                                return commands::docker::setup(system).await;
                            }
                            Some("destroy") => {
                                let force = raw_args.iter().any(|a| a == "-f" || a == "--force" || a == "-y" || a == "--yes");
                                return commands::docker::destroy(force).await;
                            }
                            Some("shell") => return commands::docker::shell().await,
                            _ => {
                                // Non-admin command (e.g. `mach docker ps` with stopped VM)
                                // falls through to transparent execution in Linux (run_cmd will auto-start)
                            }
                        }
                    } else if first_arg == "k8s" {
                        match sub {
                            Some("status") | None => return commands::k8s::status().await,
                            Some("enable") | Some("start") => return commands::k8s::enable().await,
                            Some("disable") | Some("stop") => return commands::k8s::disable().await,
                            Some("restart") => return commands::k8s::restart().await,
                            Some("shell") => return commands::k8s::shell().await,
                            _ => {
                                // Fall through
                            }
                        }
                    }
                }
            }

            // In mach, control plane administrative commands belong exclusively to machctl.
            // If invoked via mach, provide clean guidance pointing to machctl.
            let ctl_only_commands = [
                "doctor", "inspect", "dump", "top", "config", "daemon", "dns", "tls",
                "mount", "code", "link", "debug", "export", "import", "clone", "rename",
            ];
            if ctl_only_commands.contains(&first_arg.as_str()) {
                eprintln!(
                    "In mach, 'machctl' manages the macOS virtualization control plane.\nUse: machctl {} [options]\n'mach' is reserved for daily Linux shell and command execution (e.g. 'mach uname -a').\n",
                    first_arg
                );
                anyhow::bail!("'{first_arg}' is a control plane command. Run 'machctl {first_arg}' instead.");
            }

            // In `mach`, only basic machine verbs are intercepted locally;
            // everything else (inspect, dump, top, ps, uname, apt...) transparently runs in Linux!
            let mach_machine_commands = [
                "start", "stop", "restart", "shell", "create", "delete", "list", "ls",
                "default", "init", "help", "--help", "-h", "version", "--version", "-V",
                "menu", "run", "setup", "completion", "boot", "run-vm", "update",
                "info", "status", "logs", "ssh", "push", "pull",
            ];

            let cmd_name = &raw_args[idx];
            if !mach_machine_commands.contains(&cmd_name.as_str()) {
                effective_args.insert(idx, "run".to_string());
            }
        }
    } else {
        // In `machctl`, all control plane commands are recognized
        let machctl_commands = [
            "clone", "create", "list", "ls", "start", "stop", "restart", "delete",
            "default", "export", "import", "rename", "shell", "run", "info",
            "inspect", "dump",
            "status", "top", "logs", "ssh", "push", "pull", "docker", "daemon",
            "dns", "doctor", "config", "version", "update", "boot", "run-vm", "debug",
            "mount", "code", "k8s", "link", "tls", "completion", "menu", "setup", "init", "help",
            "--help", "-h", "--version", "-V",
        ];

        let first_cmd_idx = raw_args
            .iter()
            .skip(1)
            .position(|arg| arg != "-v" && arg != "--verbose")
            .map(|i| i + 1);

        if let Some(idx) = first_cmd_idx {
            let cmd_name = &raw_args[idx];
            if !machctl_commands.contains(&cmd_name.as_str()) {
                effective_args.insert(idx, "run".to_string());
            }
        }
    }

    let cli = Cli::parse_from(effective_args);

    // Initialize structured logging
    let log_level = if cli.verbose { "debug" } else { "warn" };
    mach_common::logging::init_logging(log_level);

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
        Commands::List { all, system } => {
            commands::list::run(all, system).await?;
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
        Commands::Stop { name, force, system } => {
            commands::stop::run(name.as_deref(), force, system).await?;
        }
        Commands::Restart { name } => {
            commands::restart::run(name.as_deref()).await?;
        }
        Commands::Default { name } => {
            commands::default_cmd::run(name.as_deref()).await?;
        }
        Commands::Delete { name, yes, system } => {
            commands::delete::run(&name, yes, system).await?;
        }
        Commands::Shell { name, system, command } => {
            commands::shell::run(name.as_deref(), system, &command).await?;
        }
        Commands::Run {
            machine,
            user,
            system,
            command,
        } => {
            commands::run_cmd::run(machine.as_deref(), user.as_deref(), system, &command).await?;
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
                let show_details = details || bin_name == "machctl";
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
        Commands::Inspect { name, yaml, json } => {
            commands::inspect::run(name.as_deref(), yaml, json).await?;
        }
        Commands::Dump { name, all, yaml, json } => {
            commands::dump::run(name.as_deref(), all, yaml, json).await?;
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
            DockerAction::Start => {
                commands::docker::start().await?;
            }
            DockerAction::Stop => {
                commands::docker::stop().await?;
            }
            DockerAction::Restart => {
                commands::docker::restart().await?;
            }
            DockerAction::Shell => {
                commands::docker::shell().await?;
            }
            DockerAction::Destroy { force } => {
                commands::docker::destroy(force).await?;
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
            K8sAction::Start => {
                commands::k8s::start().await?;
            }
            K8sAction::Stop => {
                commands::k8s::stop().await?;
            }
            K8sAction::Restart => {
                commands::k8s::restart().await?;
            }
            K8sAction::Shell => {
                commands::k8s::shell().await?;
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
        Commands::Init { non_interactive } => {
            commands::init::run(non_interactive).await?;
        }
        Commands::Setup { welcome, non_interactive } => {
            if non_interactive {
                commands::init::run(true).await?;
            } else {
                commands::setup::run(welcome).await?;
            }
        }
        Commands::Version => {
            commands::version::run().await?;
        }
        Commands::Update => {
            commands::update::run().await?;
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
