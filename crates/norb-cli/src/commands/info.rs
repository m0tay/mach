//! `norb info` command implementation.

use anyhow::{bail, Context, Result};
use nix::sys::signal::kill;
use nix::unistd::Pid;
use norb_common::config::NorbConfig;
use norb_common::machine::MachineState;

use super::{find_machine, load_all_machines};

#[cfg(target_os = "macos")]
fn get_proc_rss_mib(pid: u32) -> Option<u64> {
    unsafe {
        let mut info: libc::proc_taskinfo = std::mem::zeroed();
        let size = std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int;
        let ret = libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTASKINFO,
            0,
            &mut info as *mut _ as *mut libc::c_void,
            size,
        );
        if ret == size {
            Some(info.pti_resident_size / (1024 * 1024))
        } else {
            None
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn get_proc_rss_mib(_pid: u32) -> Option<u64> {
    None
}

pub async fn run(name: Option<&str>) -> Result<()> {
    let cfg = NorbConfig::load()?;

    let target_name = match name {
        Some(n) => n.to_string(),
        None => {
            let machines = load_all_machines(&cfg.machines_dir)?;
            if machines.is_empty() {
                bail!("no machines found");
            }
            machines[0].config.name.clone()
        }
    };

    let machine = find_machine(&cfg.machines_dir, &target_name)?
        .with_context(|| format!("machine '{target_name}' not found"))?;

    // 1. Real VMM Process status
    let pid_file = cfg.machines_dir.join(&machine.config.name).join("pid");
    let pid_opt = machine.pid.or_else(|| {
        std::fs::read_to_string(&pid_file)
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
    });

    let vmm_status = match pid_opt {
        Some(pid) => {
            if kill(Pid::from_raw(pid as i32), None).is_ok() {
                let rss_str = get_proc_rss_mib(pid)
                    .map(|rss| format!(", Resident Memory: {rss} MiB"))
                    .unwrap_or_default();
                format!("\x1b[1;32mRunning\x1b[0m (PID: {pid}{rss_str})")
            } else {
                format!("\x1b[1;31mDead\x1b[0m (PID: {pid} terminated unexpectedly)")
            }
        }
        None => "\x1b[2mNone (stopped)\x1b[0m".to_string(),
    };

    // 2. Host disk space consumed (st_blocks * 512)
    let disk_path = cfg.machines_dir.join(&machine.config.name).join("disk.raw");
    let disk_str = if let Ok(meta) = std::fs::metadata(&disk_path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let consumed_bytes = meta.blocks() * 512;
            let consumed_mib = consumed_bytes as f64 / (1024.0 * 1024.0);
            if consumed_mib >= 1024.0 {
                format!(
                    "{} GiB (Host allocated: {:.2} GiB, {} APFS CoW blocks)",
                    machine.config.disk_gb,
                    consumed_mib / 1024.0,
                    meta.blocks()
                )
            } else {
                format!(
                    "{} GiB (Host allocated: {:.2} MiB, {} APFS CoW blocks)",
                    machine.config.disk_gb,
                    consumed_mib,
                    meta.blocks()
                )
            }
        }
        #[cfg(not(unix))]
        {
            format!("{} GiB", machine.config.disk_gb)
        }
    } else {
        format!("{} GiB", machine.config.disk_gb)
    };

    // 3. Machine network status and MAC address
    let bytes = machine.id.as_bytes();
    let b1 = bytes[14] & 0x7f;
    let b2 = bytes[15];
    let mac_str = format!("52:54:00:12:{:02x}:{:02x}", b1, b2);

    let net_status = match (&machine.state, machine.ip_address) {
        (MachineState::Running, Some(ip)) => {
            format!("\x1b[1;32mActive\x1b[0m (IP: {ip}, MAC: {mac_str})")
        }
        (MachineState::Running, None) => {
            format!("\x1b[1;33mAcquiring DHCP lease\x1b[0m (MAC: {mac_str})")
        }
        _ => format!("\x1b[2mDisconnected\x1b[0m (MAC: {mac_str})"),
    };

    println!("\x1b[1mMachine Information\x1b[0m");
    println!("─────────────────────────────────────────────────────────────────────────────");
    println!("  Name:         \x1b[1m{}\x1b[0m", machine.config.name);
    println!("  UUID:         {}", machine.id);
    println!("  Distribution: {}", machine.config.distro);
    println!("  Architecture: {}", machine.config.arch);
    println!("  State:        {}", machine.state);
    println!("  VMM Process:  {}", vmm_status);
    println!("  CPUs:         {}", machine.config.cpus);
    println!("  Memory:       {} MiB", machine.config.memory_mb);
    println!("  Disk:         {}", disk_str);
    println!("  Network:      {}", net_status);
    println!(
        "  IP Address:   {}",
        machine
            .ip_address
            .map(|ip| ip.to_string())
            .unwrap_or_else(|| "none (stopped)".into())
    );
    println!("  MAC Address:  {}", mac_str);
    println!("  Domain:       {}.norb.local", machine.config.name);
    println!("  Created:      {}", machine.created_at.to_rfc2822());

    Ok(())
}
