//! `norb docker` command implementation.

use anyhow::Result;
use norb_common::config::NorbConfig;
use std::path::Path;
use std::process::Command;

pub async fn setup(system: bool) -> Result<()> {
    let cfg = NorbConfig::load()?;
    let sock_path = cfg.data_dir.join("run").join("docker.sock");

    let sock_active = if sock_path.exists() {
        tokio::net::UnixStream::connect(&sock_path).await.is_ok()
    } else {
        false
    };

    if !sock_active {
        let _ = std::fs::remove_file(&sock_path);
        // Terminate any stale SSH tunnels forwarding docker.sock
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

        let machines = super::load_all_machines(&cfg.machines_dir)?;
        // Never select norb-k8s for Docker host; select default or non-k8s machine
        let target_machine = machines
            .iter()
            .find(|m| {
                m.config.name != crate::commands::k8s::K8S_VM_NAME
                    && cfg.default_machine.as_deref() == Some(&m.config.name)
            })
            .or_else(|| {
                machines
                    .iter()
                    .find(|m| m.config.name != crate::commands::k8s::K8S_VM_NAME && m.state == norb_common::machine::MachineState::Running)
            })
            .or_else(|| {
                machines
                    .iter()
                    .find(|m| m.config.name != crate::commands::k8s::K8S_VM_NAME)
            });

        if let Some(m) = target_machine {
            if m.state != norb_common::machine::MachineState::Running {
                crate::commands::start::run(Some(&m.config.name)).await?;
            }
            let running_m = super::find_machine(&cfg.machines_dir, &m.config.name)?.unwrap();
            if let Some(ip) = running_m.ip_address {
                let base_ssh = crate::ssh::norb_ssh_args();
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
                    let distro = running_m.config.distro.to_ascii_lowercase();
                    println!("  \x1b[1;36m➜\x1b[0m Ensuring Docker daemon is installed and active in guest {}...", running_m.config.name);
                    let start_cmd = if distro.contains("alpine") {
                        "which dockerd >/dev/null || (apk update >/dev/null 2>&1 && apk add --no-cache docker >/dev/null 2>&1); (pidof dockerd >/dev/null || dockerd --data-root /var/lib/docker >/var/log/dockerd.log 2>&1 &)"
                    } else if distro.contains("debian") || distro.contains("ubuntu") {
                        "which dockerd >/dev/null || (apt-get update -y >/dev/null 2>&1 && apt-get install -y docker.io containerd >/dev/null 2>&1); systemctl enable --now docker >/dev/null 2>&1"
                    } else if distro.contains("fedora") {
                        "which dockerd >/dev/null || (dnf install -y docker-ce moby-engine >/dev/null 2>&1 || dnf install -y docker >/dev/null 2>&1); systemctl enable --now docker >/dev/null 2>&1"
                    } else {
                        "which dockerd >/dev/null || (apt-get update -y >/dev/null 2>&1 && apt-get install -y docker.io >/dev/null 2>&1); systemctl enable --now docker >/dev/null 2>&1 || dockerd &"
                    };

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
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
            }
        }
    }
    crate::commands::daemon::ensure_daemon_running();

    println!("\x1b[1;36m➜\x1b[0m Configuring Docker CLI context for Norb...");

    // Remove existing if present (force remove to handle in-use context)
    let _ = Command::new("docker")
        .args(["context", "rm", "-f", "norb"])
        .output();

    let output = Command::new("docker")
        .args([
            "context",
            "create",
            "norb",
            "--docker",
            &format!("host=unix://{}", sock_path.display()),
            "--description",
            "Norb Linux VM (Apple Silicon)",
        ])
        .output()?;

    if output.status.success() {
        // Switch to norb context
        let _ = Command::new("docker")
            .args(["context", "use", "norb"])
            .output();

        println!("\x1b[1;32m✓\x1b[0m Docker context \x1b[1mnorb\x1b[0m configured and activated!");
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
            println!("\n\x1b[1;33mTip:\x1b[0m To make legacy tools & IDE plugins (VS Code Dev Containers, Testcontainers) work automatically, run:");
            println!("  \x1b[1;32mnorbctl docker setup --system\x1b[0m");
        }
    }

    println!("\nYou can now run standard Docker commands directly:");
    println!("  \x1b[1;32mdocker ps\x1b[0m");
    println!("  \x1b[1;32mdocker run --rm -v $(pwd):/app -p 8080:80 nginx:alpine\x1b[0m");

    Ok(())
}

pub fn setup_system_symlink(sock_path: &Path) {
    let sys_sock = Path::new("/var/run/docker.sock");
    if sys_sock.is_symlink() {
        if let Ok(target) = std::fs::read_link(sys_sock) {
            if target == sock_path {
                println!("\x1b[1;32m✓\x1b[0m System /var/run/docker.sock is already linked to Norb.");
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
    let cfg = NorbConfig::load()?;
    let sock_path = cfg.data_dir.join("run").join("docker.sock");

    println!("\x1b[1;36m➜\x1b[0m Checking Norb Docker status...");
    println!("  Socket path: \x1b[1m{}\x1b[0m", sock_path.display());

    if sock_path.exists() {
        println!("  Socket file: \x1b[1;32m● present\x1b[0m");
        let output = Command::new("docker")
            .args(["--context", "norb", "version", "--format", "{{.Server.Version}}"])
            .output();
        if let Ok(out) = output {
            if out.status.success() {
                let ver = String::from_utf8_lossy(&out.stdout).trim().to_string();
                println!("  Engine status: \x1b[1;32m● connected\x1b[0m (version: {ver})");
                return Ok(());
            }
        }
        println!("  Engine status: \x1b[1;33m○ socket exists but daemon not responding\x1b[0m");
    } else {
        println!("  Socket file: \x1b[1;31m✖ not found\x1b[0m");
        println!("\nStart a machine with \x1b[1;34mnorb start\x1b[0m first.");
    }

    Ok(())
}
