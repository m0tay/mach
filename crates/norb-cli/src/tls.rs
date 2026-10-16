//! TLS certificate generation and management for `*.norb.local`.
//!
//! Generates a local self-signed wildcard TLS certificate and key in `~/.norb/ssl/`
//! using macOS `/usr/bin/openssl`.

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct TlsConfig {
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
}

/// Ensure local wildcard TLS certificate exists for `*.norb.local`
pub fn ensure_tls_cert(data_dir: &Path) -> Result<TlsConfig> {
    let ssl_dir = data_dir.join("ssl");
    fs::create_dir_all(&ssl_dir)
        .with_context(|| format!("Failed to create SSL dir: {}", ssl_dir.display()))?;

    let cert_path = ssl_dir.join("cert.pem");
    let key_path = ssl_dir.join("key.pem");

    if !cert_path.exists() || !key_path.exists() {
        generate_wildcard_cert(&cert_path, &key_path)?;
    }

    Ok(TlsConfig {
        cert_path,
        key_path,
    })
}

fn generate_wildcard_cert(cert_path: &Path, key_path: &Path) -> Result<()> {
    println!("\x1b[1;36m➜\x1b[0m Generating self-signed TLS certificate for *.norb.local...");

    let status = Command::new("/usr/bin/openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-keyout",
            key_path.to_str().unwrap(),
            "-out",
            cert_path.to_str().unwrap(),
            "-days",
            "3650",
            "-subj",
            "/CN=*.norb.local",
            "-addext",
            "subjectAltName=DNS:*.norb.local,DNS:norb.local,IP:127.0.0.1",
        ])
        .output()
        .context("Failed to run /usr/bin/openssl")?;

    if !status.status.success() {
        let err = String::from_utf8_lossy(&status.stderr);
        anyhow::bail!("OpenSSL failed to generate certificate: {}", err.trim());
    }

    Ok(())
}
