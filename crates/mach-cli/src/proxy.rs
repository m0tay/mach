//! Local HTTP reverse proxy for `*.mach.local` container domains.
//!
//! Listens on `127.0.0.1:80` (with automatic fallback to `127.0.0.1:8080`) and routes
//! incoming HTTP requests to the appropriate Docker container in the guest VM.

use anyhow::Result;
use serde_json::Value;
use std::net::SocketAddr;
use std::path::Path;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UnixStream};

use crate::commands::load_all_machines;

/// Stream wrapper that yields bytes from an in-memory prefix buffer first,
/// then transparently delegates all further reading and writing to an underlying `TcpStream`.
struct PrefixedStream<'a> {
    prefix: std::io::Cursor<Vec<u8>>,
    stream: &'a mut TcpStream,
}

impl<'a> tokio::io::AsyncRead for PrefixedStream<'a> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let pos = self.prefix.position() as usize;
        let prefix_bytes = self.prefix.get_ref();
        if pos < prefix_bytes.len() {
            let slice = &prefix_bytes[pos..];
            let to_write = std::cmp::min(slice.len(), buf.remaining());
            buf.put_slice(&slice[..to_write]);
            self.prefix.set_position((pos + to_write) as u64);
            Poll::Ready(Ok(()))
        } else {
            Pin::new(&mut *self.stream).poll_read(cx, buf)
        }
    }
}

impl<'a> tokio::io::AsyncWrite for PrefixedStream<'a> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut *self.stream).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut *self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut *self.stream).poll_shutdown(cx)
    }
}

/// Decode HTTP/1.1 Transfer-Encoding: chunked body.
/// Returns decoded bytes, or falls back to raw slice if not valid chunked encoding.
pub fn decode_chunked_body(raw: &[u8]) -> Vec<u8> {
    let mut cursor = 0;
    let mut decoded = Vec::new();

    while cursor < raw.len() {
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

        cursor = line_end;
        if cursor + 2 <= raw.len() && &raw[cursor..cursor + 2] == b"\r\n" {
            cursor += 2;
        } else if cursor < raw.len() && raw[cursor] == b'\n' {
            cursor += 1;
        }

        if chunk_size == 0 {
            break;
        }

        if cursor + chunk_size > raw.len() {
            decoded.extend_from_slice(&raw[cursor..]);
            break;
        }

        decoded.extend_from_slice(&raw[cursor..cursor + chunk_size]);
        cursor += chunk_size;

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

/// Container routing target
#[derive(Debug, Clone)]
pub struct RouteTarget {
    pub name: String,
    pub image_repo: Option<String>,
    pub compose_service: Option<String>,
    pub compose_project: Option<String>,
    pub port: u16,
    pub ports: Vec<u16>,
}

impl RouteTarget {
    /// Return candidate ports for HTTP probing or routing.
    pub fn candidate_ports(&self) -> Vec<u16> {
        if self.ports.is_empty() {
            vec![self.port]
        } else {
            self.ports.clone()
        }
    }
}

pub async fn run_reverse_proxy(docker_sock: &Path, machines_dir: &Path) -> Result<()> {
    // Primary listener: bind 0.0.0.0:28080 for incoming forwarded traffic from VM and host
    let listener_28080 = TcpListener::bind("0.0.0.0:28080").await?;
    println!("[proxy] HTTP reverse proxy listening on \x1b[1;32m0.0.0.0:28080\x1b[0m");

    // Optional: attempt binding 127.0.0.1:80 directly if running with elevated privileges
    let listener_80 = TcpListener::bind("127.0.0.1:80").await.ok();
    if listener_80.is_some() {
        println!("[proxy] HTTP reverse proxy also listening on \x1b[1;32m127.0.0.1:80\x1b[0m");
    }

    let sock_path = docker_sock.to_path_buf();
    let m_dir = machines_dir.to_path_buf();

    if let Some(l80) = listener_80 {
        let sock_clone = sock_path.clone();
        let m_clone = m_dir.clone();
        tokio::spawn(async move {
            loop {
                if let Ok((client_stream, _peer)) = l80.accept().await {
                    let s_c = sock_clone.clone();
                    let m_c = m_clone.clone();
                    tokio::spawn(async move {
                        let _ = handle_proxy_conn(client_stream, &s_c, &m_c).await;
                    });
                } else {
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                }
            }
        });
    }

    loop {
        let (client_stream, _peer) = match listener_28080.accept().await {
            Ok(conn) => conn,
            Err(_) => {
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                continue;
            }
        };

        let sock_clone = sock_path.clone();
        let m_clone = m_dir.clone();

        tokio::spawn(async move {
            let _ = handle_proxy_conn(client_stream, &sock_clone, &m_clone).await;
        });
    }
}

async fn handle_proxy_conn(mut client: TcpStream, docker_sock: &Path, machines_dir: &Path) -> Result<()> {
    const MAX_HEADER_SIZE: usize = 8192; // 8KB
    let mut accumulated = Vec::with_capacity(1024);
    let mut temp_buf = [0u8; 1024];

    loop {
        let n = client.read(&mut temp_buf).await?;
        if n == 0 {
            return Ok(());
        }
        accumulated.extend_from_slice(&temp_buf[..n]);

        if accumulated.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }

        if accumulated.len() >= MAX_HEADER_SIZE {
            let resp = "HTTP/1.1 431 Request Header Fields Too Large\r\nContent-Length: 32\r\nConnection: close\r\n\r\nRequest Header Fields Too Large\n";
            let _ = client.write_all(resp.as_bytes()).await;
            return Ok(());
        }
    }

    let header_str = String::from_utf8_lossy(&accumulated);
    let host = parse_host_header(&header_str);

    let host = match host {
        Some(h) => h,
        None => {
            let resp = "HTTP/1.1 400 Bad Request\r\nContent-Length: 16\r\nConnection: close\r\n\r\nMissing Host header";
            let _ = client.write_all(resp.as_bytes()).await;
            return Ok(());
        }
    };

    // Find the VM IP
    let machines = load_all_machines(machines_dir).unwrap_or_default();
    let guest_ip = machines
        .iter()
        .find(|m| (m.config.name == "mach-system" || m.config.name == "mach-docker") && m.state == mach_common::machine::MachineState::Running && m.ip_address.is_some())
        .or_else(|| machines.iter().find(|m| m.ip_address.is_some() && m.state == mach_common::machine::MachineState::Running))
        .and_then(|m| m.ip_address);

    let guest_ip = match guest_ip {
        Some(ip) => ip,
        None => {
            send_html_error(&mut client, 503, "No active mach virtual machine found.").await?;
            return Ok(());
        }
    };

    // Find matching container route with auto-detection probe or override
    let routes = fetch_container_routes(docker_sock).await.unwrap_or_default();
    let matched_port = resolve_target_port(&host, &routes, guest_ip).await;

    let target_port = match matched_port {
        Some(p) => p,
        None => {
            let msg = format!("No running container found for host: <b>{}</b>", html_escape(&host));
            send_html_error(&mut client, 502, &msg).await?;
            return Ok(());
        }
    };

    let target_addr = SocketAddr::new(guest_ip, target_port);
    let mut target = match TcpStream::connect(target_addr).await {
        Ok(t) => t,
        Err(e) => {
            let msg = format!("Failed to connect to container port {target_port}: {e}");
            send_html_error(&mut client, 504, &msg).await?;
            return Ok(());
        }
    };

    let mut client_forwarder = PrefixedStream {
        prefix: std::io::Cursor::new(accumulated),
        stream: &mut client,
    };

    let _ = tokio::io::copy_bidirectional(&mut client_forwarder, &mut target).await;
    Ok(())
}

/// Parse Host header from incoming HTTP request.
/// Preserves '@<port>' override if present while stripping proxy port (e.g. ':8080').
pub fn parse_host_header(headers: &str) -> Option<String> {
    for line in headers.lines() {
        if line.to_ascii_lowercase().starts_with("host:") {
            let mut val = line[5..].trim();
            if let Some(stripped) = val.strip_prefix("http://").or_else(|| val.strip_prefix("https://")) {
                val = stripped;
            }
            if let Some(pos) = val.find('/') {
                val = &val[..pos];
            }

            let host_only = if let Some(at_idx) = val.find('@') {
                let domain_part = &val[..at_idx];
                let after_at = &val[at_idx + 1..];
                let digits: String = after_at.chars().take_while(|c| c.is_ascii_digit()).collect();
                let rest = &after_at[digits.len()..];
                let rest_clean = rest.split(':').next().unwrap_or(rest);
                format!("{domain_part}@{digits}{rest_clean}")
            } else {
                val.split(':').next().unwrap_or(val).to_string()
            };
            return Some(host_only.trim().to_ascii_lowercase());
        }
    }
    None
}

/// Extract clean domain and optional '@<port>' override from host string.
/// Handles forms like 'http://app.mach.local@8080', 'app.mach.local@8080', and 'app.mach.local:8080'.
pub fn parse_host_and_override_port(host: &str) -> (String, Option<u16>) {
    let mut h = host.trim();
    if let Some(stripped) = h.strip_prefix("http://").or_else(|| h.strip_prefix("https://")) {
        h = stripped;
    }
    if let Some(pos) = h.find('/') {
        h = &h[..pos];
    }
    if let Some(at_idx) = h.find('@') {
        let before = &h[..at_idx];
        let after = &h[at_idx + 1..];
        let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        let rest = &after[digits.len()..];
        if let Ok(port) = digits.parse::<u16>() {
            let rest_domain = rest.trim_start_matches(|c: char| c.is_ascii_digit() || c == ':');
            let domain = format!("{}{}", before, rest_domain);
            return (domain, Some(port));
        }
    }

    let domain = h.split(':').next().unwrap_or(h).to_string();
    (domain, None)
}

/// Strip `.mach.local` or `.local` domain suffix case-insensitively.
pub fn strip_domain_suffix(domain: &str) -> &str {
    let lower = domain.to_ascii_lowercase();
    if lower.ends_with(".mach.local") {
        &domain[..domain.len() - 11]
    } else if lower.ends_with(".local") {
        &domain[..domain.len() - 6]
    } else {
        domain
    }
}

/// Find matching container route target by domain name or Docker Compose service/project.
pub fn find_target_route<'a>(domain: &str, routes: &'a [RouteTarget]) -> Option<&'a RouteTarget> {
    let clean_host = strip_domain_suffix(domain);

    for r in routes {
        // Direct container name match (handling optional leading '/')
        let raw_name = r.name.trim_start_matches('/');
        if raw_name.eq_ignore_ascii_case(clean_host) {
            return Some(r);
        }

        // Docker Compose multi-level domain routing:
        // 1. <service>.<project>.mach.local (clean_host == <service>.<project>)
        // 2. <project>-<service>.mach.local (clean_host == <project>-<service>)
        if let (Some(svc), Some(proj)) = (&r.compose_service, &r.compose_project) {
            let pattern1 = format!("{}.{}", svc, proj);
            let pattern2 = format!("{}-{}", proj, svc);
            if clean_host.eq_ignore_ascii_case(&pattern1) || clean_host.eq_ignore_ascii_case(&pattern2) {
                return Some(r);
            }
        }

        // Image repository match (e.g. "nginx" for "nginx:alpine")
        if let Some(ref img_repo) = r.image_repo {
            if img_repo.eq_ignore_ascii_case(clean_host) {
                return Some(r);
            }
        }
    }

    // Default to the first route if only one container is running
    if routes.len() == 1 {
        return Some(&routes[0]);
    }

    None
}

/// Synchronously look up target container port from Host header.
/// Supports '@<port>' override syntax (e.g. `http://app.mach.local@8080`), routing directly to it.
pub fn find_target_port(host: &str, routes: &[RouteTarget]) -> Option<u16> {
    let (domain, override_port) = parse_host_and_override_port(host);

    if let Some(port) = override_port {
        return Some(port);
    }

    let target = find_target_route(&domain, routes)?;
    Some(target.port)
}

/// Resolve target container port with asynchronous HTTP candidate port probing.
/// - If '@<port>' is specified in host, routes directly to it without probing.
/// - When a container publishes multiple ports and no override is provided,
///   probes candidate ports asynchronously (HTTP HEAD / with 50ms timeout)
///   and routes traffic to the first responsive port.
pub async fn resolve_target_port(
    host: &str,
    routes: &[RouteTarget],
    guest_ip: std::net::IpAddr,
) -> Option<u16> {
    let (domain, override_port) = parse_host_and_override_port(host);

    if let Some(port) = override_port {
        return Some(port);
    }

    let target = find_target_route(&domain, routes)?;
    let candidates = target.candidate_ports();
    if candidates.len() > 1 {
        if let Some(probe_port) = probe_candidate_ports(guest_ip, &candidates).await {
            return Some(probe_port);
        }
    }

    Some(target.port)
}

/// Probe candidate TCP ports asynchronously using HTTP HEAD / with 50ms timeout.
/// Returns the first responsive port, or None if none responded.
pub async fn probe_candidate_ports(guest_ip: std::net::IpAddr, ports: &[u16]) -> Option<u16> {
    let mut unique_ports = Vec::new();
    for &p in ports {
        if p > 0 && !unique_ports.contains(&p) {
            unique_ports.push(p);
        }
    }

    if unique_ports.is_empty() {
        return None;
    }
    if unique_ports.len() == 1 {
        return Some(unique_ports[0]);
    }

    let mut set = tokio::task::JoinSet::new();
    for &port in &unique_ports {
        let addr = SocketAddr::new(guest_ip, port);
        set.spawn(async move {
            if probe_http_port(addr).await {
                Some(port)
            } else {
                None
            }
        });
    }

    while let Some(res) = set.join_next().await {
        if let Ok(Some(port)) = res {
            set.abort_all();
            return Some(port);
        }
    }

    None
}

/// Probe a single address with HTTP HEAD / within 50ms.
async fn probe_http_port(addr: SocketAddr) -> bool {
    let probe = async {
        let mut stream = TcpStream::connect(addr).await.ok()?;
        stream
            .write_all(b"HEAD / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .ok()?;
        let mut buf = [0u8; 64];
        let mut total_read = 0;
        while total_read < 5 {
            let n = stream.read(&mut buf[total_read..]).await.ok()?;
            if n == 0 {
                break;
            }
            total_read += n;
        }
        if total_read >= 5 && buf[..total_read].starts_with(b"HTTP/") {
            Some(())
        } else {
            None
        }
    };

    tokio::time::timeout(tokio::time::Duration::from_millis(50), probe)
        .await
        .ok()
        .flatten()
        .is_some()
}

async fn fetch_container_routes(docker_sock: &Path) -> Result<Vec<RouteTarget>> {
    if !docker_sock.exists() {
        return Ok(Vec::new());
    }

    let mut stream = UnixStream::connect(docker_sock).await?;
    stream
        .write_all(b"GET /containers/json HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await?;

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await?;

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

    let containers: Value = match serde_json::from_slice(&body_bytes) {
        Ok(v) => v,
        Err(_) => {
            let fallback = decode_chunked_body(body);
            match serde_json::from_slice(&fallback) {
                Ok(v) => v,
                Err(_) => return Ok(Vec::new()),
            }
        }
    };

    let mut routes = Vec::new();
    if let Value::Array(list) = containers {
        for c in list {
            let names = c.get("Names").and_then(|v| v.as_array());
            let raw_name = names
                .and_then(|arr| arr.first())
                .and_then(|v| v.as_str())
                .map(|s| s.trim_start_matches('/'))
                .unwrap_or("")
                .to_string();

            let image = c.get("Image").and_then(|v| v.as_str());
            let image_repo = image.map(|img| {
                let repo = img.split(':').next().unwrap_or(img);
                repo.rsplit('/').next().unwrap_or(repo).to_ascii_lowercase()
            });

            let labels = c.get("Labels");
            let compose_service = labels
                .and_then(|l| l.get("com.docker.compose.service"))
                .and_then(|s| s.as_str())
                .map(String::from);
            let compose_project = labels
                .and_then(|l| l.get("com.docker.compose.project"))
                .and_then(|s| s.as_str())
                .map(String::from);

            // Collect all published or private TCP ports
            let mut candidate_ports = Vec::new();
            if let Some(ports) = c.get("Ports").and_then(|p| p.as_array()) {
                for p in ports {
                    let p_type = p.get("Type").and_then(|t| t.as_str()).unwrap_or("tcp");
                    if p_type.eq_ignore_ascii_case("tcp") {
                        let pub_port = p
                            .get("PublicPort")
                            .and_then(|v| v.as_u64())
                            .filter(|&v| v > 0 && v <= 65535);
                        let priv_port = p
                            .get("PrivatePort")
                            .and_then(|v| v.as_u64())
                            .filter(|&v| v > 0 && v <= 65535);

                        // If published to host, prefer the PublicPort
                        if let Some(port) = pub_port {
                            let port_u16 = port as u16;
                            if !candidate_ports.contains(&port_u16) {
                                candidate_ports.push(port_u16);
                            }
                        } else if let Some(port) = priv_port {
                            let port_u16 = port as u16;
                            if !candidate_ports.contains(&port_u16) {
                                candidate_ports.push(port_u16);
                            }
                        }
                    }
                }
            }

            let primary_port = candidate_ports.first().copied().unwrap_or(80);

            routes.push(RouteTarget {
                name: raw_name,
                image_repo,
                compose_service,
                compose_project,
                port: primary_port,
                ports: candidate_ports,
            });
        }
    }

    Ok(routes)
}

async fn send_html_error(stream: &mut TcpStream, status: u16, msg: &str) -> Result<()> {
    let body = format!(
        r#"<!DOCTYPE html>
<html>
<head><title>mach Proxy Error</title>
<style>
body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; background: #0f172a; color: #f8fafc; display: flex; align-items: center; justify-content: center; height: 100vh; margin: 0; }}
.card {{ background: #1e293b; padding: 2rem; border-radius: 12px; border: 1px solid #334155; max-width: 500px; text-align: center; box-shadow: 0 4px 20px rgba(0,0,0,0.5); }}
h1 {{ color: #38bdf8; font-size: 1.5rem; margin-bottom: 1rem; }}
p {{ color: #94a3b8; line-height: 1.5; }}
</style>
</head>
<body>
<div class="card">
  <h1>mach Gateway: {}</h1>
  <p>{}</p>
</div>
</body>
</html>"#,
        status, msg
    );

    let resp = format!(
        "HTTP/1.1 {} Error\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status,
        body.len(),
        body
    );

    let _ = stream.write_all(resp.as_bytes()).await;
    Ok(())
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_host_header() {
        let headers = "GET / HTTP/1.1\r\nHost: myapp.mach.local:8080\r\nUser-Agent: curl\r\n\r\n";
        assert_eq!(parse_host_header(headers), Some("myapp.mach.local".to_string()));

        let headers_lowercase = "get / http/1.1\r\nhost: backend.local\r\n\r\n";
        assert_eq!(parse_host_header(headers_lowercase), Some("backend.local".to_string()));

        let headers_override = "GET / HTTP/1.1\r\nHost: app.mach.local@8080\r\n\r\n";
        assert_eq!(parse_host_header(headers_override), Some("app.mach.local@8080".to_string()));

        let headers_url = "GET / HTTP/1.1\r\nHost: http://app.mach.local@8080\r\n\r\n";
        assert_eq!(parse_host_header(headers_url), Some("app.mach.local@8080".to_string()));

        let no_host = "GET / HTTP/1.1\r\nUser-Agent: curl\r\n\r\n";
        assert_eq!(parse_host_header(no_host), None);
    }

    #[test]
    fn test_proxy_decode_chunked_body() {
        let raw = b"7\r\nMozilla\r\n9\r\nDeveloper\r\n7\r\nNetwork\r\n0\r\n\r\n";
        let decoded = decode_chunked_body(raw);
        assert_eq!(decoded, b"MozillaDeveloperNetwork");
    }

    #[test]
    fn test_find_target_port() {
        let routes = vec![
            RouteTarget {
                name: "web-app".to_string(),
                image_repo: Some("webapp".to_string()),
                compose_service: Some("web".to_string()),
                compose_project: Some("proj".to_string()),
                port: 3000,
                ports: vec![3000],
            },
            RouteTarget {
                name: "db".to_string(),
                image_repo: Some("postgres".to_string()),
                compose_service: None,
                compose_project: None,
                port: 5432,
                ports: vec![5432],
            },
        ];

        // Direct container name
        assert_eq!(find_target_port("web-app.mach.local", &routes), Some(3000));
        assert_eq!(find_target_port("db.mach.local", &routes), Some(5432));
        assert_eq!(find_target_port("unknown.mach.local", &routes), None);

        // Docker Compose: <service>.<project>.mach.local
        assert_eq!(find_target_port("web.proj.mach.local", &routes), Some(3000));
        assert_eq!(find_target_port("WEB.PROJ.mach.local", &routes), Some(3000));

        // Docker Compose: <project>-<service>.mach.local
        assert_eq!(find_target_port("proj-web.mach.local", &routes), Some(3000));
        assert_eq!(find_target_port("PROJ-WEB.mach.local", &routes), Some(3000));

        // Port override syntax: @<port>
        assert_eq!(find_target_port("http://app.mach.local@8080", &routes), Some(8080));
        assert_eq!(find_target_port("app.mach.local@8080", &routes), Some(8080));
        assert_eq!(find_target_port("web-app.mach.local@9000", &routes), Some(9000));
        assert_eq!(find_target_port("web.proj.mach.local@9000", &routes), Some(9000));
        assert_eq!(find_target_port("proj-web.mach.local@9000", &routes), Some(9000));
    }

    #[tokio::test]
    async fn test_probe_candidate_ports() {
        // Start a mock HTTP server on an ephemeral port
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let open_port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 128];
                    let _ = socket.read(&mut buf).await;
                    let resp = "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                    let _ = socket.write_all(resp.as_bytes()).await;
                });
            }
        });

        let closed_port = 59998;
        let ip = std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);

        // Candidate ports with closed port first, then open HTTP port
        let ports = vec![closed_port, open_port];
        let responsive = probe_candidate_ports(ip, &ports).await;
        assert_eq!(responsive, Some(open_port));

        // When all candidate ports fail to respond
        let dead_ports = vec![59997, 59998];
        let dead_res = probe_candidate_ports(ip, &dead_ports).await;
        assert_eq!(dead_res, None);
    }

    #[tokio::test]
    async fn test_resolve_target_port_probe_and_override() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let open_port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 128];
                    let _ = socket.read(&mut buf).await;
                    let resp = "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                    let _ = socket.write_all(resp.as_bytes()).await;
                });
            }
        });

        let closed_port = 59996;
        let ip = std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);

        let routes = vec![RouteTarget {
            name: "multi-svc".to_string(),
            image_repo: None,
            compose_service: Some("web".to_string()),
            compose_project: Some("myapp".to_string()),
            port: closed_port,
            ports: vec![closed_port, open_port],
        }];

        // Auto-detection probe chooses open_port
        let resolved = resolve_target_port("web.myapp.mach.local", &routes, ip).await;
        assert_eq!(resolved, Some(open_port));

        let resolved_kebab = resolve_target_port("myapp-web.mach.local", &routes, ip).await;
        assert_eq!(resolved_kebab, Some(open_port));

        // Port override skips probe and routes directly
        let overridden = resolve_target_port("web.myapp.mach.local@8081", &routes, ip).await;
        assert_eq!(overridden, Some(8081));
    }
}
