//! Native macOS Multicast DNS (mDNS / Bonjour) publisher for `*.mach.local`.
//!
//! Automatically publishes DNS-SD proxy address records for running Mach virtual machines
//! and system infrastructure directly to macOS `mDNSResponder`.
//!
//! This enables instant, zero-configuration name resolution across all of macOS:
//! - `ping <machine>.mach.local`
//! - `ssh <machine>.mach.local`
//! - `curl http://<machine>.mach.local`
//! - Web browsers (Safari, Chrome, etc.)
//!
//! Operates completely in user-space without requiring root permissions, sudo,
//! or modifications to `/etc/hosts` or `/etc/resolver`.

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use crate::commands::load_all_machines;
use mach_common::machine::MachineState;

/// Clean up any lingering or orphaned `dns-sd -P` processes from previous sessions.
pub fn kill_orphan_dns_sd() {
    if let Ok(output) = Command::new("ps").args(["-eo", "pid,command"]).output() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.contains("dns-sd") && line.contains("-P") && line.contains("mach.local") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if let Some(pid_str) = parts.first() {
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

/// Immediately publish a domain record for a booting VM, ensuring zero lookup latency.
pub fn register_immediate(instance_name: &str, ip: IpAddr) {
    let domain = format!("{instance_name}.mach.local");
    let _ = Command::new("/usr/bin/dns-sd")
        .args([
            "-P",
            instance_name,
            "_workstation._tcp",
            "local",
            "22",
            &domain,
            &ip.to_string(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

pub struct MdnsPublisher {
    /// Active dns-sd processes keyed by domain name (e.g. "debian.mach.local")
    processes: HashMap<String, (IpAddr, Child)>,
}

impl Default for MdnsPublisher {
    fn default() -> Self {
        Self {
            processes: HashMap::new(),
        }
    }
}

impl MdnsPublisher {
    pub fn new() -> Self {
        kill_orphan_dns_sd();
        Self::default()
    }

    /// Register or refresh a domain mapping to an IP address.
    pub fn register(&mut self, instance_name: &str, domain: &str, ip: IpAddr) {
        if let Some((current_ip, child)) = self.processes.get_mut(domain) {
            if *current_ip == ip {
                if let Ok(None) = child.try_wait() {
                    return;
                }
            }
            let _ = child.kill();
            let _ = child.wait();
        }

        let child = Command::new("/usr/bin/dns-sd")
            .args([
                "-P",
                instance_name,
                "_workstation._tcp",
                "local",
                "22",
                domain,
                &ip.to_string(),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();

        match child {
            Ok(c) => {
                self.processes.insert(domain.to_string(), (ip, c));
            }
            Err(e) => {
                eprintln!("[mach-mdns] Failed to register {} via dns-sd: {e}", domain);
            }
        }
    }

    /// Unregister a domain by terminating its child dns-sd process.
    pub fn unregister(&mut self, domain: &str) {
        if let Some((_, mut child)) = self.processes.remove(domain) {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// Synchronize mDNS registrations with all currently running machines.
    pub fn sync(&mut self, machines_dir: &Path) {
        let mut desired = HashMap::new();

        // 1. Host and Gateway aliases (192.168.64.1)
        if let Ok(gw_ip) = "192.168.64.1".parse::<IpAddr>() {
            desired.insert("gateway.mach.local".to_string(), ("gateway".to_string(), gw_ip));
            desired.insert("host.mach.local".to_string(), ("host".to_string(), gw_ip));
        }

        // 2. Discover running machines from disk
        if let Ok(machines) = load_all_machines(machines_dir) {
            for m in machines {
                if m.state == MachineState::Running {
                    if let Some(ip) = m.ip_address {
                        let domain = format!("{}.mach.local", m.config.name);
                        desired.insert(domain, (m.config.name.clone(), ip));

                        // Provide k8s.mach.local alias for Kubernetes node
                        if m.config.name == "mach-kubernetes" || m.config.name == "mach-k8s" {
                            desired.insert("k8s.mach.local".to_string(), ("k8s".to_string(), ip));
                        }
                    }
                }
            }
        }

        // Remove domains that are no longer active
        let to_remove: Vec<String> = self
            .processes
            .keys()
            .filter(|dom| !desired.contains_key(*dom))
            .cloned()
            .collect();

        for dom in to_remove {
            self.unregister(&dom);
        }

        // Register or keep active domains
        for (domain, (instance, ip)) in desired {
            self.register(&instance, &domain, ip);
        }
    }
}

impl Drop for MdnsPublisher {
    fn drop(&mut self) {
        for (_, (_, mut child)) in self.processes.drain() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Run the background mDNS synchronization loop.
pub async fn run_mdns_loop(machines_dir: &Path) {
    let mut publisher = MdnsPublisher::new();
    loop {
        publisher.sync(machines_dir);
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
