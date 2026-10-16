//! Local HTTP reverse proxy for `*.norb.local` container domains.
//!
//! Listens on `127.0.0.1:80` (with automatic fallback to `127.0.0.1:8080`) and routes
//! incoming HTTP requests to the appropriate Docker container in the guest VM.

use anyhow::Result;
use serde_json::Value;
use std::net::SocketAddr;
use std::path::Path;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UnixStream};

use crate::commands::load_all_machines;

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
    let mut header_buf = [0u8; 4096];
    let n = client.peek(&mut header_buf).await?;
    if n == 0 {
        return Ok(());
    }

    let header_str = String::from_utf8_lossy(&header_buf[..n]);
    let host = parse_host_header(&header_str);

    let host = match host {
        Some(h) => h,
        None => {
            let resp = "HTTP/1.1 400 Bad Request\r\nContent-Length: 16\r\n\r\nMissing Host header";
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

    let _ = tokio::io::copy_bidirectional(&mut client, &mut target).await;
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

    let body = match split_pos {
        Some(pos) => &buf[pos..],
        None => return Ok(Vec::new()),
    };

    let containers: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return Ok(Vec::new()),
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
