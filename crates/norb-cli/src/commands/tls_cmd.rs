//! TLS certificate authority and trust commands.

use anyhow::{Context, Result};
use std::path::PathBuf;
use tokio::process::Command;

use norb_common::config::NorbConfig;
use crate::tls::{
    self, ca_cert_path, ca_key_path, cert_path, check_keychain_trust, get_cert_expiry,
    get_cert_fingerprint, is_cert_valid, key_path, verify_cert_chain, KeychainTrustState,
    ROOT_CA_COMMON_NAME,
};

/// Install Norb Root CA into macOS Keychain trust store.
pub async fn install(system: bool) -> Result<()> {
    let cfg = NorbConfig::load().unwrap_or_default();

    // Ensure Root CA and wildcard certificates are generated
    let tls_config = tls::ensure_tls_cert(&cfg.data_dir)
        .context("Failed to ensure Root CA and wildcard TLS certificates")?;

    let ca_cert = &tls_config.ca_cert_path;
    if !ca_cert.exists() {
        anyhow::bail!("Root CA certificate not found at {}", ca_cert.display());
    }

    let home = dirs::home_dir().context("Failed to determine user home directory")?;
    let login_db = home.join("Library/Keychains/login.keychain-db");
    let login_kc = home.join("Library/Keychains/login.keychain");

    let keychain_path: PathBuf = if system {
        PathBuf::from("/Library/Keychains/System.keychain")
    } else if login_db.exists() {
        login_db
    } else if login_kc.exists() {
        login_kc
    } else {
        login_db
    };

    println!("\x1b[1;36m➜\x1b[0m Installing Norb Root CA into macOS Keychain...");
    println!("  Certificate: {}", ca_cert.display());
    println!("  Keychain:    {}", keychain_path.display());

    let mut cmd = Command::new("/usr/bin/security");
    cmd.args([
        "add-trusted-cert",
        "-d",
        "-r",
        "trustRoot",
        "-k",
        keychain_path.to_str().unwrap(),
        ca_cert.to_str().unwrap(),
    ]);

    let status = cmd.status().await.context("Failed to run /usr/bin/security")?;

    if !status.success() {
        let code = status.code().unwrap_or(-1);
        if system {
            anyhow::bail!(
                "Failed to install into System keychain (exit code {}). Try running with sudo: sudo norb tls install --system",
                code
            );
        } else {
            anyhow::bail!(
                "Failed to install into login keychain (exit code {}). If prompted, enter your macOS password to authorize.",
                code
            );
        }
    }

    println!("\x1b[1;32m✔\x1b[0m Norb Root CA successfully installed and trusted in macOS Keychain!");
    println!("  Local HTTPS domains (*.norb.local) are now trusted across macOS browsers and CLI tools.");

    Ok(())
}

/// Uninstall Norb Root CA from macOS Keychain trust store.
pub async fn uninstall() -> Result<()> {
    let cfg = NorbConfig::load().unwrap_or_default();
    let ca_cert = ca_cert_path(&cfg.data_dir);

    if !ca_cert.exists() {
        println!(
            "\x1b[1;33m⚠\x1b[0m Norb Root CA certificate not found at {}. Nothing to uninstall.",
            ca_cert.display()
        );
        return Ok(());
    }

    println!("\x1b[1;36m➜\x1b[0m Removing Norb Root CA from macOS Keychain...");

    // Remove from admin domain
    let out_admin = Command::new("/usr/bin/security")
        .args(["remove-trusted-cert", "-d", ca_cert.to_str().unwrap()])
        .output()
        .await;

    // Remove from user domain
    let out_user = Command::new("/usr/bin/security")
        .args(["remove-trusted-cert", ca_cert.to_str().unwrap()])
        .output()
        .await;

    // Delete matching certificate entries
    let _ = Command::new("/usr/bin/security")
        .args(["delete-certificate", "-c", ROOT_CA_COMMON_NAME])
        .output()
        .await;

    let admin_ok = out_admin.as_ref().map(|o| o.status.success()).unwrap_or(false);
    let user_ok = out_user.as_ref().map(|o| o.status.success()).unwrap_or(false);

    if admin_ok || user_ok {
        println!("\x1b[1;32m✔\x1b[0m Norb Root CA trust settings successfully removed from macOS Keychain.");
    } else {
        println!("\x1b[1;32m✔\x1b[0m Norb Root CA trust settings cleared.");
    }

    Ok(())
}

/// Check TLS CA and certificate status.
pub async fn status() -> Result<()> {
    let cfg = NorbConfig::load().unwrap_or_default();

    let ca_crt = ca_cert_path(&cfg.data_dir);
    let ca_k = ca_key_path(&cfg.data_dir);
    let leaf_crt = cert_path(&cfg.data_dir);
    let leaf_k = key_path(&cfg.data_dir);

    println!("\x1b[1mNorb TLS & PKI Status\x1b[0m");
    println!("────────────────────────────────────────────────────────────");

    // 1. Root CA Status
    println!("\x1b[1;34mRoot CA:\x1b[0m");
    println!("  Certificate:  {}", ca_crt.display());
    println!("  Private Key:  {}", ca_k.display());

    if ca_crt.exists() {
        let expiry = get_cert_expiry(&ca_crt).unwrap_or_else(|| "Unknown".to_string());
        let fp = get_cert_fingerprint(&ca_crt).unwrap_or_else(|| "Unknown".to_string());
        let key_ok = if ca_k.exists() { "Present" } else { "Missing" };

        println!("  Status:       \x1b[1;32mInstalled\x1b[0m (Key: {})", key_ok);
        println!("  Expires:      {}", expiry);
        println!("  Fingerprint:  {}", fp);

        let trust = check_keychain_trust(&ca_crt);
        match trust {
            KeychainTrustState::TrustedUser => {
                println!("  Trust:        \x1b[1;32m✔ Trusted in macOS Keychain (User domain)\x1b[0m");
            }
            KeychainTrustState::TrustedSystem => {
                println!("  Trust:        \x1b[1;32m✔ Trusted in macOS Keychain (System domain)\x1b[0m");
            }
            KeychainTrustState::NotTrusted => {
                println!("  Trust:        \x1b[1;33m⚠ Not trusted in macOS Keychain\x1b[0m");
                println!("                \x1b[2mRun 'norb tls install' to trust local HTTPS certs.\x1b[0m");
            }
        }
    } else {
        println!("  Status:       \x1b[1;33mNot generated\x1b[0m");
        println!("                \x1b[2mRun 'norb tls install' to generate and trust.\x1b[0m");
    }

    println!();

    // 2. Wildcard Leaf Certificate Status
    println!("\x1b[1;34mWildcard Certificate (*.norb.local):\x1b[0m");
    println!("  Certificate:  {}", leaf_crt.display());
    println!("  Private Key:  {}", leaf_k.display());

    if leaf_crt.exists() {
        let expiry = get_cert_expiry(&leaf_crt).unwrap_or_else(|| "Unknown".to_string());
        let valid = is_cert_valid(&leaf_crt);
        let chain_ok = verify_cert_chain(&ca_crt, &leaf_crt);

        let status_str = if valid {
            "\x1b[1;32mValid\x1b[0m"
        } else {
            "\x1b[1;31mExpired / Invalid\x1b[0m"
        };

        println!("  Status:       {}", status_str);
        println!("  Expires:      {}", expiry);
        println!("  Domains:      *.norb.local, norb.local, 127.0.0.1");

        if chain_ok {
            println!("  Chain:        \x1b[1;32m✔ Verified against Norb Root CA\x1b[0m");
        } else {
            println!("  Chain:        \x1b[1;33m⚠ Chain verification failed\x1b[0m");
        }
    } else {
        println!("  Status:       \x1b[1;33mNot generated\x1b[0m");
        println!("                \x1b[2mGenerated automatically on first proxy start or 'norb tls install'.\x1b[0m");
    }

    println!("────────────────────────────────────────────────────────────");

    Ok(())
}
