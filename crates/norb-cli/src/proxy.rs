//! Local HTTP reverse proxy for `*.norb.local` container domains.
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
struct RouteTarget {
    name: String,
    compose_service: Option<String>,
    compose_project: Option<String>,
    port: u16,
}

pub async fn run_reverse_proxy(docker_sock: &Path, machines_dir: &Path) -> Result<()> {
    // Try binding 80 first, fallback to 8080 if non-root
    let listener = match TcpListener::bind("127.0.0.1:80").await {
        Ok(l) => {
            println!("[proxy] HTTP reverse proxy listening on \x1b[1;32m127.0.0.1:80\x1b[0m");
            l
        }
        Err(_) => {
            let l = TcpListener::bind("127.0.0.1:8080").await?;
            println!("[proxy] HTTP reverse proxy listening on \x1b[1;33m127.0.0.1:8080\x1b[0m (port 80 requires root)");
            l
        }
    };

    let sock_path = docker_sock.to_path_buf();
    let m_dir = machines_dir.to_path_buf();

    loop {
        let (client_stream, _peer) = match listener.accept().await {
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
        .find(|m| m.ip_address.is_some() && m.state == norb_common::machine::MachineState::Running)
        .and_then(|m| m.ip_address);

    let guest_ip = match guest_ip {
        Some(ip) => ip,
        None => {
            send_html_error(&mut client, 503, "No active Norb virtual machine found.").await?;
            return Ok(());
        }
    };

    // Find matching container route
    let routes = fetch_container_routes(docker_sock).await.unwrap_or_default();
    let matched_port = find_target_port(&host, &routes);

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

fn parse_host_header(headers: &str) -> Option<String> {
    for line in headers.lines() {
        if line.to_ascii_lowercase().starts_with("host:") {
            let val = line[5..].trim();
            // Strip port if present (e.g. host:8080)
            let host_only = val.split(':').next().unwrap_or(val).trim();
            return Some(host_only.to_ascii_lowercase());
        }
    }
    None
}

fn find_target_port(host: &str, routes: &[RouteTarget]) -> Option<u16> {
    let clean_host = host.trim_end_matches(".norb.local").trim_end_matches(".local");

    for r in routes {
        // Direct container name match
        if r.name.eq_ignore_ascii_case(clean_host) {
            return Some(r.port);
        }

        // Docker Compose service.project match (e.g. web.myproject)
        if let (Some(svc), Some(proj)) = (&r.compose_service, &r.compose_project) {
            let compose_pattern = format!("{}.{}", svc.to_ascii_lowercase(), proj.to_ascii_lowercase());
            if compose_pattern == clean_host {
                return Some(r.port);
            }
        }
    }

    // Default to the first route if only one container is running
    if routes.len() == 1 {
        return Some(routes[0].port);
    }

    None
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

            let labels = c.get("Labels");
            let compose_service = labels
                .and_then(|l| l.get("com.docker.compose.service"))
                .and_then(|s| s.as_str())
                .map(String::from);
            let compose_project = labels
                .and_then(|l| l.get("com.docker.compose.project"))
                .and_then(|s| s.as_str())
                .map(String::from);

            // Find exposed or published port
            let mut port = 80;
            if let Some(ports) = c.get("Ports").and_then(|p| p.as_array()) {
                for p in ports {
                    if let Some(priv_port) = p.get("PrivatePort").and_then(|v| v.as_u64()) {
                        port = priv_port as u16;
                        break;
                    }
                }
            }

            routes.push(RouteTarget {
                name: raw_name,
                compose_service,
                compose_project,
                port,
            });
        }
    }

    Ok(routes)
}

async fn send_html_error(stream: &mut TcpStream, status: u16, msg: &str) -> Result<()> {
    let body = format!(
        r#"<!DOCTYPE html>
<html>
<head><title>Norb Proxy Error</title>
<style>
body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; background: #0f172a; color: #f8fafc; display: flex; align-items: center; justify-content: center; height: 100vh; margin: 0; }}
.card {{ background: #1e293b; padding: 2rem; border-radius: 12px; border: 1px solid #334155; max-width: 500px; text-align: center; box-shadow: 0 4px 20px rgba(0,0,0,0.5); }}
h1 {{ color: #38bdf8; font-size: 1.5rem; margin-bottom: 1rem; }}
p {{ color: #94a3b8; line-height: 1.5; }}
</style>
</head>
<body>
<div class="card">
  <h1>Norb Gateway: {}</h1>
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
        let headers = "GET / HTTP/1.1\r\nHost: myapp.norb.local:8080\r\nUser-Agent: curl\r\n\r\n";
        assert_eq!(parse_host_header(headers), Some("myapp.norb.local".to_string()));

        let headers_lowercase = "get / http/1.1\r\nhost: backend.local\r\n\r\n";
        assert_eq!(parse_host_header(headers_lowercase), Some("backend.local".to_string()));

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
                compose_service: Some("web".to_string()),
                compose_project: Some("proj".to_string()),
                port: 3000,
            },
            RouteTarget {
                name: "db".to_string(),
                compose_service: None,
                compose_project: None,
                port: 5432,
            },
        ];

        assert_eq!(find_target_port("web-app.norb.local", &routes), Some(3000));
        assert_eq!(find_target_port("web.proj.norb.local", &routes), Some(3000));
        assert_eq!(find_target_port("db.norb.local", &routes), Some(5432));
        assert_eq!(find_target_port("unknown.norb.local", &routes), None);
    }
}

