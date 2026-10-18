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
        sleep(Duration::from_millis(500)).await;

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
            .find(|m| (m.config.name == "mach-system" || m.config.name == "mach-docker") && m.state == mach_common::machine::MachineState::Running && m.ip_address.is_some())
            .or_else(|| machines.iter().find(|m| m.ip_address.is_some() && m.state == mach_common::machine::MachineState::Running))
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

/// Decode HTTP/1.1 Transfer-Encoding: chunked body.
/// Returns decoded bytes, or falls back to raw slice if not valid chunked encoding.
pub fn decode_chunked_body(raw: &[u8]) -> Vec<u8> {
    let mut cursor = 0;
    let mut decoded = Vec::new();

    while cursor < raw.len() {
        // Find CRLF or LF marking end of chunk size line
        let line_end = match raw[cursor..].windows(2).position(|w| w == b"\r\n") {
            Some(pos) => cursor + pos,
            None => match raw[cursor..].iter().position(|&b| b == b'\n') {
                Some(pos) => cursor + pos,
                None => {
                    if decoded.is_empty() {
                        return raw.to_vec();
                    }
                    break;
                }
            },
        };

        let chunk_header = &raw[cursor..line_end];
        let chunk_header_str = match std::str::from_utf8(chunk_header) {
            Ok(s) => s.trim(),
            Err(_) => {
                if decoded.is_empty() {
                    return raw.to_vec();
                }
                break;
            }
        };

        // Chunk extensions might follow ';' (e.g. "1f;foo=bar")
        let size_str = chunk_header_str.split(';').next().unwrap_or("").trim();
        let chunk_size = match usize::from_str_radix(size_str, 16) {
            Ok(sz) => sz,
            Err(_) => {
                if decoded.is_empty() {
                    return raw.to_vec();
                }
                break;
            }
        };

        // Advance cursor past the chunk header line
        cursor = line_end;
        if cursor + 2 <= raw.len() && &raw[cursor..cursor + 2] == b"\r\n" {
            cursor += 2;
        } else if cursor < raw.len() && raw[cursor] == b'\n' {
            cursor += 1;
        }

        // 0-sized chunk signals end of chunked stream
        if chunk_size == 0 {
            break;
        }

        // Read chunk data
        if cursor + chunk_size > raw.len() {
            // Incomplete chunk: take whatever remains
            decoded.extend_from_slice(&raw[cursor..]);
            break;
        }

        decoded.extend_from_slice(&raw[cursor..cursor + chunk_size]);
        cursor += chunk_size;

        // Skip the chunk-closing CRLF / LF
        if cursor + 2 <= raw.len() && &raw[cursor..cursor + 2] == b"\r\n" {
            cursor += 2;
        } else if cursor < raw.len() && raw[cursor] == b'\n' {
            cursor += 1;
        }
    }

    if decoded.is_empty() && !raw.is_empty() {
        raw.to_vec()
    } else {
        decoded
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

    let (headers, body) = match split_pos {
        Some(pos) => (&buf[..pos], &buf[pos..]),
        None => return Ok(Vec::new()),
    };

    let is_chunked = String::from_utf8_lossy(headers)
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked");

    let body_bytes = if is_chunked {
        decode_chunked_body(body)
    } else {
        let trimmed = body.iter().find(|&&b| !b.is_ascii_whitespace()).copied();
        if trimmed == Some(b'[') || trimmed == Some(b'{') {
            body.to_vec()
        } else {
            decode_chunked_body(body)
        }
    };

    let containers: Vec<serde_json::Value> = match serde_json::from_slice(&body_bytes) {
        Ok(c) => c,
        Err(_) => {
            let fallback = decode_chunked_body(body);
            match serde_json::from_slice(&fallback) {
                Ok(c) => c,
                Err(_) => return Ok(Vec::new()),
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_chunked_body_basic() {
        let chunked = b"4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n";
        let decoded = decode_chunked_body(chunked);
        assert_eq!(decoded, b"Wikipedia");
    }

    #[test]
    fn test_decode_chunked_body_with_extension() {
        let chunked = b"5;foo=bar\r\nhello\r\n0\r\n\r\n";
        let decoded = decode_chunked_body(chunked);
        assert_eq!(decoded, b"hello");
    }

    #[test]
    fn test_decode_unchunked_fallback() {
        let raw = b"[{\"Id\":\"test-container\"}]";
        let decoded = decode_chunked_body(raw);
        assert_eq!(decoded, raw);
    }
}

