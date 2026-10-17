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
    Open {
        target: String,
    },
    Pbcopy {
        text: String,
    },
    Pbpaste,
    Code {
        path: String,
    },
    Exec {
        binary: String,
        args: Vec<String>,
        #[serde(default)]
        cwd: Option<String>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BridgeResponse {
    ExecResult {
        exit_code: i32,
        stdout: String,
        stderr: String,
    },
    Status {
        success: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
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
            return BridgeResponse::Status {
                success: false,
                data: None,
                error: Some(format!("Invalid bridge JSON request: {e}")),
            };
        }
    };

    match req {
        BridgeRequest::Open { target } => match Command::new("/usr/bin/open").arg(&target).output() {
            Ok(out) if out.status.success() => BridgeResponse::Status {
                success: true,
                data: None,
                error: None,
            },
            Ok(out) => BridgeResponse::Status {
                success: false,
                data: None,
                error: Some(String::from_utf8_lossy(&out.stderr).into_owned()),
            },
            Err(e) => BridgeResponse::Status {
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
                    BridgeResponse::Status {
                        success: true,
                        data: None,
                        error: None,
                    }
                }
                Err(e) => BridgeResponse::Status {
                    success: false,
                    data: None,
                    error: Some(e.to_string()),
                },
            }
        }
        BridgeRequest::Pbpaste => match Command::new("/usr/bin/pbpaste").output() {
            Ok(out) => BridgeResponse::Status {
                success: true,
                data: Some(String::from_utf8_lossy(&out.stdout).into_owned()),
                error: None,
            },
            Err(e) => BridgeResponse::Status {
                success: false,
                data: None,
                error: Some(e.to_string()),
            },
        },
        BridgeRequest::Code { path } => match Command::new("code").arg(&path).output() {
            Ok(out) if out.status.success() => BridgeResponse::Status {
                success: true,
                data: None,
                error: None,
            },
            Ok(out) => BridgeResponse::Status {
                success: false,
                data: None,
                error: Some(String::from_utf8_lossy(&out.stderr).into_owned()),
            },
            Err(e) => BridgeResponse::Status {
                success: false,
                data: None,
                error: Some(format!("VS Code CLI 'code' not found on macOS: {e}")),
            },
        },
        BridgeRequest::Exec { binary, args, cwd } => {
            // Verify binary is safe: no path separators or slashes
            if binary.contains('/') || binary.contains('\\') || binary.trim().is_empty() {
                return BridgeResponse::ExecResult {
                    exit_code: 1,
                    stdout: String::new(),
                    stderr: format!("Invalid or unsafe binary name: '{binary}' (path separators not allowed)"),
                };
            }

            // Find valid command name via `which <binary>`
            let resolved_path = match Command::new("which").arg(&binary).output() {
                Ok(out) if out.status.success() => {
                    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if !p.is_empty() && Path::new(&p).is_file() {
                        Some(p)
                    } else {
                        None
                    }
                }
                _ => None,
            };

            let resolved_bin = match resolved_path {
                Some(p) => p,
                None => {
                    let fallback_dirs = ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"];
                    let mut found = None;
                    for dir in &fallback_dirs {
                        let candidate = Path::new(dir).join(&binary);
                        if candidate.is_file() {
                            found = Some(candidate.to_string_lossy().to_string());
                            break;
                        }
                    }
                    match found {
                        Some(p) => p,
                        None => {
                            return BridgeResponse::ExecResult {
                                exit_code: 127,
                                stdout: String::new(),
                                stderr: format!("Command not found: '{binary}'"),
                            };
                        }
                    }
                }
            };

            let mut cmd = Command::new(&resolved_bin);
            cmd.args(&args);
            if let Some(ref dir) = cwd {
                let p = Path::new(dir);
                if p.is_dir() {
                    cmd.current_dir(p);
                }
            }

            match cmd.output() {
                Ok(out) => {
                    let exit_code = out.status.code().unwrap_or(if out.status.success() { 0 } else { 1 });
                    BridgeResponse::ExecResult {
                        exit_code,
                        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
                        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
                    }
                }
                Err(e) => BridgeResponse::ExecResult {
                    exit_code: 1,
                    stdout: String::new(),
                    stderr: format!("Execution failed: {e}"),
                },
            }
        }
    }
}

/// Script content to be installed in guest at `/usr/local/bin/mac`
pub const GUEST_MAC_SCRIPT: &str = r#"#!/usr/bin/env bash
# macOS host bridge client for mach
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
  exec)
    bin="$1"
    [ -z "$bin" ] && echo "Usage: mac exec <binary> [args...]" && exit 1
    shift || true
    python3 -c "
import socket, json, sys, os
s = socket.socket()
s.connect(('$GATEWAY_IP', $PORT))
req = json.dumps({
    'action': 'exec',
    'binary': sys.argv[1],
    'args': sys.argv[2:],
    'cwd': os.getcwd()
}) + '\n'
s.sendall(req.encode())
f = s.makefile()
line = f.readline()
if not line:
    print('Error: no response from host bridge', file=sys.stderr)
    sys.exit(1)
resp = json.loads(line)
if 'exit_code' in resp:
    sys.stdout.write(resp.get('stdout', ''))
    sys.stderr.write(resp.get('stderr', ''))
    sys.exit(resp.get('exit_code', 0))
elif not resp.get('success'):
    print(resp.get('error', 'Unknown error'), file=sys.stderr)
    sys.exit(1)
" "$bin" "$@"
    ;;
  link)
    target="$1"
    [ -z "$target" ] && echo "Usage: mac link <binary>" && exit 1
    mkdir -p /usr/local/bin
    wrapper="/usr/local/bin/$target"
    cat << EOF > "$wrapper"
#!/usr/bin/env bash
exec /usr/local/bin/mac exec $target "\$@"
EOF
    chmod +x "$wrapper"
    echo "Linked host binary '$target' -> $wrapper"
    ;;
  *)
    echo "Usage: mac {open|pbcopy|pbpaste|code|exec|link} [args...]"
    exit 1
    ;;
esac
"#;

/// Helper to install `/usr/local/bin/mac` in guest over SSH
pub fn install_guest_mac_cli(ip: IpAddr) -> Result<()> {
    let script = format!(
        "mkdir -p /usr/local/bin && cat << 'EOF' > /usr/local/bin/mac\n{}\nEOF\nchmod +x /usr/local/bin/mac; ln -sf /usr/local/bin/mac /usr/local/bin/pbcopy; ln -sf /usr/local/bin/mac /usr/local/bin/pbpaste",
        GUEST_MAC_SCRIPT
    );

    let mut cmd = Command::new("ssh");
    cmd.args(crate::ssh::mach_ssh_args());
    cmd.args([
        "-o", "ConnectTimeout=2",
        &format!("root@{ip}"),
        &script,
    ]);
    let _ = cmd.output();

    Ok(())
}

/// Helper to create an executable wrapper script in `/usr/local/bin/<binary>` inside the guest
pub fn install_guest_link(ip: IpAddr, binary: &str) -> Result<()> {
    let script = format!(
        "mkdir -p /usr/local/bin && cat << 'EOF' > /usr/local/bin/{binary}\n#!/usr/bin/env bash\nexec /usr/local/bin/mac exec {binary} \"$@\"\nEOF\nchmod +x /usr/local/bin/{binary}"
    );

    let mut cmd = Command::new("ssh");
    cmd.args(crate::ssh::mach_ssh_args());
    cmd.args([
        "-o", "ConnectTimeout=5",
        &format!("root@{ip}"),
        &script,
    ]);
    let out = cmd.output().with_context(|| format!("Failed to create link for '{binary}' in guest"))?;
    if !out.status.success() {
        anyhow::bail!("SSH failed to link binary '{binary}': {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exec_request_deserialization() {
        let json = r#"{"action": "exec", "binary": "echo", "args": ["hello", "world"]}"#;
        let req: BridgeRequest = serde_json::from_str(json).unwrap();
        match req {
            BridgeRequest::Exec { binary, args, cwd } => {
                assert_eq!(binary, "echo");
                assert_eq!(args, vec!["hello", "world"]);
                assert_eq!(cwd, None);
            }
            _ => panic!("Expected Exec variant"),
        }
    }

    #[test]
    fn test_exec_response_serialization() {
        let resp = BridgeResponse::ExecResult {
            exit_code: 0,
            stdout: "hello\n".to_string(),
            stderr: String::new(),
        };
        let s = serde_json::to_string(&resp).unwrap();
        assert!(s.contains("\"exit_code\":0"));
        assert!(s.contains("\"stdout\":\"hello\\n\""));
    }

    #[test]
    fn test_handle_request_unsafe_binary() {
        let json = r#"{"action": "exec", "binary": "../bin/sh", "args": []}"#;
        let resp = handle_request(json);
        match resp {
            BridgeResponse::ExecResult { exit_code, stderr, .. } => {
                assert_eq!(exit_code, 1);
                assert!(stderr.contains("path separators not allowed"));
            }
            _ => panic!("Expected ExecResult"),
        }
    }

    #[test]
    fn test_handle_request_nonexistent_binary() {
        let json = r#"{"action": "exec", "binary": "mach_nonexistent_binary_xyz", "args": []}"#;
        let resp = handle_request(json);
        match resp {
            BridgeResponse::ExecResult { exit_code, stderr, .. } => {
                assert_eq!(exit_code, 127);
                assert!(stderr.contains("Command not found"));
            }
            _ => panic!("Expected ExecResult"),
        }
    }

    #[test]
    fn test_handle_request_valid_echo() {
        let json = r#"{"action": "exec", "binary": "echo", "args": ["mach", "bridge", "test"]}"#;
        let resp = handle_request(json);
        match resp {
            BridgeResponse::ExecResult { exit_code, stdout, .. } => {
                assert_eq!(exit_code, 0);
                assert_eq!(stdout.trim(), "mach bridge test");
            }
            _ => panic!("Expected ExecResult"),
        }
    }
}
