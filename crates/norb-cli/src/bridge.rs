//! Host command bridge for macOS integration.
//!
//! Enables Linux guests and containers to run macOS commands like:
//! - `mac open <file_or_url>` (opens in default macOS browser or app)
//! - `mac pbcopy` / `mac pbpaste` (macOS clipboard sync)
//! - `mac code <path>` (opens file or directory in macOS VS Code)

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::process::{Command, Stdio};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum BridgeRequest {
    Open { target: String },
    Pbcopy { text: String },
    Pbpaste,
    Code { path: String },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BridgeResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Check if a peer IP is permitted to communicate with the host bridge.
/// Permitted: loopback (127.0.0.1 / ::1) or macOS hypervisor bridge network (192.168.64.0/24).
fn is_allowed_peer(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback() || (v4.octets()[0] == 192 && v4.octets()[1] == 168 && v4.octets()[2] == 64)
        }
        IpAddr::V6(v6) => v6.is_loopback(),
    }
}

async fn handle_client(mut socket: tokio::net::TcpStream, peer: SocketAddr) {
    if !is_allowed_peer(&peer.ip()) {
        tracing::warn!(peer = %peer, "Host bridge rejected unauthorized external connection");
        return;
    }

    let (reader, mut writer) = socket.split();
    let mut buf_reader = BufReader::new(reader);
    let mut line = String::new();

    if buf_reader.read_line(&mut line).await.is_ok() {
        let resp = handle_request(&line);
        let serialized = serde_json::to_string(&resp).unwrap_or_default() + "\n";
        let _ = writer.write_all(serialized.as_bytes()).await;
    }
}

/// Run the background TCP listener for the macOS host bridge on port 32222.
/// Binds to 0.0.0.0:32222 with strict IP verification to only accept connections
/// from 127.0.0.1 or 192.168.64.0/24.
pub async fn run_host_bridge(_run_dir: &Path) -> Result<()> {
    let addr = SocketAddr::from(([0, 0, 0, 0], 32222));
    let listener = TcpListener::bind(addr)
        .await
        .context("Failed to bind host bridge on port 32222")?;

    loop {
        match listener.accept().await {
            Ok((socket, peer)) => {
                tokio::spawn(handle_client(socket, peer));
            }
            Err(e) => {
                eprintln!("[bridge] Accept error: {e:?}");
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            }
        }
    }
}

fn handle_request(raw_json: &str) -> BridgeResponse {
    let req: BridgeRequest = match serde_json::from_str(raw_json.trim()) {
        Ok(r) => r,
        Err(e) => {
            return BridgeResponse {
                success: false,
                data: None,
                error: Some(format!("Invalid bridge JSON request: {e}")),
            };
        }
    };

    match req {
        BridgeRequest::Open { target } => match Command::new("/usr/bin/open").arg(&target).output() {
            Ok(out) if out.status.success() => BridgeResponse {
                success: true,
                data: None,
                error: None,
            },
            Ok(out) => BridgeResponse {
                success: false,
                data: None,
                error: Some(String::from_utf8_lossy(&out.stderr).into_owned()),
            },
            Err(e) => BridgeResponse {
                success: false,
                data: None,
                error: Some(e.to_string()),
            },
        },
        BridgeRequest::Pbcopy { text } => {
            match Command::new("/usr/bin/pbcopy")
                .stdin(Stdio::piped())
                .spawn()
            {
                Ok(mut child) => {
                    if let Some(mut stdin) = child.stdin.take() {
                        let _ = stdin.write_all(text.as_bytes());
                    }
                    let _ = child.wait();
                    BridgeResponse {
                        success: true,
                        data: None,
                        error: None,
                    }
                }
                Err(e) => BridgeResponse {
                    success: false,
                    data: None,
                    error: Some(e.to_string()),
                },
            }
        }
        BridgeRequest::Pbpaste => match Command::new("/usr/bin/pbpaste").output() {
            Ok(out) => BridgeResponse {
                success: true,
                data: Some(String::from_utf8_lossy(&out.stdout).into_owned()),
                error: None,
            },
            Err(e) => BridgeResponse {
                success: false,
                data: None,
                error: Some(e.to_string()),
            },
        },
        BridgeRequest::Code { path } => match Command::new("code").arg(&path).output() {
            Ok(out) if out.status.success() => BridgeResponse {
                success: true,
                data: None,
                error: None,
            },
            Ok(out) => BridgeResponse {
                success: false,
                data: None,
                error: Some(String::from_utf8_lossy(&out.stderr).into_owned()),
            },
            Err(e) => BridgeResponse {
                success: false,
                data: None,
                error: Some(format!("VS Code CLI 'code' not found on macOS: {e}")),
            },
        },
    }
}

/// Script content to be installed in guest at `/usr/local/bin/mac`
pub const GUEST_MAC_SCRIPT: &str = r#"#!/usr/bin/env bash
# macOS host bridge client for Norb
GATEWAY_IP="192.168.64.1"
PORT=32222

cmd="$1"
shift || true

case "$cmd" in
  open)
    target="$1"
    [ -z "$target" ] && echo "Usage: mac open <url_or_path>" && exit 1
    # Convert path if in /Users
    python3 -c "
import socket, json, sys
s = socket.socket()
s.connect(('$GATEWAY_IP', $PORT))
req = json.dumps({'action': 'open', 'target': sys.argv[1]}) + '\n'
s.sendall(req.encode())
resp = json.loads(s.makefile().readline())
if not resp.get('success'):
    print(resp.get('error', 'Unknown error'), file=sys.stderr)
    sys.exit(1)
" "$target"
    ;;
  pbcopy)
    python3 -c "
import socket, json, sys
data = sys.stdin.read()
s = socket.socket()
s.connect(('$GATEWAY_IP', $PORT))
req = json.dumps({'action': 'pbcopy', 'text': data}) + '\n'
s.sendall(req.encode())
s.close()
"
    ;;
  pbpaste)
    python3 -c "
import socket, json, sys
s = socket.socket()
s.connect(('$GATEWAY_IP', $PORT))
req = json.dumps({'action': 'pbpaste'}) + '\n'
s.sendall(req.encode())
resp = json.loads(s.makefile().readline())
if resp.get('success') and resp.get('data') is not None:
    sys.stdout.write(resp['data'])
"
    ;;
  code)
    target="${1:-.}"
    python3 -c "
import socket, json, sys
s = socket.socket()
s.connect(('$GATEWAY_IP', $PORT))
req = json.dumps({'action': 'code', 'path': sys.argv[1]}) + '\n'
s.sendall(req.encode())
resp = json.loads(s.makefile().readline())
if not resp.get('success'):
    print(resp.get('error', 'Unknown error'), file=sys.stderr)
    sys.exit(1)
" "$target"
    ;;
  *)
    echo "Usage: mac {open|pbcopy|pbpaste|code} [args...]"
    exit 1
    ;;
esac
"#;

/// Helper to install `/usr/local/bin/mac` in guest over SSH
pub fn install_guest_mac_cli(ip: IpAddr) -> Result<()> {
    let script = format!(
        "cat << 'EOF' > /usr/local/bin/mac\n{}\nEOF\nchmod +x /usr/local/bin/mac; ln -sf /usr/local/bin/mac /usr/local/bin/pbcopy; ln -sf /usr/local/bin/mac /usr/local/bin/pbpaste",
        GUEST_MAC_SCRIPT
    );

    let mut cmd = Command::new("ssh");
    cmd.args(crate::ssh::norb_ssh_args());
    cmd.args([
        "-o", "ConnectTimeout=2",
        &format!("root@{ip}"),
        &script,
    ]);
    let _ = cmd.output();

    Ok(())
}
