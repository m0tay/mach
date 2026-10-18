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

    /// Synchronize mDNS registrations with all currently running machines and Docker containers.
    pub async fn sync(&mut self, machines_dir: &Path, docker_sock: &Path) {
        let mut desired = HashMap::new();

        // 1. Host and Gateway aliases (192.168.64.1)
        if let Ok(gw_ip) = "192.168.64.1".parse::<IpAddr>() {
            desired.insert("gateway.mach.local".to_string(), ("gateway".to_string(), gw_ip));
            desired.insert("host.mach.local".to_string(), ("host".to_string(), gw_ip));
        }

        // 2. Discover running machines from disk
        let mut docker_ip = None;
        if let Ok(machines) = load_all_machines(machines_dir) {
            for m in &machines {
                if m.state == MachineState::Running {
                    if let Some(ip) = m.ip_address {
                        let domain = format!("{}.mach.local", m.config.name);
                        desired.insert(domain, (m.config.name.clone(), ip));

                        // Provide k8s.mach.local alias for Kubernetes node
                        if m.config.name == "mach-kubernetes" || m.config.name == "mach-k8s" {
                            desired.insert("k8s.mach.local".to_string(), ("k8s".to_string(), ip));
                        }

                        if m.config.name == "mach-system" || m.config.name == "mach-docker" {
                            docker_ip = Some(ip);
                        } else if docker_ip.is_none() {
                            docker_ip = Some(ip);
                        }
                    }
                }
            }
        }

        // 3. Discover running Docker containers from docker.sock
        if let Some(ip) = docker_ip {
            let container_domains = fetch_docker_container_domains(docker_sock).await;
            for (instance, domain) in container_domains {
                desired.insert(domain, (instance, ip));
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

/// Query Docker daemon Unix socket to extract domain names for active containers.
async fn fetch_docker_container_domains(docker_sock: &Path) -> Vec<(String, String)> {
    if !docker_sock.exists() {
        return Vec::new();
    }
    let mut stream = match tokio::net::UnixStream::connect(docker_sock).await {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    if stream
        .write_all(b"GET /containers/json HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .is_err()
    {
        return Vec::new();
    }
    let mut buf = Vec::new();
    if stream.read_to_end(&mut buf).await.is_err() {
        return Vec::new();
    }

    let split_pos = buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4);
    let (headers, body) = match split_pos {
        Some(pos) => (&buf[..pos], &buf[pos..]),
        None => return Vec::new(),
    };

    let is_chunked = String::from_utf8_lossy(headers)
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked");
    let body_bytes = if is_chunked {
        crate::proxy::decode_chunked_body(body)
    } else {
        body.to_vec()
    };

    let containers: serde_json::Value = match serde_json::from_slice(&body_bytes) {
        Ok(v) => v,
        Err(_) => {
            let fallback = crate::proxy::decode_chunked_body(body);
            match serde_json::from_slice(&fallback) {
                Ok(v) => v,
                Err(_) => return Vec::new(),
            }
        }
    };

    let mut domains = Vec::new();
    if let serde_json::Value::Array(list) = containers {
        for c in list {
            if let Some(names) = c.get("Names").and_then(|v| v.as_array()) {
                if let Some(first_name) = names.first().and_then(|v| v.as_str()) {
                    let clean = first_name.trim_start_matches('/');
                    if !clean.is_empty() {
                        domains.push((format!("mach-{}", clean), format!("{}.mach.local", clean)));
                    }
                }
            }
            if let Some(img) = c.get("Image").and_then(|v| v.as_str()) {
                let repo = img.split(':').next().unwrap_or(img);
                let clean_repo = repo.rsplit('/').next().unwrap_or(repo);
                if !clean_repo.is_empty() {
                    domains.push((format!("mach-{}", clean_repo), format!("{}.mach.local", clean_repo)));
                }
            }
            if let Some(labels) = c.get("Labels") {
                let svc = labels.get("com.docker.compose.service").and_then(|s| s.as_str());
                let proj = labels.get("com.docker.compose.project").and_then(|s| s.as_str());
                if let (Some(s), Some(p)) = (svc, proj) {
                    domains.push((format!("mach-{}-{}", p, s), format!("{}.{}.mach.local", s, p)));
                    domains.push((format!("mach-{}", s), format!("{}.mach.local", s)));
                }
            }
        }
    }
    domains
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
pub async fn run_mdns_loop(machines_dir: &Path, docker_sock: &Path) {
    let mut publisher = MdnsPublisher::new();
    loop {
        publisher.sync(machines_dir, docker_sock).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
