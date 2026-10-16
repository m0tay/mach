//! Embedded lightweight UDP DNS responder for Norb.
//!
//! Handles local DNS queries:
//! - `<machine>.norb.local` -> Machine's IPv4 address
//! - `host.norb.internal` / `host.docker.internal` -> Host gateway IP (192.168.64.1)

use anyhow::{Context, Result};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
use tokio::net::UdpSocket;

use crate::commands::load_all_machines;

pub const DNS_PORT: u16 = 5354;

/// Run the DNS responder loop asynchronously.
pub async fn run_dns_server(machines_dir: &Path) -> Result<()> {
    let bind_addr = SocketAddr::from(([127, 0, 0, 1], DNS_PORT));
    let socket = UdpSocket::bind(bind_addr)
        .await
        .with_context(|| format!("failed to bind DNS responder to {bind_addr}"))?;

    let mut buf = [0u8; 1024];

    loop {
        let (len, peer) = match socket.recv_from(&mut buf).await {
            Ok(res) => res,
            Err(_) => continue,
        };

        if len < 12 {
            continue;
        }

        if let Some(resp) = handle_dns_query(&buf[..len], machines_dir) {
            let _ = socket.send_to(&resp, peer).await;
        }
    }
}

/// Parse query and synthesize DNS answer packet.
pub fn handle_dns_query(packet: &[u8], machines_dir: &Path) -> Option<Vec<u8>> {
    let id = u16::from_be_bytes([packet[0], packet[1]]);
    let qdcount = u16::from_be_bytes([packet[4], packet[5]]);
    if qdcount == 0 {
        return None;
    }

    // Parse QNAME
    let mut pos = 12;
    let mut labels = Vec::new();
    while pos < packet.len() {
        let len = packet[pos] as usize;
        if len == 0 {
            pos += 1;
            break;
        }
        pos += 1;
        if pos + len > packet.len() {
            return None;
        }
        let label = std::str::from_utf8(&packet[pos..pos + len]).ok()?;
        labels.push(label.to_lowercase());
        pos += len;
    }

    if pos + 4 > packet.len() {
        return None;
    }
    let qtype = u16::from_be_bytes([packet[pos], packet[pos + 1]]);
    let qend = pos + 4;

    let domain = labels.join(".");
    let target_ip: Option<Ipv4Addr> = if domain.ends_with(".norb.local") || domain == "norb.local" {
        let prefix = labels.first().map(|s| s.as_str()).unwrap_or("");
        if prefix == "host" || prefix == "gateway" {
            Some(Ipv4Addr::new(192, 168, 64, 1))
        } else {
            let machines = load_all_machines(machines_dir).ok()?;
            machines
                .into_iter()
                .find(|m| m.config.name.eq_ignore_ascii_case(prefix))
                .and_then(|m| m.ip_address)
                .and_then(|ip| match ip {
                    std::net::IpAddr::V4(v4) => Some(v4),
                    _ => None,
                })
        }
    } else if domain == "host.docker.internal" || domain == "host.norb.internal" {
        Some(Ipv4Addr::new(192, 168, 64, 1))
    } else {
        None
    };

    let mut resp = Vec::with_capacity(512);
    // Header
    resp.extend_from_slice(&id.to_be_bytes()); // ID

    if let Some(ip) = target_ip {
        if qtype == 1 || qtype == 255 {
            // Standard query response, No error, Recursion Desired + Available
            resp.extend_from_slice(&[0x81, 0x80]); // Flags
            resp.extend_from_slice(&[0x00, 0x01]); // QDCOUNT: 1
            resp.extend_from_slice(&[0x00, 0x01]); // ANCOUNT: 1
            resp.extend_from_slice(&[0x00, 0x00]); // NSCOUNT: 0
            resp.extend_from_slice(&[0x00, 0x00]); // ARCOUNT: 0

            // Copy Question
            resp.extend_from_slice(&packet[12..qend]);

            // Answer
            resp.extend_from_slice(&[0xc0, 0x0c]); // Pointer to QNAME
            resp.extend_from_slice(&[0x00, 0x01]); // Type A
            resp.extend_from_slice(&[0x00, 0x01]); // Class IN
            resp.extend_from_slice(&[0x00, 0x00, 0x00, 0x0a]); // TTL 10 seconds
            resp.extend_from_slice(&[0x00, 0x04]); // RDLENGTH 4
            resp.extend_from_slice(&ip.octets()); // RDATA
            return Some(resp);
        } else if qtype == 28 {
            // AAAA query for valid name -> NODATA response (NOERROR, 0 answers)
            resp.extend_from_slice(&[0x81, 0x80]);
            resp.extend_from_slice(&[0x00, 0x01]); // QDCOUNT: 1
            resp.extend_from_slice(&[0x00, 0x00]); // ANCOUNT: 0
            resp.extend_from_slice(&[0x00, 0x00]); // NSCOUNT: 0
            resp.extend_from_slice(&[0x00, 0x00]); // ARCOUNT: 0
            resp.extend_from_slice(&packet[12..qend]);
            return Some(resp);
        }
    }

    // Name error (NXDOMAIN: 0x8183)
    resp.extend_from_slice(&[0x81, 0x83]);
    resp.extend_from_slice(&[0x00, 0x01]); // QDCOUNT: 1
    resp.extend_from_slice(&[0x00, 0x00]); // ANCOUNT: 0
    resp.extend_from_slice(&[0x00, 0x00]); // NSCOUNT: 0
    resp.extend_from_slice(&[0x00, 0x00]); // ARCOUNT: 0
    resp.extend_from_slice(&packet[12..qend]);
    Some(resp)
}

/// Helper to verify or configure macOS `/etc/resolver/norb.local`.
pub fn check_or_setup_resolver() -> (bool, Option<String>) {
    let resolver_path = Path::new("/etc/resolver/norb.local");
    if resolver_path.exists() {
        if let Ok(content) = std::fs::read_to_string(resolver_path) {
            if content.contains("5354") {
                return (true, None);
            }
        }
    }

    let setup_cmd = "sudo mkdir -p /etc/resolver && sudo tee /etc/resolver/norb.local <<< 'nameserver 127.0.0.1\nport 5354'".to_string();
    (false, Some(setup_cmd))
}
