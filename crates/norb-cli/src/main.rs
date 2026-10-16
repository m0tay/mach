//! Norb CLI binary entry point.

use anyhow::Result;
use clap::Parser;
use std::path::Path;

mod cli;
pub mod cloud_init;
mod commands;
pub mod dns;
pub mod images;
pub mod port_forward;

use cli::{Cli, Commands, ConfigAction, DaemonAction, DnsAction, DockerAction};
use norb_common::config::NorbConfig;
use norb_common::machine::MachineState;

fn print_norb_help() {
    println!(r#"Norb's short "norb" command can be used in 3 ways:

1. Start a Linux shell.
   Just run "norb" with no arguments.
   Usage: norb

2. Run commands on Linux, like "norbctl run".
   Prefix any command with "norb" to run it in Linux.
   Usage: norb [flags] <command> [args...]
   Example: norb uname -a

   The default user and machine will be used, unless specified with flags.
   For example, to log in to "debian" as root: norb -m debian -u root uname -a

   Use "norbctl run --help" for a list of flags.
   If you prefer SSH, use "norbctl ssh" for details.

3. Control Linux machines, like "norbctl".
   Start, stop, and manage Norb and its machines with any "norbctl" subcommand.
   Usage: norb <subcommand> [args...]

   Use "norbctl --help" for a list of subcommands.

For Docker containers, use the "docker" command. "norb" is for managing Norb and machines."#);
}

fn print_norbctl_help() {
    println!(r#"Use and manage Norb and its machines.

The listed commands can be used with either "norbctl" or "norb".

You can also prefix commands with "norb" to run them on Linux. For example:
    norb uname -a
will run "uname -a" on macOS, and is equivalent to:
    norbctl run uname -a

In this mode, the default user and machine will be used.

Usage:
  norbctl [command]

Linux machines:
  clone       Clone a machine
  create      Create a new machine
  default     Get or set the default machine
  delete      Delete a machine
  export      Export a machine to a file
  import      Import a machine from a file
  info        Get info about a machine
  list        List machines
  logs        Show logs for a machine
  pull        Copy files from Linux
  push        Copy files to Linux
  rename      Rename a machine
  restart     Restart a machine
  run         Run command on Linux
  ssh         Show SSH details

Containers:
  docker      Extension commands for Docker

General:
  config      Change Norb settings
  daemon      Manage background DNS and port forwarding daemon
  dns         Configure or show macOS local DNS (*.norb.local)
  doctor      Check if Norb is properly configured and in control
  start       Start Norb or a machine
  status      Check whether Norb is running
  stop        Stop Norb or a machine
  top         Show a live activity monitor
  version     Show Norb version

Flags:
  -h, --help      help for norbctl
  -v, --verbose   enable verbose debug logging

Use "norbctl [command] --help" for more information about a command."#);
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
        let machines = commands::load_all_machines(&cfg.machines_dir)?;
        if machines.is_empty() {
            println!("To use Docker:\n    docker run ...\nSee \"norb docker\" for more info.\n");
            println!("To create a Linux machine:\n    norb create debian\nSee \"norb create --help\" for supported distros and options.");
            return Ok(());
        }

        let target_name = if let Some(ref def) = cfg.default_machine {
            def.clone()
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

    // Case 2: Check if args should be transparently forwarded to `run`
    let known_subcommands = [
        "clone", "create", "list", "ls", "start", "stop", "restart", "delete",
        "default", "export", "import", "rename", "shell", "run", "info",
        "status", "top", "logs", "ssh", "push", "pull", "docker", "daemon",
        "dns", "doctor", "config", "version", "boot", "run-vm", "help",
        "--help", "-h", "--version", "-V",
    ];

    let mut effective_args = raw_args.clone();
    let first_arg = &raw_args[1];

    if !known_subcommands.contains(&first_arg.as_str())
        && first_arg != "-v"
        && first_arg != "--verbose"
    {
        // Synthesize `norb run <args...>`
        effective_args.insert(1, "run".to_string());
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
            args,
        } => {
            commands::ssh::run(name.as_deref(), &args, details).await?;
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
            DockerAction::Setup => {
                commands::docker::setup().await?;
            }
            DockerAction::Status => {
                commands::docker::status().await?;
            }
        },
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
