//! Embedded lightweight UDP DNS responder for Mach.
//!
//! Handles local DNS queries:
//! - `<machine>.mach.local` -> Machine's IPv4 address
//! - `host.mach.local` / `gateway.mach.local` -> Host gateway IP (192.168.64.1)
//! - `web.app.mach.local` / `<container>.mach.local` -> Reverse proxy IP (127.0.0.1)
//! - `host.mach.internal` / `host.docker.internal` -> Host gateway IP (192.168.64.1)
//! - Reverse PTR: `1.64.168.192.in-addr.arpa` -> `gateway.mach.local`
//! - Reverse PTR: `<x>.64.168.192.in-addr.arpa` -> `<machine>.mach.local`

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::RwLock;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;

use crate::commands::load_all_machines;

pub const DNS_PORT: u16 = 5354;
pub const DNS_CACHE_TTL: Duration = Duration::from_secs(2);

/// In-memory cache entry for machine names to IPv4 addresses and reverse lookups.
#[derive(Debug, Clone)]
pub struct DnsCacheEntry {
    pub last_updated: Instant,
    pub machines: HashMap<String, Ipv4Addr>,
    pub reverse: HashMap<Ipv4Addr, String>,
}

/// Global thread-safe in-memory cache with 2-second TTL.
pub static DNS_CACHE: RwLock<Option<DnsCacheEntry>> = RwLock::new(None);

/// Refresh the machine DNS cache from disk if the TTL has expired.
fn refresh_cache_if_needed(machines_dir: &Path) {
    let now = Instant::now();

    // Fast path: check valid cache under read lock
    {
        let guard = match DNS_CACHE.read() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        if let Some(ref cache) = *guard {
            if now.duration_since(cache.last_updated) < DNS_CACHE_TTL {
                return;
            }
        }
    }

    // Slow path: acquire write lock and refresh
    let mut guard = match DNS_CACHE.write() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };

    if let Some(ref cache) = *guard {
        if now.duration_since(cache.last_updated) < DNS_CACHE_TTL {
            return;
        }
    }

    let machines = load_all_machines(machines_dir).unwrap_or_default();
    let mut map = HashMap::new();
    let mut rev = HashMap::new();
    for m in machines {
        if m.state == mach_common::machine::MachineState::Running
            || m.state == mach_common::machine::MachineState::Starting
        {
            if let Some(std::net::IpAddr::V4(v4)) = m.ip_address {
                let name = m.config.name.to_ascii_lowercase();
                map.insert(name.clone(), v4);
                rev.insert(v4, name);
            }
        }
    }

    *guard = Some(DnsCacheEntry {
        last_updated: now,
        machines: map,
        reverse: rev,
    });
}

/// Lookup machine IPv4 by exact machine name.
pub fn lookup_machine_ip(machines_dir: &Path, name: &str) -> Option<Ipv4Addr> {
    refresh_cache_if_needed(machines_dir);
    let guard = match DNS_CACHE.read() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    guard
        .as_ref()
        .and_then(|c| c.machines.get(&name.to_ascii_lowercase()).copied())
}

/// Lookup machine name by IPv4 address.
pub fn lookup_machine_by_ip(machines_dir: &Path, ip: Ipv4Addr) -> Option<String> {
    refresh_cache_if_needed(machines_dir);
    let guard = match DNS_CACHE.read() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    guard.as_ref().and_then(|c| c.reverse.get(&ip).cloned())
}

/// Resolve a machine name to an IP using the cache if valid, or refreshing from disk.
/// Defaults to 127.0.0.1 (reverse proxy) if not matching a running machine.
pub fn resolve_machine_ip(machines_dir: &Path, prefix: &str) -> Ipv4Addr {
    lookup_machine_ip(machines_dir, prefix).unwrap_or(Ipv4Addr::new(127, 0, 0, 1))
}

/// Encode a domain name into DNS wire format (e.g. "gateway.mach.local" -> "\x07gateway\x04mach\x05local\x00").
pub fn encode_dns_name(name: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    for part in name.split('.') {
        if !part.is_empty() {
            bytes.push(part.len() as u8);
            bytes.extend_from_slice(part.as_bytes());
        }
    }
    bytes.push(0x00);
    bytes
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DnsRecord {
    A(Ipv4Addr),
    Ptr(String),
}

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
    if packet.len() < 12 {
        return None;
    }

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

    let is_mach_local = domain.ends_with(".mach.local") || domain == "mach.local";
    let is_norb_local = domain.ends_with(".norb.local") || domain == "norb.local";

    let record: Option<DnsRecord> = if is_mach_local || is_norb_local {
        if domain == "mach.local" || domain == "norb.local" {
            Some(DnsRecord::A(Ipv4Addr::new(127, 0, 0, 1)))
        } else {
            let sub_labels = &labels[..labels.len() - 2];
            if sub_labels.len() == 1 {
                let label = &sub_labels[0];
                if label == "host" || label == "gateway" {
                    Some(DnsRecord::A(Ipv4Addr::new(192, 168, 64, 1)))
                } else if let Some(ip) = lookup_machine_ip(machines_dir, label) {
                    Some(DnsRecord::A(ip))
                } else {
                    // Container or unmatched name -> reverse proxy
                    Some(DnsRecord::A(Ipv4Addr::new(127, 0, 0, 1)))
                }
            } else {
                // Multi-level subdomain (e.g. web.app.mach.local) -> reverse proxy
                Some(DnsRecord::A(Ipv4Addr::new(127, 0, 0, 1)))
            }
        }
    } else if domain == "host.docker.internal"
        || domain == "host.mach.internal"
        || domain == "host.norb.internal"
        || domain == "gateway.docker.internal"
        || domain == "gateway.mach.internal"
        || domain == "gateway.norb.internal"
    {
        Some(DnsRecord::A(Ipv4Addr::new(192, 168, 64, 1)))
    } else if domain.ends_with(".in-addr.arpa") {
        if labels.len() == 6 && labels[4] == "in-addr" && labels[5] == "arpa" {
            if let (Ok(d), Ok(c), Ok(b), Ok(a)) = (
                labels[0].parse::<u8>(),
                labels[1].parse::<u8>(),
                labels[2].parse::<u8>(),
                labels[3].parse::<u8>(),
            ) {
                let ip = Ipv4Addr::new(a, b, c, d);
                if ip == Ipv4Addr::new(192, 168, 64, 1) {
                    Some(DnsRecord::Ptr("gateway.mach.local".to_string()))
                } else if let Some(m_name) = lookup_machine_by_ip(machines_dir, ip) {
                    Some(DnsRecord::Ptr(format!("{}.mach.local", m_name)))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    let mut resp = Vec::with_capacity(512);
    // Header: ID
    resp.extend_from_slice(&id.to_be_bytes());

    match record {
        Some(DnsRecord::A(ip)) => {
            if qtype == 1 || qtype == 255 {
                // Standard query response, No error, Recursion Desired + Available
                resp.extend_from_slice(&[0x81, 0x80]); // Flags
                resp.extend_from_slice(&[0x00, 0x01]); // QDCOUNT: 1
                resp.extend_from_slice(&[0x00, 0x01]); // ANCOUNT: 1
                resp.extend_from_slice(&[0x00, 0x00]); // NSCOUNT: 0
                resp.extend_from_slice(&[0x00, 0x00]); // ARCOUNT: 0

                // Copy Question
                resp.extend_from_slice(&packet[12..qend]);

                // Answer section
                resp.extend_from_slice(&[0xc0, 0x0c]); // Pointer to QNAME at byte 12
                resp.extend_from_slice(&[0x00, 0x01]); // Type A
                resp.extend_from_slice(&[0x00, 0x01]); // Class IN
                resp.extend_from_slice(&[0x00, 0x00, 0x00, 0x0a]); // TTL 10 seconds
                resp.extend_from_slice(&[0x00, 0x04]); // RDLENGTH 4
                resp.extend_from_slice(&ip.octets()); // RDATA
                Some(resp)
            } else if qtype == 28 {
                // AAAA query for valid name -> NODATA response (NOERROR, 0 answers)
                resp.extend_from_slice(&[0x81, 0x80]);
                resp.extend_from_slice(&[0x00, 0x01]); // QDCOUNT: 1
                resp.extend_from_slice(&[0x00, 0x00]); // ANCOUNT: 0
                resp.extend_from_slice(&[0x00, 0x00]); // NSCOUNT: 0
                resp.extend_from_slice(&[0x00, 0x00]); // ARCOUNT: 0
                resp.extend_from_slice(&packet[12..qend]);
                Some(resp)
            } else {
                // Other query types on valid name: NODATA
                resp.extend_from_slice(&[0x81, 0x80]);
                resp.extend_from_slice(&[0x00, 0x01]);
                resp.extend_from_slice(&[0x00, 0x00]);
                resp.extend_from_slice(&[0x00, 0x00]);
                resp.extend_from_slice(&[0x00, 0x00]);
                resp.extend_from_slice(&packet[12..qend]);
                Some(resp)
            }
        }
        Some(DnsRecord::Ptr(target_name)) => {
            if qtype == 12 || qtype == 255 {
                // Type PTR (12) or ANY: return PTR record
                resp.extend_from_slice(&[0x81, 0x80]); // Flags: NOERROR
                resp.extend_from_slice(&[0x00, 0x01]); // QDCOUNT: 1
                resp.extend_from_slice(&[0x00, 0x01]); // ANCOUNT: 1
                resp.extend_from_slice(&[0x00, 0x00]); // NSCOUNT: 0
                resp.extend_from_slice(&[0x00, 0x00]); // ARCOUNT: 0

                // Copy Question
                resp.extend_from_slice(&packet[12..qend]);

                // Answer section
                resp.extend_from_slice(&[0xc0, 0x0c]); // Pointer to QNAME at byte 12
                resp.extend_from_slice(&[0x00, 0x0c]); // Type PTR (12)
                resp.extend_from_slice(&[0x00, 0x01]); // Class IN (1)
                resp.extend_from_slice(&[0x00, 0x00, 0x00, 0x0a]); // TTL 10 seconds
                let rdata = encode_dns_name(&target_name);
                resp.extend_from_slice(&(rdata.len() as u16).to_be_bytes()); // RDLENGTH
                resp.extend_from_slice(&rdata); // RDATA
                Some(resp)
            } else {
                // A or AAAA on PTR domain: NODATA
                resp.extend_from_slice(&[0x81, 0x80]);
                resp.extend_from_slice(&[0x00, 0x01]);
                resp.extend_from_slice(&[0x00, 0x00]);
                resp.extend_from_slice(&[0x00, 0x00]);
                resp.extend_from_slice(&[0x00, 0x00]);
                resp.extend_from_slice(&packet[12..qend]);
                Some(resp)
            }
        }
        None => {
            // Name error (NXDOMAIN: 0x8183)
            resp.extend_from_slice(&[0x81, 0x83]);
            resp.extend_from_slice(&[0x00, 0x01]); // QDCOUNT: 1
            resp.extend_from_slice(&[0x00, 0x00]); // ANCOUNT: 0
            resp.extend_from_slice(&[0x00, 0x00]); // NSCOUNT: 0
            resp.extend_from_slice(&[0x00, 0x00]); // ARCOUNT: 0
            resp.extend_from_slice(&packet[12..qend]);
            Some(resp)
        }
    }
}

/// Helper to verify or configure macOS `/etc/resolver/mach.local`.
pub fn check_or_setup_resolver() -> (bool, Option<String>) {
    let resolver_path = Path::new("/etc/resolver/mach.local");
    if resolver_path.exists() {
        if let Ok(content) = std::fs::read_to_string(resolver_path) {
            if content.contains("5354") && content.contains("127.0.0.1") {
                return (true, None);
            }
        }
    }

    let setup_cmd = "sudo mkdir -p /etc/resolver && sudo tee /etc/resolver/mach.local <<< 'nameserver 127.0.0.1\nport 5354'".to_string();
    (false, Some(setup_cmd))
}

/// Attempt to install `/etc/resolver/mach.local` and `/etc/resolver/norb.local` directly or via sudo.
pub fn install_resolver() -> Result<()> {
    let resolver_dir = Path::new("/etc/resolver");
    let content = "nameserver 127.0.0.1\nport 5354\n";

    // 1. Direct write if directory is writable
    if resolver_dir.exists() || std::fs::create_dir_all(resolver_dir).is_ok() {
        let _ = std::fs::write(resolver_dir.join("mach.local"), content);
        let _ = std::fs::write(resolver_dir.join("norb.local"), content);
        if resolver_dir.join("mach.local").exists() {
            return Ok(());
        }
    }

    // 2. Try sudo
    let status = std::process::Command::new("sudo")
        .args([
            "sh",
            "-c",
            "mkdir -p /etc/resolver && printf 'nameserver 127.0.0.1\\nport 5354\\n' | tee /etc/resolver/mach.local /etc/resolver/norb.local >/dev/null",
        ])
        .status()
        .context("failed to execute sudo")?;

    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("failed to configure /etc/resolver/mach.local with sudo");
    }
}

/// Query the DNS responder at 127.0.0.1:5354 with a short timeout.
pub fn probe_dns_responder(timeout: Duration) -> bool {
    let socket = match std::net::UdpSocket::bind("127.0.0.1:0") {
        Ok(s) => s,
        Err(_) => return false,
    };
    if socket.set_read_timeout(Some(timeout)).is_err() {
        return false;
    }
    if socket.set_write_timeout(Some(timeout)).is_err() {
        return false;
    }

    let id: u16 = 0x5a5a;
    let mut query = Vec::with_capacity(35);
    query.extend_from_slice(&id.to_be_bytes());
    query.extend_from_slice(&[0x01, 0x00]); // Standard query, RD=1
    query.extend_from_slice(&[0x00, 0x01]); // QDCOUNT: 1
    query.extend_from_slice(&[0x00, 0x00]); // ANCOUNT: 0
    query.extend_from_slice(&[0x00, 0x00]); // NSCOUNT: 0
    query.extend_from_slice(&[0x00, 0x00]); // ARCOUNT: 0
    // QNAME: gateway.mach.local
    query.extend_from_slice(b"\x07gateway\x04mach\x05local\x00");
    query.extend_from_slice(&[0x00, 0x01]); // QTYPE: A
    query.extend_from_slice(&[0x00, 0x01]); // QCLASS: IN

    let target = format!("127.0.0.1:{}", DNS_PORT);
    if socket.send_to(&query, &target).is_err() {
        return false;
    }

    let mut buf = [0u8; 512];
    match socket.recv_from(&mut buf) {
        Ok((len, _)) => {
            if len >= 12 {
                let resp_id = u16::from_be_bytes([buf[0], buf[1]]);
                let flags = u16::from_be_bytes([buf[2], buf[3]]);
                let is_response = (flags & 0x8000) != 0;
                let rcode = flags & 0x000F;
                resp_id == id && is_response && rcode == 0
            } else {
                false
            }
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_query(id: u16, name: &str, qtype: u16) -> Vec<u8> {
        let mut pkt = Vec::new();
        pkt.extend_from_slice(&id.to_be_bytes());
        pkt.extend_from_slice(&[0x01, 0x00]); // Standard query
        pkt.extend_from_slice(&[0x00, 0x01]); // QDCOUNT: 1
        pkt.extend_from_slice(&[0x00, 0x00]); // ANCOUNT: 0
        pkt.extend_from_slice(&[0x00, 0x00]); // NSCOUNT: 0
        pkt.extend_from_slice(&[0x00, 0x00]); // ARCOUNT: 0
        pkt.extend_from_slice(&encode_dns_name(name));
        pkt.extend_from_slice(&qtype.to_be_bytes());
        pkt.extend_from_slice(&[0x00, 0x01]); // QCLASS: IN
        pkt
    }

    fn parse_a_response(resp: &[u8]) -> Option<(u16, Ipv4Addr)> {
        if resp.len() < 12 {
            return None;
        }
        let flags = u16::from_be_bytes([resp[2], resp[3]]);
        let ancount = u16::from_be_bytes([resp[6], resp[7]]);
        if ancount == 0 {
            return Some((flags, Ipv4Addr::UNSPECIFIED));
        }
        let len = resp.len();
        if len >= 16 {
            let ip = Ipv4Addr::new(resp[len - 4], resp[len - 3], resp[len - 2], resp[len - 1]);
            Some((flags, ip))
        } else {
            None
        }
    }

    fn parse_ptr_response(resp: &[u8]) -> Option<(u16, String)> {
        if resp.len() < 12 {
            return None;
        }
        let flags = u16::from_be_bytes([resp[2], resp[3]]);
        let ancount = u16::from_be_bytes([resp[6], resp[7]]);
        if ancount == 0 {
            return Some((flags, String::new()));
        }

        // Skip Question
        let mut pos = 12;
        while pos < resp.len() {
            let l = resp[pos] as usize;
            pos += 1;
            if l == 0 {
                break;
            }
            pos += l;
        }
        pos += 4; // QTYPE (2) + QCLASS (2)

        if pos + 12 > resp.len() {
            return None;
        }
        // Answer: NAME(2) + TYPE(2) + CLASS(2) + TTL(4) = 10 bytes
        pos += 10;
        let rdlength = u16::from_be_bytes([resp[pos], resp[pos + 1]]) as usize;
        pos += 2;
        if pos + rdlength > resp.len() {
            return None;
        }

        let mut labels = Vec::new();
        let end = pos + rdlength;
        while pos < end {
            let l = resp[pos] as usize;
            pos += 1;
            if l == 0 {
                break;
            }
            if pos + l > end {
                return None;
            }
            labels.push(std::str::from_utf8(&resp[pos..pos + l]).ok()?);
            pos += l;
        }
        Some((flags, labels.join(".")))
    }

    static TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn test_dns_cache_resolution() {
        let _lock = TEST_MUTEX.lock().unwrap();
        let temp_dir = std::env::temp_dir().join("mach_dns_test_cache");
        let _ = std::fs::create_dir_all(&temp_dir);

        {
            let mut guard = DNS_CACHE.write().unwrap();
            let mut map = HashMap::new();
            map.insert("testvm".to_string(), Ipv4Addr::new(192, 168, 64, 42));
            let mut rev = HashMap::new();
            rev.insert(Ipv4Addr::new(192, 168, 64, 42), "testvm".to_string());
            *guard = Some(DnsCacheEntry {
                last_updated: Instant::now(),
                machines: map,
                reverse: rev,
            });
        }

        let ip = resolve_machine_ip(&temp_dir, "testvm");
        assert_eq!(ip, Ipv4Addr::new(192, 168, 64, 42));

        let ip2 = resolve_machine_ip(&temp_dir, "unknown-container");
        assert_eq!(ip2, Ipv4Addr::new(127, 0, 0, 1));
    }

    #[test]
    fn test_handle_host_and_gateway() {
        let _lock = TEST_MUTEX.lock().unwrap();
        let temp_dir = std::env::temp_dir().join("mach_dns_test_host");
        let _ = std::fs::create_dir_all(&temp_dir);

        // host.mach.local -> 192.168.64.1
        let pkt = make_test_query(1, "host.mach.local", 1);
        let resp = handle_dns_query(&pkt, &temp_dir).expect("should answer");
        let (flags, ip) = parse_a_response(&resp).unwrap();
        assert_eq!(flags, 0x8180);
        assert_eq!(ip, Ipv4Addr::new(192, 168, 64, 1));

        // host.norb.local -> 192.168.64.1
        let pkt_legacy = make_test_query(11, "host.norb.local", 1);
        let resp_legacy = handle_dns_query(&pkt_legacy, &temp_dir).expect("should answer");
        let (_, ip_legacy) = parse_a_response(&resp_legacy).unwrap();
        assert_eq!(ip_legacy, Ipv4Addr::new(192, 168, 64, 1));

        // gateway.mach.local -> 192.168.64.1
        let pkt = make_test_query(2, "gateway.mach.local", 1);
        let resp = handle_dns_query(&pkt, &temp_dir).expect("should answer");
        let (flags, ip) = parse_a_response(&resp).unwrap();
        assert_eq!(flags, 0x8180);
        assert_eq!(ip, Ipv4Addr::new(192, 168, 64, 1));
    }

    #[test]
    fn test_handle_running_machine_and_containers() {
        let _lock = TEST_MUTEX.lock().unwrap();
        let temp_dir = std::env::temp_dir().join("mach_dns_test_machines");
        let _ = std::fs::create_dir_all(&temp_dir);

        {
            let mut guard = DNS_CACHE.write().unwrap();
            let mut map = HashMap::new();
            map.insert("ubuntu".to_string(), Ipv4Addr::new(192, 168, 64, 10));
            let mut rev = HashMap::new();
            rev.insert(Ipv4Addr::new(192, 168, 64, 10), "ubuntu".to_string());
            *guard = Some(DnsCacheEntry {
                last_updated: Instant::now(),
                machines: map,
                reverse: rev,
            });
        }

        // Exact single label matches running machine -> machine IP
        let pkt = make_test_query(10, "ubuntu.mach.local", 1);
        let resp = handle_dns_query(&pkt, &temp_dir).expect("should answer");
        let (_, ip) = parse_a_response(&resp).unwrap();
        assert_eq!(ip, Ipv4Addr::new(192, 168, 64, 10));

        let pkt_legacy = make_test_query(101, "ubuntu.norb.local", 1);
        let resp_legacy = handle_dns_query(&pkt_legacy, &temp_dir).expect("should answer");
        let (_, ip_legacy) = parse_a_response(&resp_legacy).unwrap();
        assert_eq!(ip_legacy, Ipv4Addr::new(192, 168, 64, 10));

        // Single label container (not a machine) -> 127.0.0.1
        let pkt = make_test_query(11, "redis.mach.local", 1);
        let resp = handle_dns_query(&pkt, &temp_dir).expect("should answer");
        let (_, ip) = parse_a_response(&resp).unwrap();
        assert_eq!(ip, Ipv4Addr::new(127, 0, 0, 1));

        // Multi-level subdomain -> 127.0.0.1
        let pkt = make_test_query(12, "web.app.mach.local", 1);
        let resp = handle_dns_query(&pkt, &temp_dir).expect("should answer");
        let (_, ip) = parse_a_response(&resp).unwrap();
        assert_eq!(ip, Ipv4Addr::new(127, 0, 0, 1));
    }

    #[test]
    fn test_handle_internal_host_names() {
        let _lock = TEST_MUTEX.lock().unwrap();
        let temp_dir = std::env::temp_dir().join("mach_dns_test_internal");
        let _ = std::fs::create_dir_all(&temp_dir);

        let pkt1 = make_test_query(20, "host.docker.internal", 1);
        let resp1 = handle_dns_query(&pkt1, &temp_dir).unwrap();
        let (_, ip1) = parse_a_response(&resp1).unwrap();
        assert_eq!(ip1, Ipv4Addr::new(192, 168, 64, 1));

        let pkt2 = make_test_query(21, "host.mach.internal", 1);
        let resp2 = handle_dns_query(&pkt2, &temp_dir).unwrap();
        let (_, ip2) = parse_a_response(&resp2).unwrap();
        assert_eq!(ip2, Ipv4Addr::new(192, 168, 64, 1));
    }

    #[test]
    fn test_handle_reverse_ptr() {
        let _lock = TEST_MUTEX.lock().unwrap();
        let temp_dir = std::env::temp_dir().join("mach_dns_test_ptr");
        let _ = std::fs::create_dir_all(&temp_dir);

        {
            let mut guard = DNS_CACHE.write().unwrap();
            let mut map = HashMap::new();
            map.insert("debian".to_string(), Ipv4Addr::new(192, 168, 64, 15));
            let mut rev = HashMap::new();
            rev.insert(Ipv4Addr::new(192, 168, 64, 15), "debian".to_string());
            *guard = Some(DnsCacheEntry {
                last_updated: Instant::now(),
                machines: map,
                reverse: rev,
            });
        }

        // 1.64.168.192.in-addr.arpa -> gateway.mach.local
        let pkt = make_test_query(30, "1.64.168.192.in-addr.arpa", 12);
        let resp = handle_dns_query(&pkt, &temp_dir).expect("should answer");
        let (flags, name) = parse_ptr_response(&resp).unwrap();
        assert_eq!(flags, 0x8180);
        assert_eq!(name, "gateway.mach.local");

        // 15.64.168.192.in-addr.arpa -> debian.mach.local
        let pkt = make_test_query(31, "15.64.168.192.in-addr.arpa", 12);
        let resp = handle_dns_query(&pkt, &temp_dir).expect("should answer");
        let (flags, name) = parse_ptr_response(&resp).unwrap();
        assert_eq!(flags, 0x8180);
        assert_eq!(name, "debian.mach.local");

        // Unknown IP -> NXDOMAIN (0x8183)
        let pkt = make_test_query(32, "99.64.168.192.in-addr.arpa", 12);
        let resp = handle_dns_query(&pkt, &temp_dir).expect("should answer");
        let flags = u16::from_be_bytes([resp[2], resp[3]]);
        assert_eq!(flags, 0x8183);
    }

    #[test]
    fn test_handle_aaaa_and_unknown() {
        let _lock = TEST_MUTEX.lock().unwrap();
        let temp_dir = std::env::temp_dir().join("mach_dns_test_aaaa");
        let _ = std::fs::create_dir_all(&temp_dir);

        // AAAA for host.mach.local -> NOERROR (0x8180), ANCOUNT = 0
        let pkt = make_test_query(40, "host.mach.local", 28);
        let resp = handle_dns_query(&pkt, &temp_dir).expect("should answer");
        let flags = u16::from_be_bytes([resp[2], resp[3]]);
        let ancount = u16::from_be_bytes([resp[6], resp[7]]);
        assert_eq!(flags, 0x8180);
        assert_eq!(ancount, 0);

        // Unknown external domain -> NXDOMAIN (0x8183)
        let pkt = make_test_query(41, "google.com", 1);
        let resp = handle_dns_query(&pkt, &temp_dir).expect("should answer");
        let flags = u16::from_be_bytes([resp[2], resp[3]]);
        assert_eq!(flags, 0x8183);
    }
}
