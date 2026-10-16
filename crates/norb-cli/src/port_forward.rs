//! Dynamic port forwarder for Docker containers.
//!
//! Automatically binds `127.0.0.1:PORT` on macOS and forwards TCP connections
//! to the Linux guest VM when a Docker container publishes ports (`-p PORT:PORT`).

use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream, UnixStream};
use tokio::sync::oneshot;
use tokio::time::sleep;

use crate::commands::load_all_machines;

/// Monitor published Docker container ports and forward localhost traffic.
pub async fn run_port_forwarder(docker_sock: &Path, machines_dir: &Path) -> Result<()> {
    let mut active_listeners: HashMap<u16, oneshot::Sender<()>> = HashMap::new();

    loop {
        sleep(Duration::from_millis(1500)).await;

        if !docker_sock.exists() {
            // Clean up any lingering listeners if docker daemon stopped
            active_listeners.clear();
            continue;
        }

        // Find the IP of the running VM
        let machines = match load_all_machines(machines_dir) {
            Ok(m) => m,
            Err(_) => continue,
        };

        let guest_ip = machines
            .iter()
            .find(|m| m.ip_address.is_some() && m.state == norb_common::machine::MachineState::Running)
            .and_then(|m| m.ip_address);

        let guest_ip = match guest_ip {
            Some(ip) => ip,
            None => continue,
        };

        // Query Docker /containers/json via Unix socket
        let ports = match fetch_published_ports(docker_sock).await {
            Ok(p) => p,
            Err(_) => continue,
        };

        let target_ports: HashSet<u16> = ports.into_iter().collect();

        // 1. Remove listeners for closed ports
        active_listeners.retain(|port, cancel_tx| {
            if target_ports.contains(port) {
                true
            } else {
                // Drop or signal cancel
                let _ = cancel_tx;
                false
            }
        });

        // 2. Add listeners for newly exposed ports
        for port in target_ports {
            if !active_listeners.contains_key(&port) {
                let (tx, rx) = oneshot::channel();
                let bind_addr: SocketAddr = SocketAddr::from(([127, 0, 0, 1], port));
                let guest_target: SocketAddr = SocketAddr::new(guest_ip, port);

                if let Ok(listener) = TcpListener::bind(bind_addr).await {
                    active_listeners.insert(port, tx);
                    tokio::spawn(async move {
                        forward_loop(listener, guest_target, rx).await;
                    });
                }
            }
        }
    }
}

async fn fetch_published_ports(docker_sock: &Path) -> Result<Vec<u16>> {
    let mut stream = UnixStream::connect(docker_sock).await?;
    stream
        .write_all(b"GET /containers/json HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await?;

    let mut buf = Vec::new();
    use tokio::io::AsyncReadExt;
    stream.read_to_end(&mut buf).await?;

    // Split headers and body
    let mut split_pos = None;
    for i in 0..buf.len().saturating_sub(3) {
        if &buf[i..i + 4] == b"\r\n\r\n" {
            split_pos = Some(i + 4);
            break;
        }
    }

    let body = match split_pos {
        Some(pos) => &buf[pos..],
        None => return Ok(Vec::new()),
    };

    let containers: Vec<serde_json::Value> = match serde_json::from_slice(body) {
        Ok(c) => c,
        Err(_) => return Ok(Vec::new()),
    };

    let mut published = Vec::new();
    for c in containers {
        if let Some(ports) = c.get("Ports").and_then(|p| p.as_array()) {
            for port_obj in ports {
                let p_type = port_obj.get("Type").and_then(|t| t.as_str()).unwrap_or("");
                if p_type == "tcp" {
                    if let Some(pub_port) = port_obj.get("PublicPort").and_then(|p| p.as_u64()) {
                        if pub_port > 0 && pub_port <= 65535 {
                            published.push(pub_port as u16);
                        }
                    }
                }
            }
        }
    }

    Ok(published)
}

async fn forward_loop(listener: TcpListener, target: SocketAddr, mut cancel_rx: oneshot::Receiver<()>) {
    loop {
        tokio::select! {
            _ = &mut cancel_rx => {
                break;
            }
            accept_res = listener.accept() => {
                let (mut inbound, _) = match accept_res {
                    Ok(res) => res,
                    Err(_) => continue,
                };

                tokio::spawn(async move {
                    if let Ok(mut outbound) = TcpStream::connect(target).await {
                        let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
                    }
                });
            }
        }
    }
}
