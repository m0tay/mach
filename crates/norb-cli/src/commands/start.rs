//! `norb start` command implementation.

use anyhow::{bail, Context, Result};
use norb_common::config::NorbConfig;
use norb_common::machine::MachineState;
use std::fs;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::time::sleep;

use super::{find_machine, load_all_machines, save_machine};

/// Resolve kernel and initramfs assets from ~/.norb/assets or repo assets/
fn resolve_assets(cfg: &NorbConfig) -> Result<(PathBuf, PathBuf)> {
    let home_assets = cfg.data_dir.join("assets");
    let home_kernel = home_assets.join("kernel");
    let home_initrd = home_assets.join("initrd.cpio.gz");

    if home_kernel.exists() && home_initrd.exists() {
        return Ok((home_kernel, home_initrd));
    }

    let local_kernel = Path::new("assets/kernel");
    let local_initrd = Path::new("assets/initrd.cpio.gz");

    if local_kernel.exists() && local_initrd.exists() {
        return Ok((local_kernel.to_path_buf(), local_initrd.to_path_buf()));
    }

    bail!(
        "Norb Linux assets not found in {} or ./assets.\nRun `./scratch/build_rootfs.sh` or check ~/.norb/assets.",
        home_assets.display()
    );
}

/// Generate a stable MAC address string from machine UUID bytes: `52:54:00:12:xx:yy`
fn generate_mac(machine_id: &uuid::Uuid) -> (String, String) {
    let bytes = machine_id.as_bytes();
    let b1 = bytes[14];
    let b2 = bytes[15];
    // Standard format for VZMACAddress: 52:54:00:12:xx:yy
    let full_mac = format!("52:54:00:12:{:02x}:{:02x}", b1, b2);
    // macOS dhcpd_leases strips leading zero on single digits
    let lease_mac = format!("52:54:{:x}:12:{:x}:{:x}", 0, b1, b2);
    (full_mac, lease_mac)
}

/// Parse the assigned IP address from /var/db/dhcpd_leases for a given MAC address.
fn get_latest_lease_timestamp() -> u64 {
    let Ok(content) = fs::read_to_string("/var/db/dhcpd_leases") else {
        return 0;
    };
    let mut max_ts = 0u64;
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(stripped) = trimmed.strip_prefix("lease=0x") {
            if let Ok(val) = u64::from_str_radix(stripped, 16) {
                if val > max_ts {
                    max_ts = val;
                }
            }
        }
    }
    max_ts
}

/// Parse the assigned IP address from /var/db/dhcpd_leases.
fn lookup_dhcp_lease(mac_patterns: &[String], lease_after: u64) -> Option<IpAddr> {
    let Ok(content) = fs::read_to_string("/var/db/dhcpd_leases") else {
        return None;
    };

    let blocks: Vec<&str> = content.split('}').collect();
    let mut best_lease: Option<(u64, IpAddr)> = None;

    for block in blocks {
        let matches_pattern = mac_patterns.iter().any(|pattern| block.contains(pattern));
        let mut ip_opt = None;
        let mut lease_time = 0u64;
        for line in block.lines() {
            let trimmed = line.trim();
            if let Some(stripped) = trimmed.strip_prefix("ip_address=") {
                if let Ok(ip) = stripped.parse::<IpAddr>() {
                    ip_opt = Some(ip);
                }
            } else if let Some(stripped) = trimmed.strip_prefix("lease=0x") {
                if let Ok(val) = u64::from_str_radix(stripped, 16) {
                    lease_time = val;
                }
            }
        }
        if let Some(ip) = ip_opt {
            if matches_pattern || (lease_after > 0 && lease_time > lease_after) {
                if best_lease.is_none() || lease_time > best_lease.unwrap().0 {
                    best_lease = Some((lease_time, ip));
                }
            }
        }
    }
    best_lease.map(|(_, ip)| ip)
}

pub async fn run(name: Option<&str>) -> Result<()> {
    let cfg = NorbConfig::load()?;

    let target_name = match name {
        Some(n) => n.to_string(),
        None => {
            if let Some(ref default_name) = cfg.default_machine {
                default_name.clone()
            } else {
                let machines = load_all_machines(&cfg.machines_dir)?;
                if machines.is_empty() {
                    bail!("no machines found. Run `norb create <distro>` first.");
                }
                machines[0].config.name.clone()
            }
        }
    };

    let mut machine = find_machine(&cfg.machines_dir, &target_name)?
        .with_context(|| format!("machine '{target_name}' not found"))?;

    // Check if already running by checking PID
    if let Some(pid) = machine.pid {
        if nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), None).is_ok() {
            println!("Machine \x1b[1m{}\x1b[0m is already running (PID: {}).", machine.config.name, pid);
            if let Some(ip) = machine.ip_address {
                println!("  IP Address:  \x1b[1;34m{}\x1b[0m", ip);
                println!("  SSH Command: \x1b[1;32mssh root@{}\x1b[0m", ip);
            }
            return Ok(());
        }
    }

    let (mac_str, lease_pattern) = generate_mac(&machine.id);

    println!("\x1b[1;36m➜\x1b[0m Booting real Linux VM \x1b[1m{}\x1b[0m on Apple Silicon...", machine.config.name);

    let machine_dir = cfg.machines_dir.join(&machine.config.name);
    fs::create_dir_all(&machine_dir)?;
    let log_file = fs::File::create(machine_dir.join("vm.log"))?;

    let disk_path = machine_dir.join("disk.raw");
    if !disk_path.exists() {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&disk_path)?;
        file.set_len(machine.config.disk_gb * 1024 * 1024 * 1024)?;
    }

    let is_alpine = machine.config.distro.eq_ignore_ascii_case("alpine");

    use std::os::unix::process::CommandExt;
    let current_exe = std::env::current_exe()?;
    let mut cmd = std::process::Command::new(current_exe);
    cmd.args([
        "run-vm",
        "--name",
        &machine.config.name,
        "--disk",
        disk_path.to_str().unwrap(),
        "--shared-dir",
        "/Users",
        "--cpus",
        &machine.config.cpus.to_string(),
        "--memory",
        &machine.config.memory_mb.to_string(),
        "--mac",
        &mac_str,
    ]);

    if is_alpine {
        let (kernel_path, initrd_path) = resolve_assets(&cfg)?;
        cmd.args([
            "--kernel",
            kernel_path.to_str().unwrap(),
            "--initrd",
            initrd_path.to_str().unwrap(),
        ]);
    } else {
        let cidata_path = machine_dir.join("cidata.iso");
        if cidata_path.exists() {
            cmd.args(["--cidata", cidata_path.to_str().unwrap()]);
        }
    }

    cmd.stdout(log_file.try_clone()?)
        .stderr(log_file)
        .stdin(std::process::Stdio::null())
        .process_group(0);

    let lease_before = get_latest_lease_timestamp();

    let child = cmd.spawn()
        .with_context(|| "failed to spawn background VMM process")?;

    let child_pid = child.id();
    fs::write(machine_dir.join("pid"), child_pid.to_string())?;

    print!("  Waiting for network & DHCP lease...");
    std::io::Write::flush(&mut std::io::stdout())?;

    // Poll for DHCP IP assignment
    let patterns = vec![
        mac_str.clone(),
        lease_pattern,
    ];
    let mut assigned_ip = None;

    for _ in 0..75 {
        sleep(Duration::from_millis(200)).await;
        if let Some(ip) = lookup_dhcp_lease(&patterns, lease_before) {
            assigned_ip = Some(ip);
            break;
        }
    }

    println!();

    let ip = assigned_ip.unwrap_or_else(|| {
        std::net::IpAddr::V4(std::net::Ipv4Addr::new(192, 168, 64, 4))
    });

    // Wait for SSH port to accept connections
    print!("  Waiting for SSH service to become ready...");
    std::io::Write::flush(&mut std::io::stdout())?;
    for _ in 0..60 {
        let test_ssh = std::process::Command::new("ssh")
            .args([
                "-o", "StrictHostKeyChecking=no",
                "-o", "UserKnownHostsFile=/dev/null",
                "-o", "LogLevel=ERROR",
                "-o", "ConnectTimeout=1",
                &format!("root@{ip}"),
                "true",
            ])
            .status();
        if let Ok(st) = test_ssh {
            if st.success() {
                break;
            }
        }
        sleep(Duration::from_millis(500)).await;
    }
    println!();

    machine.state = MachineState::Running;
    machine.ip_address = Some(ip);
    machine.pid = Some(child_pid);

    save_machine(&cfg.machines_dir, &machine)?;

    println!(
        "\x1b[1;32m✓\x1b[0m Machine \x1b[1m{}\x1b[0m is UP and RUNNING (PID: {}).",
        machine.config.name, child_pid
    );
    println!("  IP Address:  \x1b[1;34m{}\x1b[0m", ip);
    println!("  Domain:      \x1b[1;34m{}.norb.local\x1b[0m", machine.config.name);
    println!("  SSH Command: \x1b[1;32mssh root@{}\x1b[0m", ip);

    // Setup background Docker socket forwarder if Docker is running in this guest
    let check_docker = std::process::Command::new("ssh")
        .args([
            "-o", "StrictHostKeyChecking=no",
            "-o", "UserKnownHostsFile=/dev/null",
            "-o", "LogLevel=ERROR",
            "-o", "ConnectTimeout=2",
            &format!("root@{ip}"),
            "test -S /var/run/docker.sock",
        ])
        .status();

    if let Ok(st) = check_docker {
        if st.success() {
            let run_dir = cfg.data_dir.join("run");
            let _ = fs::create_dir_all(&run_dir);
            let host_sock = run_dir.join("docker.sock");
            let _ = fs::remove_file(&host_sock);
            let _ = std::process::Command::new("ssh")
                .args([
                    "-f", "-nNT",
                    "-L", &format!("{}:/var/run/docker.sock", host_sock.display()),
                    "-o", "StrictHostKeyChecking=no",
                    "-o", "UserKnownHostsFile=/dev/null",
                    "-o", "LogLevel=ERROR",
                    "-o", "ConnectTimeout=2",
                    &format!("root@{ip}"),
                ])
                .spawn();

            for _ in 0..15 {
                if host_sock.exists() {
                    break;
                }
                sleep(Duration::from_millis(200)).await;
            }

            println!("  Docker:      \x1b[1;34munix://{}\x1b[0m", host_sock.display());
        }
    }

    // Ensure background daemon is running for local DNS and port forwarding
    crate::commands::daemon::ensure_daemon_running();

    println!("\nRun \x1b[1;34mnorb shell {}\x1b[0m to enter the machine.", machine.config.name);
    println!("Run \x1b[1;32mdocker ps\x1b[0m (or \x1b[1;32mnorb docker setup\x1b[0m) to control containers.");

    Ok(())
}
