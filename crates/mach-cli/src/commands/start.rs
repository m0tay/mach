//! `mach start` command implementation.

use anyhow::{bail, Context, Result};
use mach_common::config::MachConfig;
use mach_common::machine::MachineState;
use std::fs;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};
use tokio::time::sleep;

use super::{find_machine, load_all_machines, save_machine};

/// Resolve kernel and initramfs assets from ~/.mach/assets or repo assets/
fn resolve_assets(cfg: &MachConfig) -> Result<(PathBuf, PathBuf)> {
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
        "mach Linux assets not found in {} or ./assets.\nCheck ~/.mach/assets.",
        home_assets.display()
    );
}

/// Generate stable MAC address patterns from machine UUID bytes: `52:54:00:12:xx:yy`
fn generate_mac(machine_id: &uuid::Uuid) -> (String, String) {
    let bytes = machine_id.as_bytes();
    let b1 = bytes[14];
    let b2 = bytes[15];
    // Standard format for VZMACAddress: 52:54:00:12:xx:yy
    let full_mac = format!("52:54:00:12:{:02x}:{:02x}", b1, b2);
    // macOS dhcpd_leases strips leading zero on single digits: e.g. 52:54:0:12:a:bf
    let lease_mac = format!("52:54:0:12:{:x}:{:x}", b1, b2);
    (full_mac, lease_mac)
}

/// RAII guard ensuring background hypervisor child process is killed if startup fails or is cancelled.
struct VmmChildGuard {
    pid: u32,
    pid_file: PathBuf,
    started_successfully: bool,
}

impl Drop for VmmChildGuard {
    fn drop(&mut self) {
        if !self.started_successfully {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(self.pid as i32),
                nix::sys::signal::Signal::SIGKILL,
            );
            let _ = fs::remove_file(&self.pid_file);
        }
    }
}

static LEASE_CACHE: Mutex<Option<(SystemTime, String, u64, u64)>> = Mutex::new(None);

/// Get the latest lease expiration timestamps currently recorded for a machine's MAC and name.
fn get_machine_lease_timestamps(name: &str, mac_patterns: &[String]) -> (u64, u64) {
    let Ok(meta) = fs::metadata("/var/db/dhcpd_leases") else {
        return (0, 0);
    };
    let Ok(mtime) = meta.modified() else {
        return (0, 0);
    };

    if let Ok(guard) = LEASE_CACHE.lock() {
        if let Some((cached_mtime, cached_name, mac_ts, name_ts)) = &*guard {
            if *cached_mtime == mtime && cached_name == name {
                return (*mac_ts, *name_ts);
            }
        }
    }

    let Ok(content) = fs::read_to_string("/var/db/dhcpd_leases") else {
        return (0, 0);
    };
    let blocks: Vec<&str> = content.split('}').collect();
    let mut max_mac_ts = 0u64;
    let mut max_name_ts = 0u64;

    for block in blocks {
        let mut lease_time = 0u64;
        let mut matches_name = false;
        let matches_pattern = mac_patterns.iter().any(|p| block.contains(p));

        for line in block.lines() {
            let trimmed = line.trim();
            if let Some(stripped) = trimmed.strip_prefix("name=") {
                if stripped.eq_ignore_ascii_case(name) {
                    matches_name = true;
                }
            } else if let Some(stripped) = trimmed.strip_prefix("lease=0x") {
                if let Ok(val) = u64::from_str_radix(stripped, 16) {
                    lease_time = val;
                }
            }
        }

        if matches_pattern && lease_time > max_mac_ts {
            max_mac_ts = lease_time;
        }
        if matches_name && lease_time > max_name_ts {
            max_name_ts = lease_time;
        }
    }

    if let Ok(mut guard) = LEASE_CACHE.lock() {
        *guard = Some((mtime, name.to_string(), max_mac_ts, max_name_ts));
    }

    (max_mac_ts, max_name_ts)
}

/// Parse the assigned IP address from /var/db/dhcpd_leases matching either MAC address or machine name.
fn lookup_dhcp_lease(
    name: &str,
    mac_patterns: &[String],
    prev_mac_ts: u64,
    prev_name_ts: u64,
) -> Option<IpAddr> {
    let Ok(content) = fs::read_to_string("/var/db/dhcpd_leases") else {
        return None;
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let blocks: Vec<&str> = content.split('}').collect();
    let mut best_mac_lease: Option<(u64, IpAddr)> = None;
    let mut best_name_lease: Option<(u64, IpAddr)> = None;

    for block in blocks {
        let mut ip_opt = None;
        let mut lease_time = 0u64;
        let mut matches_name = false;
        let matches_pattern = mac_patterns.iter().any(|pattern| block.contains(pattern));

        for line in block.lines() {
            let trimmed = line.trim();
            if let Some(stripped) = trimmed.strip_prefix("name=") {
                if stripped.eq_ignore_ascii_case(name) {
                    matches_name = true;
                }
            } else if let Some(stripped) = trimmed.strip_prefix("ip_address=") {
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
            // For deterministic hardware MAC address: any unexpired lease or new lease belongs to this machine
            let mac_valid = (lease_time > now) || (prev_mac_ts == 0) || (lease_time >= prev_mac_ts);
            if matches_pattern && mac_valid {
                if best_mac_lease.is_none() || lease_time > best_mac_lease.unwrap().0 {
                    best_mac_lease = Some((lease_time, ip));
                }
            }
            // For machine name fallback
            let name_valid = (prev_name_ts == 0 || lease_time > prev_name_ts) && (lease_time > now || prev_name_ts == 0);
            if matches_name && name_valid {
                if best_name_lease.is_none() || lease_time > best_name_lease.unwrap().0 {
                    best_name_lease = Some((lease_time, ip));
                }
            }
        }
    }

    best_mac_lease.or(best_name_lease).map(|(_, ip)| ip)
}

pub async fn run(name: Option<&str>) -> Result<()> {
    run_ext(name, false).await
}

pub async fn run_ext(name: Option<&str>, to_stderr: bool) -> Result<()> {
    macro_rules! out_ln {
        ($($arg:tt)*) => {
            if to_stderr {
                eprintln!($($arg)*);
            } else {
                println!($($arg)*);
            }
        };
    }

    macro_rules! out_pr {
        ($($arg:tt)*) => {
            if to_stderr {
                eprint!($($arg)*);
                let _ = std::io::Write::flush(&mut std::io::stderr());
            } else {
                print!($($arg)*);
                let _ = std::io::Write::flush(&mut std::io::stdout());
            }
        };
    }
    let cfg = MachConfig::load()?;

    let target_name = match name {
        Some(n) => n.to_string(),
        None => {
            if let Some(ref default_name) = cfg.default_machine {
                default_name.clone()
            } else {
                let machines = load_all_machines(&cfg.machines_dir)?;
                if machines.is_empty() {
                    bail!("no machines found. Run `mach create <distro>` first.");
                }
                machines[0].config.name.clone()
            }
        }
    };

    let mut machine = find_machine(&cfg.machines_dir, &target_name)?
        .with_context(|| format!("machine '{target_name}' not found"))?;

    if machine.config.name == "mach-docker" {
        let target_mem = cfg.system_memory_mib();
        if machine.config.memory_mb != target_mem {
            machine.config.memory_mb = target_mem;
            let _ = machine.save(&cfg.machines_dir);
        }
    }

    let machine_dir = cfg.machines_dir.join(&machine.config.name);
    fs::create_dir_all(&machine_dir)?;

    let pid_file = machine_dir.join("pid");
    let check_pid = machine.pid
        .or_else(|| fs::read_to_string(&pid_file).ok().and_then(|s| s.trim().parse::<u32>().ok()))
        .or_else(|| super::find_running_vm_pid(&machine.config.name));

    if let Some(pid) = check_pid {
        let raw_pid = nix::unistd::Pid::from_raw(pid as i32);
        if nix::sys::signal::kill(raw_pid, None).is_ok() {
            if let Some(ip) = machine.ip_address {
                let connect_fut = tokio::net::TcpStream::connect((ip, 22));
                if tokio::time::timeout(Duration::from_millis(1500), connect_fut).await.map(|r| r.is_ok()).unwrap_or(false) {
                    machine.state = MachineState::Running;
                    machine.pid = Some(pid);
                    let _ = save_machine(&cfg.machines_dir, &machine);
                    out_ln!("Machine \x1b[1m{}\x1b[0m is already running (PID: {}).", machine.config.name, pid);
                    out_ln!("  IP Address:  \x1b[1;34m{}\x1b[0m", ip);
                    out_ln!("  SSH Command: \x1b[1;32mssh root@{}\x1b[0m", ip);
                    out_ln!("\nNext steps:");
                    out_ln!("  ➜ To open shell:      \x1b[1;32mmach\x1b[0m (or \x1b[1;32mmach shell {}\x1b[0m)", machine.config.name);
                    out_ln!("  ➜ To run commands:    \x1b[1;32mmach uname -a\x1b[0m");
                    out_ln!("  ➜ To view dashboard:  \x1b[1;34mmachctl top\x1b[0m");
                    return Ok(());
                }
            }
            // Stale or stuck orphan process holding disk/EFI lock, kill it cleanly
            let _ = nix::sys::signal::kill(raw_pid, nix::sys::signal::Signal::SIGKILL);
            sleep(Duration::from_millis(250)).await;
        }
        let _ = fs::remove_file(&pid_file);
    }

    // Ensure no lingering duplicate hypervisor process holds the disk lock for this machine
    super::kill_running_vm_processes(Some(&machine.config.name));
    sleep(Duration::from_millis(150)).await;

    let (mac_str, lease_pattern) = generate_mac(&machine.id);

    out_ln!("\x1b[1;36m➜\x1b[0m Booting real Linux VM \x1b[1m{}\x1b[0m on Apple Silicon...", machine.config.name);

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

    let patterns = vec![
        mac_str.clone(),
        lease_pattern,
    ];
    let (prev_mac_ts, prev_name_ts) = get_machine_lease_timestamps(&machine.config.name, &patterns);

    let mut child = cmd.spawn()
        .with_context(|| "failed to spawn background VMM process")?;

    let child_pid = child.id();
    fs::write(machine_dir.join("pid"), child_pid.to_string())?;

    let base_ssh = crate::ssh::mach_ssh_args();

    let mut guard = VmmChildGuard {
        pid: child_pid,
        pid_file: machine_dir.join("pid"),
        started_successfully: false,
    };

    let log_path = machine_dir.join("vm.log");

    let ip = tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            out_ln!("\n\x1b[1;33m➜\x1b[0m Startup cancelled by user (Ctrl-C). Hypervisor terminated.");
            drop(guard);
            bail!("boot cancelled by user");
        }
        res = async {
            out_pr!("  Waiting for network & DHCP lease...");

            let mut assigned_ip = None;
            for _ in 0..150 {
                if let Ok(Some(status)) = child.try_wait() {
                    let log_tail = fs::read_to_string(&log_path)
                        .unwrap_or_default()
                        .lines()
                        .rev()
                        .take(10)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect::<Vec<_>>()
                        .join("\n");
                    bail!("hypervisor process exited unexpectedly with {status}.\nLog details:\n{log_tail}");
                }
                if let Some(ip) = lookup_dhcp_lease(&machine.config.name, &patterns, prev_mac_ts, prev_name_ts) {
                    assigned_ip = Some(ip);
                    break;
                }
                sleep(Duration::from_millis(200)).await;
            }

            out_ln!();

            let ip = match assigned_ip {
                Some(ip) => ip,
                None => {
                    bail!("machine started (PID: {child_pid}), but did not acquire a DHCP lease in time. Check {} for details.", log_path.display());
                }
            };

            // Probe TCP port 22 with 100ms backoff before SSH auth loop to avoid spawning failed child processes
            out_pr!("  Waiting for SSH service to become ready...");

            let mut port_open = false;
            for _ in 0..600 {
                if let Ok(Some(status)) = child.try_wait() {
                    let log_tail = fs::read_to_string(&log_path)
                        .unwrap_or_default()
                        .lines()
                        .rev()
                        .take(10)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect::<Vec<_>>()
                        .join("\n");
                    bail!("hypervisor process exited unexpectedly with {status}.\nLog details:\n{log_tail}");
                }
                if tokio::net::TcpStream::connect((ip, 22)).await.is_ok() {
                    port_open = true;
                    break;
                }
                sleep(Duration::from_millis(100)).await;
            }

            if !port_open {
                bail!("machine acquired IP {ip}, but port 22 did not open within 60 seconds.");
            }

            let mut ssh_ready = false;

            for _ in 0..60 {
                if let Ok(Some(status)) = child.try_wait() {
                    bail!("hypervisor process exited unexpectedly with {status}.");
                }
                let mut test_args = base_ssh.clone();
                test_args.extend([
                    "-o".to_string(), "ConnectTimeout=2".to_string(),
                    format!("root@{ip}"),
                    "true".to_string(),
                ]);
                let test_ssh = std::process::Command::new("ssh")
                    .args(&test_args)
                    .status();
                if let Ok(st) = test_ssh {
                    if st.success() {
                        ssh_ready = true;
                        break;
                    }
                }
                sleep(Duration::from_millis(500)).await;
            }
            out_ln!();

            if !ssh_ready {
                bail!("machine acquired IP {ip}, but SSH service did not become ready within 60 seconds.");
            }

            Ok::<IpAddr, anyhow::Error>(ip)
        } => {
            res?
        }
    };

    machine.state = MachineState::Running;
    machine.ip_address = Some(ip);
    machine.pid = Some(child_pid);

    save_machine(&cfg.machines_dir, &machine)?;
    guard.started_successfully = true;

    // Install host bridge CLI `mac` inside guest
    if let Err(e) = crate::bridge::install_guest_mac_cli(ip) {
        eprintln!("[mach] Notice: failed to install guest mac CLI: {e}");
    }

    // Immediately publish native Apple Silicon mDNS domain (<name>.mach.local)
    crate::mdns::register_immediate(&machine.config.name, ip);

    out_ln!(
        "\x1b[1;32m✓\x1b[0m Machine \x1b[1m{}\x1b[0m is UP and RUNNING (PID: {}).",
        machine.config.name, child_pid
    );
    out_ln!("  IP Address:  \x1b[1;34m{}\x1b[0m", ip);
    out_ln!("  Domain:      \x1b[1;34m{}.mach.local\x1b[0m", machine.config.name);
    out_ln!("  SSH Command: \x1b[1;32mssh root@{}\x1b[0m", ip);

    // Mount VirtioFS 1:1 host directory sharing (/Users) if not isolated
    if !machine.config.isolated {
        let mut mount_users_args = base_ssh.clone();
        mount_users_args.extend([
            "-o".to_string(), "ConnectTimeout=5".to_string(),
            format!("root@{ip}"),
            "mkdir -p /Users && (mountpoint -q /Users || mount -t virtiofs mach_share /Users 2>/dev/null || true)".to_string(),
        ]);
        let _ = std::process::Command::new("ssh")
            .args(&mount_users_args)
            .output();
    }

    // Mount and register Apple Rosetta 2 for Linux if available
    let mut rosetta_args = base_ssh.clone();
    rosetta_args.extend([
        "-o".to_string(), "ConnectTimeout=5".to_string(),
        format!("root@{ip}"),
        "mkdir -p /media/rosetta && mount -t virtiofs rosetta /media/rosetta 2>/dev/null; if [ -f /media/rosetta/rosetta ]; then mount -t binfmt_misc binfmt_misc /proc/sys/fs/binfmt_misc 2>/dev/null; echo ':rosetta:M::\\x7fELF\\x02\\x01\\x01\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x02\\x00\\x3e\\x00:\\xff\\xff\\xff\\xff\\xff\\xfe\\xfe\\x00\\xff\\xff\\xff\\xff\\xff\\xff\\xff\\xff\\xfe\\xff\\xff\\xff:/media/rosetta/rosetta:CF' > /proc/sys/fs/binfmt_misc/register 2>/dev/null; echo rosetta_ready; fi".to_string(),
    ]);

    for _ in 0..3 {
        let rosetta_res = std::process::Command::new("ssh")
            .args(&rosetta_args)
            .output();

        if let Ok(out) = rosetta_res {
            if String::from_utf8_lossy(&out.stdout).contains("rosetta_ready") {
                out_ln!("  Rosetta 2:   \x1b[1;32mActive\x1b[0m (x86_64 emulation enabled)");
                break;
            }
        }
        sleep(Duration::from_millis(500)).await;
    }

    // Setup background Docker socket forwarder if Docker is running in this guest
    let mut check_docker_args = base_ssh.clone();
    check_docker_args.extend([
        "-o".to_string(), "ConnectTimeout=2".to_string(),
        format!("root@{ip}"),
        "test -S /var/run/docker.sock".to_string(),
    ]);
    let check_docker = std::process::Command::new("ssh")
        .args(&check_docker_args)
        .status();

    if let Ok(st) = check_docker {
        if st.success() {
            let run_dir = cfg.data_dir.join("run");
            let _ = fs::create_dir_all(&run_dir);
            let host_sock = run_dir.join("docker.sock");
            let _ = fs::remove_file(&host_sock);
            let mut docker_fwd_args = vec!["-f".to_string(), "-nNT".to_string(), "-L".to_string(), format!("{}:/var/run/docker.sock", host_sock.display())];
            docker_fwd_args.extend(base_ssh.clone());
            docker_fwd_args.extend([
                "-o".to_string(), "ConnectTimeout=2".to_string(),
                format!("root@{ip}"),
            ]);
            let _ = std::process::Command::new("ssh")
                .args(&docker_fwd_args)
                .spawn();

            for _ in 0..15 {
                if host_sock.exists() {
                    break;
                }
                sleep(Duration::from_millis(200)).await;
            }

            out_ln!("  Docker:      \x1b[1;34munix://{}\x1b[0m", host_sock.display());
        }
    }

    // Ensure background daemon is running for local DNS and port forwarding
    crate::commands::daemon::ensure_daemon_running();
    crate::commands::menu_cmd::ensure_menu_bar_running();

    // If starting a user machine, restore background services if active in last session
    if !machine.is_system() && (cfg.docker.was_running || cfg.k8s.was_running) {
        let cfg_clone = cfg.clone();
        std::thread::spawn(move || {
            super::restore_services_background(&cfg_clone);
        });
    }

    if !to_stderr {
        println!("\nNext steps:");
        println!("  ➜ To open shell:      \x1b[1;32mmach\x1b[0m (or \x1b[1;32mmach shell {}\x1b[0m)", machine.config.name);
        println!("  ➜ To run commands:    \x1b[1;32mmach uname -a\x1b[0m");
        println!("  ➜ To view dashboard:  \x1b[1;34mmachctl top\x1b[0m");
        println!("  ➜ To manage Docker:   \x1b[1;32mdocker ps\x1b[0m (or \x1b[1;32mmachctl docker setup\x1b[0m)");
    }

    Ok(())
}
