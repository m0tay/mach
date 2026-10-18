//! `mach docker` command implementation for dedicated Docker infrastructure.

use anyhow::Result;
use mach_common::config::MachConfig;
use mach_common::machine::{Machine, MachineState};
use std::path::Path;
use std::process::Command;

pub const DOCKER_VM_NAME: &str = "mach-docker";

/// Ensure the dedicated Docker infrastructure machine exists.
pub async fn ensure_docker_vm(cfg: &MachConfig) -> Result<Machine> {
    if let Some(m) = super::find_machine(&cfg.machines_dir, DOCKER_VM_NAME)? {
        return Ok(m);
    }

    println!("  \x1b[1;36m➜\x1b[0m Provisioning dedicated Docker infrastructure machine (\x1b[1m{}\x1b[0m)...", DOCKER_VM_NAME);
    crate::commands::create::run_with_isolated("debian", Some(DOCKER_VM_NAME), 2, 2048, 20, "aarch64", false).await?;

    let m = super::find_machine(&cfg.machines_dir, DOCKER_VM_NAME)?
        .ok_or_else(|| anyhow::anyhow!("failed to create Docker infrastructure machine"))?;
    Ok(m)
}

/// Terminate any existing background SSH port-forward tunnel for docker.sock.
pub fn terminate_docker_ssh_tunnel() {
    if let Ok(output) = Command::new("ps").args(["-eo", "pid,command"]).output() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.contains("ssh") && line.contains("docker.sock:/var/run/docker.sock") {
                if let Some(pid_str) = line.split_whitespace().next() {
                    if let Ok(pid) = pid_str.parse::<i32>() {
                        let _ = nix::sys::signal::kill(
                            nix::unistd::Pid::from_raw(pid),
                            nix::sys::signal::Signal::SIGTERM,
                        );
                    }
                }
            }
        }
    }
}

/// Configure and ensure the dedicated Docker environment is running.
pub async fn setup(system: bool) -> Result<()> {
    let cfg = MachConfig::load()?;
    let sock_path = cfg.data_dir.join("run").join("docker.sock");

    // 1. Ensure dedicated mach-docker VM exists and is running
    let docker_vm = ensure_docker_vm(&cfg).await?;
    let mut is_alive = docker_vm.state == MachineState::Running && docker_vm.pid.map(|p| nix::sys::signal::kill(nix::unistd::Pid::from_raw(p as i32), None).is_ok()).unwrap_or(false);
    if is_alive {
        if let Some(ip) = docker_vm.ip_address {
            if !tokio::time::timeout(std::time::Duration::from_millis(1500), tokio::net::TcpStream::connect((ip, 22))).await.map(|r| r.is_ok()).unwrap_or(false) {
                is_alive = false;
            }
        } else {
            is_alive = false;
        }
    }
    if !is_alive {
        println!("  \x1b[1;36m➜\x1b[0m Starting Docker infrastructure machine ({DOCKER_VM_NAME})...");
        crate::commands::start::run(Some(DOCKER_VM_NAME)).await?;
    }

    let running_m = super::find_machine(&cfg.machines_dir, DOCKER_VM_NAME)?
        .ok_or_else(|| anyhow::anyhow!("machine {DOCKER_VM_NAME} not found"))?;
    let ip = running_m
        .ip_address
        .ok_or_else(|| anyhow::anyhow!("machine {DOCKER_VM_NAME} has no IP address"))?;

    // 2. Ensure dockerd is installed and running in the VM
    let base_ssh = crate::ssh::mach_ssh_args();
    let mut check_args = base_ssh.clone();
    check_args.extend([
        "-o".to_string(), "ConnectTimeout=2".to_string(),
        format!("root@{ip}"),
        "test -S /var/run/docker.sock".to_string(),
    ]);

    let is_running = Command::new("ssh")
        .args(&check_args)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !is_running {
        println!("  \x1b[1;36m➜\x1b[0m Initializing Docker engine inside {DOCKER_VM_NAME}...");
        let start_cmd = "which dockerd >/dev/null || (apt-get update -y >/dev/null 2>&1 && apt-get install -y docker.io containerd >/dev/null 2>&1); systemctl enable --now docker >/dev/null 2>&1";
        let mut start_args = base_ssh.clone();
        start_args.extend([
            "-o".to_string(), "ConnectTimeout=120".to_string(),
            format!("root@{ip}"),
            start_cmd.to_string(),
        ]);
        let _ = Command::new("ssh").args(&start_args).status();

        for _ in 0..30 {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            if Command::new("ssh").args(&check_args).status().map(|s| s.success()).unwrap_or(false) {
                break;
            }
        }
    }

    // 3. Establish socket forwarding to ~/.mach/run/docker.sock
    terminate_docker_ssh_tunnel();
    let run_dir = cfg.data_dir.join("run");
    let _ = std::fs::create_dir_all(&run_dir);
    let _ = std::fs::remove_file(&sock_path);

    let mut docker_fwd_args = vec![
        "-f".to_string(),
        "-nNT".to_string(),
        "-L".to_string(),
        format!("{}:/var/run/docker.sock", sock_path.display()),
    ];
    docker_fwd_args.extend(base_ssh);
    docker_fwd_args.extend([
        "-o".to_string(), "ConnectTimeout=5".to_string(),
        format!("root@{ip}"),
    ]);
    let _ = Command::new("ssh").args(&docker_fwd_args).status();

    for _ in 0..30 {
        if sock_path.exists() && tokio::net::UnixStream::connect(&sock_path).await.is_ok() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }

    // 4. Ensure background daemon is running for port forwarder & local domains
    crate::commands::daemon::ensure_daemon_running();

    // 5. Configure Docker context
    println!("\x1b[1;36m➜\x1b[0m Configuring Docker CLI context for mach...");
    let _ = Command::new("docker").args(["context", "rm", "-f", "mach"]).output();

    let output = Command::new("docker")
        .args([
            "context",
            "create",
            "mach",
            "--docker",
            &format!("host=unix://{}", sock_path.display()),
            "--description",
            "mach Docker Engine (Apple Silicon)",
        ])
        .output()?;

    if output.status.success() {
        let _ = Command::new("docker").args(["context", "use", "mach"]).output();
        println!("\x1b[1;32m✓\x1b[0m Docker context \x1b[1mmach\x1b[0m configured and activated!");
        println!("  Endpoint: \x1b[1;34munix://{}\x1b[0m", sock_path.display());
    } else {
        let err = String::from_utf8_lossy(&output.stderr);
        eprintln!("\x1b[1;31m✖\x1b[0m Failed to create Docker context: {}", err.trim());
    }

    if system {
        setup_system_symlink(&sock_path);
    } else {
        let sys_sock = Path::new("/var/run/docker.sock");
        if !sys_sock.exists() {
            println!("\n\x1b[1;33mTip:\x1b[0m For IDE plugins and legacy tools (VS Code Dev Containers, Testcontainers):");
            println!("  \x1b[1;32mmachctl docker setup --system\x1b[0m");
        }
    }

    println!("\nYou can now run standard Docker commands directly:");
    println!("  \x1b[1;32mdocker ps\x1b[0m");
    println!("  \x1b[1;32mdocker run --rm -v $(pwd):/app -p 8080:80 nginx:alpine\x1b[0m");

    // Record in Mach settings that Docker was running
    if let Ok(mut c) = MachConfig::load() {
        c.docker.was_running = true;
        let _ = c.save();
    }

    Ok(())
}

/// Start Docker engine
pub async fn start() -> Result<()> {
    setup(false).await
}

/// Stop Docker engine
pub async fn stop() -> Result<()> {
    let mut cfg = MachConfig::load()?;
    cfg.docker.was_running = false;
    let _ = cfg.save();

    println!("  \x1b[1;36m➜\x1b[0m Stopping Docker engine and terminating socket forwarder...");
    terminate_docker_ssh_tunnel();
    let sock_path = cfg.data_dir.join("run").join("docker.sock");
    let _ = std::fs::remove_file(&sock_path);

    if super::find_machine(&cfg.machines_dir, DOCKER_VM_NAME)?.is_some() {
        crate::commands::stop::run(Some(DOCKER_VM_NAME), false, true).await?;
        println!("\x1b[1;32m✓\x1b[0m Docker infrastructure machine stopped.");
    } else {
        println!("Docker engine is not currently initialized.");
    }
    Ok(())
}

/// Restart Docker engine
pub async fn restart() -> Result<()> {
    stop().await?;
    start().await
}

/// Open an interactive shell inside the Docker infrastructure machine
pub async fn shell() -> Result<()> {
    let cfg = MachConfig::load()?;
    if super::find_machine(&cfg.machines_dir, DOCKER_VM_NAME)?.is_none() {
        setup(false).await?;
    }
    println!("\x1b[1;36m➜\x1b[0m Connecting to Docker infrastructure machine (\x1b[1m{}\x1b[0m)...", DOCKER_VM_NAME);
    super::shell::run(Some(DOCKER_VM_NAME), true, &[]).await
}

/// Destroy the Docker infrastructure machine and its container volumes
pub async fn destroy(force: bool) -> Result<()> {
    if let Ok(mut c) = MachConfig::load() {
        c.docker.was_running = false;
        let _ = c.save();
    }
    let cfg = MachConfig::load()?;
    if super::find_machine(&cfg.machines_dir, DOCKER_VM_NAME)?.is_none() {
        println!("Docker infrastructure machine does not exist.");
        return Ok(());
    }

    println!("\x1b[1;33m⚠\x1b[0m Destroying Docker infrastructure machine ({DOCKER_VM_NAME}) and all stored container data...");
    stop().await?;
    super::delete::run(DOCKER_VM_NAME, force, true).await?;
    println!("\x1b[1;32m✓\x1b[0m Docker infrastructure machine destroyed.");
    Ok(())
}

pub fn setup_system_symlink(sock_path: &Path) {
    let sys_sock = Path::new("/var/run/docker.sock");
    if sys_sock.is_symlink() {
        if let Ok(target) = std::fs::read_link(sys_sock) {
            if target == sock_path {
                println!("\x1b[1;32m✓\x1b[0m System /var/run/docker.sock is already linked to mach.");
                return;
            }
        }
    }

    println!("\x1b[1;36m➜\x1b[0m Linking /var/run/docker.sock to {}...", sock_path.display());
    let res = Command::new("sudo")
        .args(["ln", "-sf", sock_path.to_str().unwrap(), "/var/run/docker.sock"])
        .status();

    if let Ok(st) = res {
        if st.success() {
            println!("\x1b[1;32m✓\x1b[0m Successfully linked /var/run/docker.sock!");
            return;
        }
    }

    println!("\x1b[1;33m!\x1b[0m Could not create symlink automatically. Run manually:\n");
    println!("  \x1b[1;32msudo ln -sf {} /var/run/docker.sock\x1b[0m\n", sock_path.display());
}

pub async fn status() -> Result<()> {
    let cfg = MachConfig::load()?;
    let sock_path = cfg.data_dir.join("run").join("docker.sock");
    let docker_vm = super::find_machine(&cfg.machines_dir, DOCKER_VM_NAME)?;

    println!("\x1b[1;36m➜\x1b[0m Checking mach Docker status...");

    // Check VM state FIRST
    let is_running = match docker_vm.as_ref() {
        Some(m) => m.state == MachineState::Running,
        None => false,
    };

    if !is_running {
        // Remove stale socket file if present
        if sock_path.exists() {
            let _ = std::fs::remove_file(&sock_path);
        }
        terminate_docker_ssh_tunnel();

        println!("  Infrastructure VM: \x1b[1;33m○ stopped\x1b[0m");
        println!("  Socket path:       \x1b[1m{}\x1b[0m", sock_path.display());
        println!("  Socket file:       \x1b[1;31m✖ not found\x1b[0m");
        println!("  Engine status:     \x1b[1;33m○ stopped (run 'machctl docker start' to start)\x1b[0m");
        return Ok(());
    }

    let m = docker_vm.unwrap();
    println!("  Infrastructure VM: \x1b[1;32m● {}\x1b[0m (running)", m.config.name);
    println!("  Socket path:       \x1b[1m{}\x1b[0m", sock_path.display());

    if sock_path.exists() {
        println!("  Socket file:       \x1b[1;32m● present\x1b[0m");
        let responsive = tokio::time::timeout(
            std::time::Duration::from_millis(400),
            tokio::net::UnixStream::connect(&sock_path),
        )
        .await
        .map(|res| res.is_ok())
        .unwrap_or(false);

        if responsive {
            let output = Command::new("docker")
                .args(["--context", "mach", "version", "--format", "{{.Server.Version}}"])
                .output();
            if let Ok(out) = output {
                if out.status.success() {
                    let ver = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    println!("  Engine status:     \x1b[1;32m● connected\x1b[0m (version: {ver})");
                    return Ok(());
                }
            }
        }
        println!("  Engine status:     \x1b[1;33m○ socket exists but daemon not responding\x1b[0m");
    } else {
        println!("  Socket file:       \x1b[1;31m✖ not found\x1b[0m");
        println!("  Engine status:     \x1b[1;33m○ stopped (run 'machctl docker start' to start)\x1b[0m");
    }

    Ok(())
}

