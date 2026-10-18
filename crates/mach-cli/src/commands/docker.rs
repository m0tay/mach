//! `mach docker` command implementation for dedicated Docker infrastructure.

use anyhow::Result;
use mach_common::config::MachConfig;
use mach_common::machine::{Machine, MachineState};
use std::net::IpAddr;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

pub const SYSTEM_VM_NAME: &str = "mach-system";
pub const DOCKER_VM_NAME: &str = SYSTEM_VM_NAME;

/// Find active system infrastructure machine (prefers mach-system, falls back to legacy mach-docker).
pub fn find_system_machine(machines_dir: &Path) -> Result<Option<Machine>> {
    if let Some(m) = super::find_machine(machines_dir, SYSTEM_VM_NAME)? {
        return Ok(Some(m));
    }
    if let Some(m) = super::find_machine(machines_dir, "mach-docker")? {
        return Ok(Some(m));
    }
    Ok(None)
}

/// Ensure the dedicated system infrastructure machine exists.
pub async fn ensure_docker_vm(cfg: &MachConfig) -> Result<Machine> {
    let target_mem = cfg.system_memory_mib();
    if let Some(mut m) = find_system_machine(&cfg.machines_dir)? {
        if m.config.memory_mb != target_mem {
            m.config.memory_mb = target_mem;
            let _ = m.save(&cfg.machines_dir);
        }
        return Ok(m);
    }

    println!("  \x1b[1;36m➜\x1b[0m Provisioning unified system infrastructure machine (\x1b[1m{}\x1b[0m, {} MiB RAM)...", SYSTEM_VM_NAME, target_mem);
    crate::commands::create::run_with_isolated("debian", Some(SYSTEM_VM_NAME), 2, target_mem, 20, "aarch64", false).await?;

    let m = super::find_machine(&cfg.machines_dir, SYSTEM_VM_NAME)?
        .ok_or_else(|| anyhow::anyhow!("failed to create system infrastructure machine"))?;
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

/// Check whether the Docker daemon is responding inside the VM via /var/run/docker.sock.
pub async fn is_docker_running_in_vm(ip: IpAddr) -> bool {
    let base_ssh = crate::ssh::mach_ssh_args();
    let mut check_args = base_ssh;
    check_args.extend([
        "-o".to_string(),
        "ConnectTimeout=2".to_string(),
        format!("root@{ip}"),
        "test -S /var/run/docker.sock".to_string(),
    ]);

    tokio::time::timeout(
        Duration::from_secs(3),
        tokio::process::Command::new("ssh").args(&check_args).status(),
    )
    .await
    .map(|res| res.map(|s| s.success()).unwrap_or(false))
    .unwrap_or(false)
}

/// Ensure /etc/docker/daemon.json is cleanly configured on the VM.
pub async fn ensure_daemon_json(ip: IpAddr) -> Result<()> {
    let daemon_cfg_script = r#"
mkdir -p /etc/docker
if [ ! -f /etc/docker/daemon.json ]; then
cat << 'EOF' > /etc/docker/daemon.json
{
  "log-driver": "json-file",
  "log-opts": {
    "max-size": "10m",
    "max-file": "3"
  },
  "features": {
    "buildkit": true
  }
}
EOF
if pidof dockerd >/dev/null 2>&1; then
    systemctl reload docker 2>/dev/null || kill -s HUP $(pidof dockerd) 2>/dev/null || true
fi
fi
"#;
    let base_ssh = crate::ssh::mach_ssh_args();
    let mut args = base_ssh;
    args.extend([
        "-o".to_string(),
        "ConnectTimeout=5".to_string(),
        format!("root@{ip}"),
        daemon_cfg_script.to_string(),
    ]);

    let _ = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::process::Command::new("ssh").args(&args).status(),
    )
    .await;

    Ok(())
}

/// Provision Docker Engine inside the specified VM using fast official static binaries
/// or get.docker.com with strict timeout and status feedback.
pub async fn ensure_docker_installed(ip: IpAddr, vm_name: &str) -> Result<()> {
    // 1. If Docker socket already exists and is active, ensure daemon.json and return
    if is_docker_running_in_vm(ip).await {
        let _ = ensure_daemon_json(ip).await;
        return Ok(());
    }

    // 2. Clear progress indication
    println!("  \x1b[1;36m➜\x1b[0m Installing Docker Engine inside {vm_name}...");
    let start_time = std::time::Instant::now();

    // 3. Resilient provisioning script
    let install_script = r#"set -e

# If dockerd binary exists, just ensure it is started
if command -v dockerd >/dev/null 2>&1; then
    systemctl daemon-reload 2>/dev/null || true
    systemctl enable --now docker 2>/dev/null || service docker start 2>/dev/null || true
    for i in $(seq 1 10); do
        if [ -S /var/run/docker.sock ]; then exit 0; fi
        sleep 0.5
    done
fi

# Configure docker group
getent group docker >/dev/null 2>&1 || groupadd -r docker >/dev/null 2>&1 || true

# Pre-configure /etc/docker/daemon.json
mkdir -p /etc/docker
if [ ! -f /etc/docker/daemon.json ]; then
cat << 'EOF' > /etc/docker/daemon.json
{
  "log-driver": "json-file",
  "log-opts": {
    "max-size": "10m",
    "max-file": "3"
  },
  "features": {
    "buildkit": true
  }
}
EOF
fi

# Detect architecture
ARCH=$(uname -m)
case "$ARCH" in
    aarch64|arm64) DOCKER_ARCH="aarch64" ;;
    x86_64|amd64)  DOCKER_ARCH="x86_64" ;;
    *)             DOCKER_ARCH="aarch64" ;;
esac

STATIC_URL="https://download.docker.com/linux/static/stable/${DOCKER_ARCH}/docker-26.1.4.tgz"
INSTALLED=0

# Primary Path: Fast static binary installation (~2-4 seconds)
TMP_DIR=$(mktemp -d /tmp/docker-setup.XXXXXX)
if command -v curl >/dev/null 2>&1; then
    if curl -fsSL --connect-timeout 8 --max-time 35 "$STATIC_URL" -o "$TMP_DIR/docker.tgz"; then
        tar -xzf "$TMP_DIR/docker.tgz" -C "$TMP_DIR"
        cp -f "$TMP_DIR/docker/"* /usr/bin/
        chmod +x /usr/bin/docker* /usr/bin/containerd* /usr/bin/ctr /usr/bin/runc 2>/dev/null || true
        INSTALLED=1
    fi
elif command -v wget >/dev/null 2>&1; then
    if wget -q --timeout=35 -O "$TMP_DIR/docker.tgz" "$STATIC_URL"; then
        tar -xzf "$TMP_DIR/docker.tgz" -C "$TMP_DIR"
        cp -f "$TMP_DIR/docker/"* /usr/bin/
        chmod +x /usr/bin/docker* /usr/bin/containerd* /usr/bin/ctr /usr/bin/runc 2>/dev/null || true
        INSTALLED=1
    fi
fi
rm -rf "$TMP_DIR"

if [ "$INSTALLED" -eq 1 ]; then
    cat << 'EOF' > /etc/systemd/system/docker.service
[Unit]
Description=Docker Application Container Engine
Documentation=https://docs.docker.com
After=network-online.target firewalld.service
Wants=network-online.target

[Service]
Type=notify
ExecStart=/usr/bin/dockerd
ExecReload=/bin/kill -s HUP $MAINPID
TimeoutStartSec=20
RestartSec=2
Restart=always
LimitNOFILE=infinity
LimitNPROC=infinity
LimitCORE=infinity
TasksMax=infinity
Delegate=yes
KillMode=process

[Install]
WantedBy=multi-user.target
EOF
    systemctl daemon-reload
    systemctl enable --now docker.service
    for i in $(seq 1 20); do
        if [ -S /var/run/docker.sock ]; then exit 0; fi
        sleep 0.5
    done
    exit 0
fi

# Fallback 1: get.docker.com convenience script
if command -v curl >/dev/null 2>&1; then
    if curl -fsSL --connect-timeout 8 --max-time 40 https://get.docker.com | sh; then
        systemctl enable --now docker.service 2>/dev/null || true
        exit 0
    fi
fi

# Fallback 2: Fast minimal non-interactive apt
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq -o Acquire::Connect-Timeout=8 -o Acquire::Http::Timeout=15 || true
apt-get install -y -qq --no-install-recommends docker.io containerd || true
systemctl enable --now docker.service 2>/dev/null || true
"#;

    let base_ssh = crate::ssh::mach_ssh_args();
    let mut install_args = base_ssh.clone();
    install_args.extend([
        "-o".to_string(),
        "ConnectTimeout=10".to_string(),
        format!("root@{ip}"),
        install_script.to_string(),
    ]);

    // 60-second execution timeout using tokio timeout so it never hangs indefinitely
    let install_future = async {
        let output = tokio::process::Command::new("ssh")
            .args(&install_args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .output()
            .await?;
        Ok::<_, anyhow::Error>(output)
    };

    match tokio::time::timeout(Duration::from_secs(60), install_future).await {
        Ok(Ok(output)) => {
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stdout = String::from_utf8_lossy(&output.stdout);
                let err_msg = if !stderr.trim().is_empty() {
                    stderr.trim()
                } else if !stdout.trim().is_empty() {
                    stdout.trim()
                } else {
                    "install script returned non-zero exit code"
                };
                tracing::warn!("Docker install script message: {err_msg}");
            }
        }
        Ok(Err(e)) => {
            anyhow::bail!("Failed to execute Docker installation command via SSH: {e}");
        }
        Err(_) => {
            anyhow::bail!("Docker installation timed out after 60 seconds inside {vm_name}");
        }
    }

    // Wait up to 15 seconds for /var/run/docker.sock to become active
    let mut sock_active = false;
    for _ in 0..30 {
        if is_docker_running_in_vm(ip).await {
            sock_active = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    if !sock_active {
        anyhow::bail!("Docker engine failed to start /var/run/docker.sock inside {vm_name}");
    }

    // Ensure /etc/docker/daemon.json is clean
    let _ = ensure_daemon_json(ip).await;

    let elapsed = start_time.elapsed().as_secs();
    println!("  \x1b[1;32m✓\x1b[0m Docker Engine installed and ready inside {vm_name} ({elapsed}s)");
    Ok(())
}

/// Ensure reverse proxy forwarder on port 80 is active inside VM
pub async fn ensure_proxy_forwarder(ip: IpAddr) -> Result<()> {
    let proxy_setup_script = r#"which socat >/dev/null 2>&1 || (export DEBIAN_FRONTEND=noninteractive; apt-get update -qq -o Acquire::Connect-Timeout=5 -o Acquire::Http::Timeout=10 && apt-get install -y -qq --no-install-recommends socat);
if [ ! -f /etc/systemd/system/mach-proxy.service ]; then
cat << 'EOF' > /etc/systemd/system/mach-proxy.service
[Unit]
Description=Mach Reverse Proxy Forwarder
After=network.target

[Service]
ExecStart=/usr/bin/socat TCP-LISTEN:80,fork,reuseaddr TCP:192.168.64.1:28080
Restart=always
RestartSec=2

[Install]
WantedBy=multi-user.target
EOF
systemctl daemon-reload
fi
systemctl enable --now mach-proxy.service >/dev/null 2>&1 || true
"#;
    let base_ssh = crate::ssh::mach_ssh_args();
    let mut proxy_args = base_ssh;
    proxy_args.extend([
        "-o".to_string(),
        "ConnectTimeout=5".to_string(),
        format!("root@{ip}"),
        proxy_setup_script.to_string(),
    ]);

    let _ = tokio::time::timeout(
        Duration::from_secs(15),
        tokio::process::Command::new("ssh").args(&proxy_args).status(),
    )
    .await;

    Ok(())
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
    ensure_docker_installed(ip, DOCKER_VM_NAME).await?;

    // 2.5 Ensure reverse proxy forwarder on port 80 is active inside VM
    ensure_proxy_forwarder(ip).await?;

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
    docker_fwd_args.extend(crate::ssh::mach_ssh_args());
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
    let docker_bin = super::find_binary("docker").unwrap_or_else(|| std::path::PathBuf::from("docker"));
    let _ = Command::new(&docker_bin).args(["context", "rm", "-f", "mach"]).output();

    let output = Command::new(&docker_bin)
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
        let _ = Command::new(&docker_bin).args(["context", "use", "mach"]).output();
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
            println!("  \x1b[1;32mmachctl docker enable --system\x1b[0m");
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

/// Start Docker engine and configure CLI context
pub async fn start(system: bool) -> Result<()> {
    setup(system).await
}

/// Enable Docker engine and configure CLI context
pub async fn enable(system: bool) -> Result<()> {
    start(system).await
}

/// Disable Docker engine
pub async fn disable() -> Result<()> {
    stop().await
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

    if let Some(m) = find_system_machine(&cfg.machines_dir)? {
        crate::commands::stop::run(Some(&m.config.name), false, true).await?;
        println!("\x1b[1;32m✓\x1b[0m System infrastructure machine stopped.");
    } else {
        println!("Docker engine is not currently initialized.");
    }
    Ok(())
}

/// Restart Docker engine
pub async fn restart() -> Result<()> {
    stop().await?;
    start(false).await
}

/// Open an interactive shell inside the system infrastructure machine
pub async fn shell() -> Result<()> {
    let cfg = MachConfig::load()?;
    let vm_name = match find_system_machine(&cfg.machines_dir)? {
        Some(m) => m.config.name,
        None => {
            let _ = setup(false).await?;
            SYSTEM_VM_NAME.to_string()
        }
    };
    println!("\x1b[1;36m➜\x1b[0m Connecting to system infrastructure machine (\x1b[1m{}\x1b[0m)...", vm_name);
    super::shell::run(Some(&vm_name), true, &[]).await
}

/// Destroy the system infrastructure machine and its container volumes
pub async fn destroy(force: bool) -> Result<()> {
    if let Ok(mut c) = MachConfig::load() {
        c.docker.was_running = false;
        let _ = c.save();
    }
    let cfg = MachConfig::load()?;
    let sys_m = find_system_machine(&cfg.machines_dir)?;
    if sys_m.is_none() {
        println!("System infrastructure machine does not exist.");
        return Ok(());
    }
    let vm_name = sys_m.unwrap().config.name;

    println!("\x1b[1;33m⚠\x1b[0m Destroying system infrastructure machine ({vm_name}) and all stored container data...");
    stop().await?;
    super::delete::run(&vm_name, force, true).await?;
    println!("\x1b[1;32m✓\x1b[0m System infrastructure machine destroyed.");
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
    let docker_vm = find_system_machine(&cfg.machines_dir)?;

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

/// Stream or show Docker daemon systemd logs from the infrastructure VM.
pub async fn logs(follow: bool, lines: usize) -> Result<()> {
    let cfg = MachConfig::load()?;
    let m = find_system_machine(&cfg.machines_dir)?
        .ok_or_else(|| anyhow::anyhow!("Docker infrastructure machine not found"))?;

    if m.state != MachineState::Running {
        anyhow::bail!("Docker machine '{}' is not running (state: {:?})", m.config.name, m.state);
    }

    let ip = m.ip_address.ok_or_else(|| anyhow::anyhow!("Docker machine has no IP address"))?;

    let mut args = crate::ssh::mach_ssh_args();
    args.extend([
        "-t".to_string(),
        format!("root@{ip}"),
        format!("journalctl -u docker -n {}{}", lines, if follow { " -f" } else { "" }),
    ]);

    let status = Command::new("ssh").args(&args).status()?;
    if !status.success() {
        if let Some(code) = status.code() {
            if code != 130 { // 130 is Ctrl+C
                tracing::debug!("ssh journalctl exited with status {code}");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_docker_vm_names() {
        assert_eq!(SYSTEM_VM_NAME, "mach-system");
        assert_eq!(DOCKER_VM_NAME, "mach-system");
    }

    #[test]
    fn test_daemon_json_template_is_valid_json() {
        let json_str = r#"{
  "log-driver": "json-file",
  "log-opts": {
    "max-size": "10m",
    "max-file": "3"
  },
  "features": {
    "buildkit": true
  }
}"#;
        let parsed: serde_json::Value = serde_json::from_str(json_str).expect("Valid JSON");
        assert_eq!(parsed["log-driver"], "json-file");
        assert_eq!(parsed["log-opts"]["max-size"], "10m");
        assert_eq!(parsed["log-opts"]["max-file"], "3");
        assert_eq!(parsed["features"]["buildkit"], true);
    }

    #[test]
    fn test_docker_static_arch_mapping() {
        let resolve_arch = |arch: &str| -> &'static str {
            match arch {
                "aarch64" | "arm64" => "aarch64",
                "x86_64" | "amd64" => "x86_64",
                _ => "aarch64",
            }
        };

        assert_eq!(resolve_arch("aarch64"), "aarch64");
        assert_eq!(resolve_arch("arm64"), "aarch64");
        assert_eq!(resolve_arch("x86_64"), "x86_64");
        assert_eq!(resolve_arch("amd64"), "x86_64");
        assert_eq!(resolve_arch("other"), "aarch64");

        let url = format!(
            "https://download.docker.com/linux/static/stable/{}/docker-26.1.4.tgz",
            resolve_arch("aarch64")
        );
        assert_eq!(
            url,
            "https://download.docker.com/linux/static/stable/aarch64/docker-26.1.4.tgz"
        );
    }
}

