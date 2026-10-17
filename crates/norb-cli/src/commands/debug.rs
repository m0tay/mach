//! `norbctl debug` command implementation for container diagnostics.
//!
//! Provides an interactive debugging shell attached directly to a target container's
//! network, PID, and IPC namespaces. Works even on distroless or scratch containers.

use anyhow::{bail, Context, Result};
use std::process::Command;

/// Diagnostic metadata parsed from `docker inspect`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerMetadata {
    pub name: String,
    pub short_id: String,
    pub image: String,
    pub ip: String,
    pub ports: Vec<String>,
    pub env: Vec<String>,
    pub is_running: bool,
}

impl ContainerMetadata {
    /// Parse container metadata from `docker inspect` JSON output.
    pub fn parse(inspect_json: &serde_json::Value, fallback_name: &str) -> Result<Self> {
        let item = inspect_json
            .get(0)
            .context(format!("No inspection records returned for container '{fallback_name}'"))?;

        let raw_name = item.get("Name").and_then(|v| v.as_str()).unwrap_or(fallback_name);
        let name = raw_name.trim_start_matches('/').to_string();

        let id_str = item.get("Id").and_then(|v| v.as_str()).unwrap_or("");
        let short_id = if id_str.len() > 12 {
            id_str[..12].to_string()
        } else {
            id_str.to_string()
        };

        let image = item
            .pointer("/Config/Image")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .or_else(|| item.get("Image").and_then(|v| v.as_str()))
            .unwrap_or("unknown")
            .to_string();

        let is_running = item
            .pointer("/State/Running")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Resolve IP address (top-level NetworkSettings.IPAddress or iterate NetworkSettings.Networks)
        let mut ips = Vec::new();
        if let Some(ip) = item
            .pointer("/NetworkSettings/IPAddress")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            ips.push(ip.to_string());
        }
        if let Some(networks) = item.pointer("/NetworkSettings/Networks").and_then(|v| v.as_object()) {
            for (net_name, net_info) in networks {
                if let Some(net_ip) = net_info.get("IPAddress").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                    let entry = format!("{net_ip} ({net_name})");
                    if !ips.contains(&net_ip.to_string()) && !ips.contains(&entry) {
                        ips.push(entry);
                    }
                }
            }
        }
        let ip = if ips.is_empty() {
            let net_mode = item
                .pointer("/HostConfig/NetworkMode")
                .and_then(|v| v.as_str())
                .unwrap_or("none");
            format!("none (network_mode: {net_mode})")
        } else {
            ips.join(", ")
        };

        // Resolve published and exposed ports
        let mut ports = Vec::new();
        if let Some(port_map) = item.pointer("/NetworkSettings/Ports").and_then(|v| v.as_object()) {
            for (container_port, bindings) in port_map {
                if let Some(arr) = bindings.as_array() {
                    if arr.is_empty() {
                        ports.push(format!("{container_port} (exposed)"));
                    } else {
                        for b in arr {
                            let host_ip = b.get("HostIp").and_then(|v| v.as_str()).unwrap_or("0.0.0.0");
                            let host_port = b.get("HostPort").and_then(|v| v.as_str()).unwrap_or("?");
                            ports.push(format!("{container_port} -> {host_ip}:{host_port}"));
                        }
                    }
                } else {
                    ports.push(format!("{container_port} (exposed)"));
                }
            }
        }
        if ports.is_empty() {
            if let Some(exposed) = item.pointer("/Config/ExposedPorts").and_then(|v| v.as_object()) {
                for container_port in exposed.keys() {
                    ports.push(format!("{container_port} (exposed)"));
                }
            }
        }
        ports.sort();

        // Resolve environment variables
        let mut env = Vec::new();
        if let Some(env_arr) = item.pointer("/Config/Env").and_then(|v| v.as_array()) {
            for e in env_arr {
                if let Some(s) = e.as_str() {
                    env.push(s.to_string());
                }
            }
        }
        env.sort();

        Ok(Self {
            name,
            short_id,
            image,
            ip,
            ports,
            env,
            is_running,
        })
    }

    /// Print formatted container metadata to standard output.
    pub fn print_summary(&self) {
        println!();
        println!("\x1b[1;34m── Container Diagnostic Metadata ───────────────────────────────\x1b[0m");
        if self.short_id.is_empty() {
            println!("  \x1b[1mContainer:\x1b[0m    \x1b[1;36m{}\x1b[0m", self.name);
        } else {
            println!(
                "  \x1b[1mContainer:\x1b[0m    \x1b[1;36m{}\x1b[0m \x1b[2m({})\x1b[0m",
                self.name, self.short_id
            );
        }
        println!("  \x1b[1mImage:\x1b[0m        \x1b[1;33m{}\x1b[0m", self.image);
        println!("  \x1b[1mIP Address:\x1b[0m   \x1b[1;32m{}\x1b[0m", self.ip);
        if self.ports.is_empty() {
            println!("  \x1b[1mPorts:\x1b[0m        \x1b[2m(none)\x1b[0m");
        } else {
            println!("  \x1b[1mPorts:\x1b[0m        {}", self.ports.join(", "));
        }
        println!("  \x1b[1mEnvironment:\x1b[0m");
        if self.env.is_empty() {
            println!("    \x1b[2m(none)\x1b[0m");
        } else {
            for env_entry in &self.env {
                if let Some((k, v)) = env_entry.split_once('=') {
                    println!("    \x1b[2m•\x1b[0m \x1b[1m{k}\x1b[0m={v}");
                } else {
                    println!("    \x1b[2m•\x1b[0m {env_entry}");
                }
            }
        }
        println!("\x1b[1;34m────────────────────────────────────────────────────────────────\x1b[0m");
        println!();
    }
}

/// Debug container candidate image configuration.
struct DebugCandidate {
    image: &'static str,
    cmd: Option<&'static str>,
}

/// Priority-ordered candidate images for the debug container.
const CANDIDATES: &[DebugCandidate] = &[
    DebugCandidate {
        image: "nicolaka/netshoot",
        cmd: None, // defaults to interactive diagnostic shell with all networking tools loaded
    },
    DebugCandidate {
        image: "busybox",
        cmd: Some("sh"),
    },
    DebugCandidate {
        image: "alpine",
        cmd: Some("sh"),
    },
];

pub async fn run(container: &str) -> Result<()> {
    println!("\x1b[1;36m➜\x1b[0m Inspecting target container \x1b[1;34m{container}\x1b[0m...");

    // 1. Inspect container
    let inspect_out = Command::new("docker")
        .args(["inspect", container])
        .output()
        .context("Failed to execute `docker inspect`. Is Docker context 'norb' active?")?;

    if !inspect_out.status.success() {
        let err = String::from_utf8_lossy(&inspect_out.stderr);
        bail!("Container '{}' not found: {}", container, err.trim());
    }

    let inspect_json: serde_json::Value = serde_json::from_slice(&inspect_out.stdout)
        .context("Failed to parse `docker inspect` output as JSON")?;

    let meta = ContainerMetadata::parse(&inspect_json, container)?;

    if !meta.is_running {
        bail!("Container '{}' is not currently running. Cannot attach debug shell.", container);
    }

    println!("\x1b[1;32m✓\x1b[0m Container is running.");

    // 2. Print container metadata (env, ports, image, IP) before dropping into interactive shell
    meta.print_summary();

    println!("\x1b[1;36m➜\x1b[0m Attaching diagnostic shell (PID, Net, IPC namespaces joined)...");
    println!("  Target root filesystem mounted at: \x1b[1;33m/host\x1b[0m");
    println!("  Type 'exit' to leave debug shell.\n");

    // 3. Connect to target container namespaces using nicolaka/netshoot with fallback to busybox or alpine
    let mut session_started = false;
    for (idx, cand) in CANDIDATES.iter().enumerate() {
        println!(
            "\x1b[1;36m➜\x1b[0m Spawning debug container with \x1b[1;35m{}\x1b[0m...",
            cand.image
        );

        let mut cmd = Command::new("docker");
        cmd.args([
            "run",
            "--rm",
            "-it",
            &format!("--net=container:{container}"),
            &format!("--pid=container:{container}"),
            &format!("--ipc=container:{container}"),
            "-v",
            "/:/host:ro",
            cand.image,
        ]);
        if let Some(shell_cmd) = cand.cmd {
            cmd.arg(shell_cmd);
        }

        match cmd.status() {
            Ok(status) if status.success() => {
                session_started = true;
                break;
            }
            Ok(status) => {
                let code_str = status
                    .code()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "terminated by signal".into());
                if let Some(next) = CANDIDATES.get(idx + 1) {
                    eprintln!(
                        "\x1b[1;33m!\x1b[0m Debug container '{}' exited with status {}. Falling back to '{}'...",
                        cand.image, code_str, next.image
                    );
                } else {
                    eprintln!(
                        "\x1b[1;31m✘\x1b[0m Debug container '{}' exited with status {}.",
                        cand.image, code_str
                    );
                }
            }
            Err(e) => {
                if let Some(next) = CANDIDATES.get(idx + 1) {
                    eprintln!(
                        "\x1b[1;33m!\x1b[0m Failed to run '{}': {}. Falling back to '{}'...",
                        cand.image, e, next.image
                    );
                } else {
                    eprintln!(
                        "\x1b[1;31m✘\x1b[0m Failed to run '{}': {}",
                        cand.image, e
                    );
                }
            }
        }
    }

    if !session_started {
        bail!("Failed to start debug session using any candidate image (nicolaka/netshoot, busybox, alpine).");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_metadata_full() {
        let inspect_data = json!([
            {
                "Id": "abc123456789def012345678",
                "Name": "/my-web-app",
                "State": {
                    "Running": true
                },
                "Config": {
                    "Image": "nginx:alpine",
                    "Env": [
                        "NGINX_VERSION=1.25.3",
                        "PORT=80"
                    ]
                },
                "NetworkSettings": {
                    "IPAddress": "172.17.0.2",
                    "Ports": {
                        "80/tcp": [
                            {
                                "HostIp": "0.0.0.0",
                                "HostPort": "8080"
                            }
                        ],
                        "443/tcp": null
                    }
                }
            }
        ]);

        let meta = ContainerMetadata::parse(&inspect_data, "my-web-app").unwrap();
        assert_eq!(meta.name, "my-web-app");
        assert_eq!(meta.short_id, "abc123456789");
        assert_eq!(meta.image, "nginx:alpine");
        assert_eq!(meta.ip, "172.17.0.2");
        assert_eq!(
            meta.ports,
            vec!["443/tcp (exposed)", "80/tcp -> 0.0.0.0:8080"]
        );
        assert_eq!(meta.env, vec!["NGINX_VERSION=1.25.3", "PORT=80"]);
        assert!(meta.is_running);
    }

    #[test]
    fn test_parse_metadata_custom_network() {
        let inspect_data = json!([
            {
                "Id": "1122334455667788",
                "Name": "/redis-svc",
                "State": {
                    "Running": true
                },
                "Config": {
                    "Image": "redis:7.2",
                    "Env": []
                },
                "NetworkSettings": {
                    "IPAddress": "",
                    "Networks": {
                        "backend_net": {
                            "IPAddress": "10.0.1.5"
                        }
                    },
                    "Ports": {}
                }
            }
        ]);

        let meta = ContainerMetadata::parse(&inspect_data, "redis-svc").unwrap();
        assert_eq!(meta.name, "redis-svc");
        assert_eq!(meta.short_id, "112233445566");
        assert_eq!(meta.image, "redis:7.2");
        assert_eq!(meta.ip, "10.0.1.5 (backend_net)");
        assert!(meta.ports.is_empty());
        assert!(meta.env.is_empty());
        assert!(meta.is_running);
    }

    #[test]
    fn test_parse_metadata_not_running() {
        let inspect_data = json!([
            {
                "Id": "stopped12345678",
                "Name": "/stopped-container",
                "State": {
                    "Running": false
                },
                "Config": {
                    "Image": "busybox"
                }
            }
        ]);

        let meta = ContainerMetadata::parse(&inspect_data, "stopped-container").unwrap();
        assert!(!meta.is_running);
    }

    #[test]
    fn test_candidates_order() {
        assert_eq!(CANDIDATES[0].image, "nicolaka/netshoot");
        assert_eq!(CANDIDATES[1].image, "busybox");
        assert_eq!(CANDIDATES[2].image, "alpine");
    }
}
