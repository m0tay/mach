//! Kubernetes (K3s) integration for Mach.

use anyhow::{bail, Context, Result};
use mach_common::config::MachConfig;
use mach_common::machine::MachineState;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use crate::commands::find_machine;

pub const K8S_VM_NAME: &str = "mach-kubernetes";
pub const ALT_K8S_VM_NAME: &str = "mach-k8s";

pub fn resolve_k8s_machine_name(machines_dir: &std::path::Path) -> Result<String> {
    if find_machine(machines_dir, K8S_VM_NAME)?.is_some() {
        return Ok(K8S_VM_NAME.to_string());
    }
    if find_machine(machines_dir, ALT_K8S_VM_NAME)?.is_some() {
        return Ok(ALT_K8S_VM_NAME.to_string());
    }
    Ok(K8S_VM_NAME.to_string())
}

/// Terminate any existing background SSH port-forward tunnel for Kubernetes API (26443).
pub fn terminate_k8s_ssh_tunnel() {
    let mut killed = false;
    if let Ok(output) = Command::new("ps").args(["-eo", "pid,command"]).output() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.contains("ssh") && line.contains("26443") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if let Some(pid_str) = parts.first() {
                    if let Ok(pid) = pid_str.parse::<i32>() {
                        let _ = nix::sys::signal::kill(
                            nix::unistd::Pid::from_raw(pid),
                            nix::sys::signal::Signal::SIGTERM,
                        );
                        killed = true;
                    }
                }
            }
        }
    }
    if killed {
        std::thread::sleep(Duration::from_millis(150));
    }
}

/// Wait for SSH service to become ready with exponential backoff for up to 30s.
async fn wait_for_ssh_ready(ip: &std::net::IpAddr) -> Result<()> {
    let start = Instant::now();
    let timeout = Duration::from_secs(30);
    let mut backoff = Duration::from_millis(200);

    println!("  \x1b[1;36m➜\x1b[0m Waiting for SSH service to become ready on {ip}...");

    while start.elapsed() < timeout {
        let addr = std::net::SocketAddr::new(*ip, 22);

        // 1. Poll TCP port 22
        let port_open = tokio::time::timeout(
            Duration::from_millis(500),
            tokio::net::TcpStream::connect(addr),
        )
        .await
        .map(|r| r.is_ok())
        .unwrap_or(false);

        if port_open {
            // 2. Test SSH echo
            let mut cmd = Command::new("ssh");
            cmd.args(crate::ssh::mach_ssh_args());
            cmd.args([
                "-o", "ConnectTimeout=2",
                &format!("root@{ip}"),
                "echo ok",
            ]);

            if let Ok(output) = cmd.output() {
                if output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "ok" {
                    return Ok(());
                }
            }
        }

        tokio::time::sleep(backoff).await;
        backoff = std::cmp::min(backoff * 2, Duration::from_millis(2000));
    }

    bail!("Timed out after 30s waiting for SSH connection on {ip}:22");
}

/// Enable or start the local Kubernetes cluster.
pub async fn enable() -> Result<()> {
    let cfg = MachConfig::load()?;

    println!("\x1b[1;36m➜\x1b[0m Checking mach Kubernetes cluster...");

    let vm_name = resolve_k8s_machine_name(&cfg.machines_dir)?;
    let machine = find_machine(&cfg.machines_dir, &vm_name)?;

    if machine.is_none() {
        println!("  Creating dedicated Kubernetes node \x1b[1m{}\x1b[0m...", vm_name);
        crate::commands::create::run(
            "debian",
            Some(&vm_name),
            2,
            3072,
            20,
            "aarch64",
        )
        .await?;
    }

    let m = find_machine(&cfg.machines_dir, &vm_name)?.unwrap();
    let mut is_alive = m.state == MachineState::Running && m.pid.map(|p| nix::sys::signal::kill(nix::unistd::Pid::from_raw(p as i32), None).is_ok()).unwrap_or(false);
    if is_alive {
        if let Some(ip) = m.ip_address {
            if !tokio::time::timeout(std::time::Duration::from_millis(1500), tokio::net::TcpStream::connect((ip, 22))).await.map(|r| r.is_ok()).unwrap_or(false) {
                is_alive = false;
            }
        } else {
            is_alive = false;
        }
    }
    if !is_alive {
        crate::commands::start::run(Some(&vm_name)).await?;
    }

    let m = find_machine(&cfg.machines_dir, &vm_name)?.unwrap();
    let ip = m.ip_address.context("Kubernetes VM has no IP address")?;

    // Wait for SSH to be completely ready with exponential backoff
    wait_for_ssh_ready(&ip).await?;

    println!("  \x1b[1;36m➜\x1b[0m Installing and configuring K3s inside {}...", vm_name);

    // Install k3s if not present
    let mut check_k3s_cmd = Command::new("ssh");
    check_k3s_cmd.args(crate::ssh::mach_ssh_args());
    check_k3s_cmd.args([
        &format!("root@{ip}"),
        "which k3s >/dev/null 2>&1 || curl -sfL https://get.k3s.io | sh -s - --disable traefik",
    ]);
    let check_k3s = check_k3s_cmd.status()?;

    if !check_k3s.success() {
        bail!("Failed to initialize K3s inside Kubernetes VM");
    }

    // Retrieve kubeconfig
    println!("  \x1b[1;36m➜\x1b[0m Fetching cluster kubeconfig...");
    let mut get_conf_output = None;
    for _ in 0..30 {
        let mut get_conf_cmd = Command::new("ssh");
        get_conf_cmd.args(crate::ssh::mach_ssh_args());
        get_conf_cmd.args([
            "-o", "ConnectTimeout=2",
            &format!("root@{ip}"),
            "cat /etc/rancher/k3s/k3s.yaml 2>/dev/null",
        ]);
        if let Ok(out) = get_conf_cmd.output() {
            if out.status.success() && !out.stdout.is_empty() {
                get_conf_output = Some(out);
                break;
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    let get_conf = match get_conf_output {
        Some(out) => out,
        None => bail!("Failed to read /etc/rancher/k3s/k3s.yaml from cluster VM"),
    };

    // Establish persistent background SSH tunnel forwarding 127.0.0.1:26443 -> <ip>:6443
    println!("  \x1b[1;36m➜\x1b[0m Establishing persistent background SSH tunnel (127.0.0.1:26443 -> {ip}:6443)...");
    terminate_k8s_ssh_tunnel();

    let mut tunnel_cmd = Command::new("ssh");
    tunnel_cmd.args(crate::ssh::mach_ssh_args());
    tunnel_cmd.args([
        "-f",
        "-nNT",
        "-L", "26443:127.0.0.1:6443",
        &format!("root@{ip}"),
    ]);
    let tunnel_status = tunnel_cmd.status().context("Failed to spawn SSH tunnel command")?;
    if !tunnel_status.success() {
        bail!("Failed to establish Kubernetes SSH tunnel (127.0.0.1:26443 -> {ip}:6443)");
    }

    // Give tunnel a moment to bind and verify reachability
    for _ in 0..20 {
        if tokio::net::TcpStream::connect("127.0.0.1:26443").await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Update kubeconfig cleanly without touching base64 certificate data:
    // - server: https://127.0.0.1:26443
    // - cluster: mach
    // - context: mach
    // - current-context: mach
    let raw_yaml = String::from_utf8_lossy(&get_conf.stdout);
    let mut patched_lines = Vec::new();
    for line in raw_yaml.lines() {
        let trimmed = line.trim();
        let leading_spaces = line.len() - line.trim_start().len();
        let indent = " ".repeat(leading_spaces);

        if trimmed == "name: default" {
            patched_lines.push(format!("{}name: mach", indent));
        } else if trimmed == "- name: default" {
            patched_lines.push(format!("{}- name: mach", indent));
        } else if trimmed == "cluster: default" {
            patched_lines.push(format!("{}cluster: mach", indent));
        } else if trimmed == "user: default" {
            patched_lines.push(format!("{}user: mach", indent));
        } else if trimmed == "current-context: default" {
            patched_lines.push(format!("{}current-context: mach", indent));
        } else if trimmed.starts_with("server:") {
            patched_lines.push(format!("{}server: https://127.0.0.1:26443", indent));
        } else {
            patched_lines.push(line.to_string());
        }
    }
    let mut patched_yaml = patched_lines.join("\n") + "\n";
    if !patched_yaml.contains("current-context:") {
        patched_yaml.push_str("current-context: mach\n");
    }

    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/Users/Shared"));
    let kube_dir = home.join(".kube");
    fs::create_dir_all(&kube_dir)?;

    let mach_kubeconfig = kube_dir.join("config.mach");
    fs::write(&mach_kubeconfig, &patched_yaml)?;

    // Set or merge into ~/.kube/config safely, NEVER overwriting user's active context without permission
    let main_config = kube_dir.join("config");
    if !main_config.exists() {
        fs::write(&main_config, &patched_yaml)?;
    } else {
        // 1. Create a timestamped backup before touching anything
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let backup_file = kube_dir.join(format!("config.bak.{}", timestamp));
        let _ = fs::copy(&main_config, &backup_file);

        // 2. Remember user's existing active context
        let prev_context = Command::new("kubectl")
            .args(["config", "current-context"])
            .output()
            .ok()
            .and_then(|o| if o.status.success() {
                let ctx = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if ctx.is_empty() { None } else { Some(ctx) }
            } else {
                None
            });

        // 3. Remove any stale 'mach' entries from main config before merging
        if let Some(main_str) = main_config.to_str() {
            for old in &["mach"] {
                let _ = Command::new("kubectl")
                    .args(["--kubeconfig", main_str, "config", "delete-cluster", old])
                    .output();
                let _ = Command::new("kubectl")
                    .args(["--kubeconfig", main_str, "config", "delete-context", old])
                    .output();
                let _ = Command::new("kubectl")
                    .args(["--kubeconfig", main_str, "config", "delete-user", old])
                    .output();
            }
        }

        // 4. Attempt clean merge via kubectl config view --flatten
        let merge_res = Command::new("kubectl")
            .env("KUBECONFIG", format!("{}:{}", main_config.display(), mach_kubeconfig.display()))
            .args(["config", "view", "--flatten"])
            .output();

        if let Ok(merged) = merge_res {
            if merged.status.success() && !merged.stdout.is_empty() {
                let _ = fs::write(&main_config, &merged.stdout);
            }
        }

        let _ = Command::new("kubectl")
            .args(["config", "set-cluster", "mach", "--server=https://127.0.0.1:26443"])
            .output();
        let _ = Command::new("kubectl")
            .args(["config", "set-context", "mach", "--cluster=mach", "--user=mach"])
            .output();

        // 5. If the user already had an active context set (and it wasn't mach), restore it!
        if let Some(ctx) = prev_context {
            if ctx != "mach" {
                let _ = Command::new("kubectl")
                    .args(["config", "use-context", &ctx])
                    .output();
                println!("  Preserved existing active kubectl context: \x1b[1m{}\x1b[0m", ctx);
                println!("  (To switch to mach: \x1b[1;32mkubectl config use-context mach\x1b[0m)");
            }
        }
    }

    println!("\x1b[1;32m✓\x1b[0m Kubernetes cluster \x1b[1mmach\x1b[0m is UP and READY!");
    println!("  Server:     \x1b[1;34mhttps://127.0.0.1:26443\x1b[0m");
    println!("  Kubeconfig: {}", mach_kubeconfig.display());
    println!("\nTest your cluster:");
    println!("  \x1b[1;32mkubectl get nodes\x1b[0m");
    println!("  \x1b[1;32mkubectl --kubeconfig {} get nodes\x1b[0m", mach_kubeconfig.display());

    Ok(())
}

/// Stop the Kubernetes cluster node and terminate port forwarding.
pub async fn disable() -> Result<()> {
    stop().await
}

/// Start the Kubernetes cluster node.
pub async fn start() -> Result<()> {
    enable().await
}

/// Stop the Kubernetes cluster node and terminate port forwarding.
pub async fn stop() -> Result<()> {
    let cfg = MachConfig::load()?;
    println!("  \x1b[1;36m➜\x1b[0m Terminating Kubernetes SSH tunnel...");
    terminate_k8s_ssh_tunnel();

    let vm_name = resolve_k8s_machine_name(&cfg.machines_dir)?;
    if find_machine(&cfg.machines_dir, &vm_name)?.is_some() {
        println!("  \x1b[1;36m➜\x1b[0m Stopping Kubernetes node {}...", vm_name);
        crate::commands::stop::run(Some(&vm_name), false, true).await?;
        println!("\x1b[1;32m✓\x1b[0m Kubernetes cluster stopped.");
    } else {
        println!("Kubernetes cluster is not currently installed.");
    }
    Ok(())
}

/// Restart the Kubernetes cluster node.
pub async fn restart() -> Result<()> {
    stop().await?;
    start().await
}

/// Open an interactive shell inside the Kubernetes infrastructure machine.
pub async fn shell() -> Result<()> {
    let cfg = MachConfig::load()?;
    let vm_name = resolve_k8s_machine_name(&cfg.machines_dir)?;
    if find_machine(&cfg.machines_dir, &vm_name)?.is_none() {
        enable().await?;
    }
    println!("\x1b[1;36m➜\x1b[0m Connecting to Kubernetes infrastructure machine (\x1b[1m{}\x1b[0m)...", vm_name);
    crate::commands::shell::run(Some(&vm_name), true, &[]).await
}

/// Show status of the Kubernetes cluster.
pub async fn status() -> Result<()> {
    let cfg = MachConfig::load()?;
    let vm_name = resolve_k8s_machine_name(&cfg.machines_dir)?;
    let machine = find_machine(&cfg.machines_dir, &vm_name)?;

    match machine {
        Some(m) if m.state == MachineState::Running => {
            let ip = m.ip_address.unwrap_or_else(|| std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
            println!("\x1b[1;32m●\x1b[0m Kubernetes is \x1b[1;32mrunning\x1b[0m on {} (IP: {})", vm_name, ip);

            // Test connecting to https://127.0.0.1:26443
            let tunnel_connected = tokio::time::timeout(
                Duration::from_millis(600),
                tokio::net::TcpStream::connect("127.0.0.1:26443"),
            )
            .await
            .map(|r| r.is_ok())
            .unwrap_or(false);

            if tunnel_connected {
                println!("  API Server:  \x1b[1;32m●\x1b[0m \x1b[1;34mhttps://127.0.0.1:26443\x1b[0m (reachable)");
                let home = dirs::home_dir().unwrap_or_default();
                let conf = home.join(".kube").join("config.mach");
                if conf.exists() {
                    println!("  Kubeconfig:  {}", conf.display());
                    let _ = Command::new("kubectl")
                        .args(["--kubeconfig", conf.to_str().unwrap(), "--request-timeout=2s", "get", "nodes"])
                        .status();
                }
            } else {
                println!("  API Server:  \x1b[1;31m○\x1b[0m \x1b[1;33mhttps://127.0.0.1:26443\x1b[0m (not responding)");
                println!("               Run \x1b[1;34mmachctl k8s start\x1b[0m to restore the tunnel.");
            }
        }
        Some(m) => {
            terminate_k8s_ssh_tunnel();
            println!("\x1b[1;33m○\x1b[0m Kubernetes node {} is \x1b[1;33mstopped\x1b[0m ({})", vm_name, m.state);
            println!("  API Server:  \x1b[1;33m○ stopped (run 'machctl k8s start')\x1b[0m");
        }
        None => {
            terminate_k8s_ssh_tunnel();
            println!("\x1b[1;31m○\x1b[0m Kubernetes is not installed.");
            println!("  API Server:  \x1b[1;33m○ stopped (run 'machctl k8s start')\x1b[0m");
            println!("  Run \x1b[1;34mmachctl k8s start\x1b[0m to provision a lightweight local cluster.");
        }
    }

    Ok(())
}
