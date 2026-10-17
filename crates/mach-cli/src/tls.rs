//! TLS certificate authority and certificate management for Mach.
//!
//! Generates and manages:
//! 1. A persistent local Root CA (`~/.mach/ssl/mach-root-ca.crt` & key), valid for 10 years.
//! 2. Dynamic wildcard leaf certificates for `*.mach.local`, `mach.local`, and `127.0.0.1`
//!    signed by the Mach Root CA.
//! 3. PKI utilities to verify certificate validity, chains, and macOS Keychain trust status.

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const ROOT_CA_CERT_NAME: &str = "mach-root-ca.crt";
pub const ROOT_CA_KEY_NAME: &str = "mach-root-ca.key";
pub const LEAF_CERT_NAME: &str = "cert.pem";
pub const LEAF_KEY_NAME: &str = "key.pem";
pub const WILDCARD_CERT_ALIAS: &str = "mach-wildcard.crt";
pub const WILDCARD_KEY_ALIAS: &str = "mach-wildcard.key";

pub const ROOT_CA_COMMON_NAME: &str = "Mach Local Root CA";
pub const ROOT_CA_SUBJECT: &str = "/CN=Mach Local Root CA/O=Mach Virtualization/OU=Local Development";
pub const LEAF_SUBJECT: &str = "/CN=*.mach.local/O=Mach Virtualization/OU=Local Development";

#[derive(Debug, Clone)]
pub struct TlsConfig {
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
    pub ca_cert_path: PathBuf,
    pub ca_key_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeychainTrustState {
    TrustedUser,
    TrustedSystem,
    NotTrusted,
}

/// Returns the path to the Root CA certificate (`<data_dir>/ssl/mach-root-ca.crt`).
pub fn ca_cert_path(data_dir: &Path) -> PathBuf {
    data_dir.join("ssl").join(ROOT_CA_CERT_NAME)
}

/// Returns the path to the Root CA private key (`<data_dir>/ssl/mach-root-ca.key`).
pub fn ca_key_path(data_dir: &Path) -> PathBuf {
    data_dir.join("ssl").join(ROOT_CA_KEY_NAME)
}

/// Returns the path to the wildcard leaf certificate (`<data_dir>/ssl/cert.pem`).
pub fn cert_path(data_dir: &Path) -> PathBuf {
    data_dir.join("ssl").join(LEAF_CERT_NAME)
}

/// Returns the path to the wildcard leaf private key (`<data_dir>/ssl/key.pem`).
pub fn key_path(data_dir: &Path) -> PathBuf {
    data_dir.join("ssl").join(LEAF_KEY_NAME)
}

/// Ensure local persistent Root CA exists in `<data_dir>/ssl/mach-root-ca.crt` & key.
/// Valid for 10 years (3650 days).
pub fn ensure_ca_cert(data_dir: &Path) -> Result<(PathBuf, PathBuf)> {
    let ssl_dir = data_dir.join("ssl");
    fs::create_dir_all(&ssl_dir)
        .with_context(|| format!("Failed to create SSL directory: {}", ssl_dir.display()))?;

    let ca_crt = ca_cert_path(data_dir);
    let ca_key = ca_key_path(data_dir);

    if !ca_crt.exists() || !ca_key.exists() {
        generate_root_ca(&ca_crt, &ca_key)?;
    }

    Ok((ca_crt, ca_key))
}

/// Ensure local wildcard TLS certificate exists for `*.mach.local`, `mach.local`, and `127.0.0.1`,
/// signed by the persistent mach Root CA.
pub fn ensure_tls_cert(data_dir: &Path) -> Result<TlsConfig> {
    let ssl_dir = data_dir.join("ssl");
    fs::create_dir_all(&ssl_dir)
        .with_context(|| format!("Failed to create SSL directory: {}", ssl_dir.display()))?;

    let (ca_cert_path, ca_key_path) = ensure_ca_cert(data_dir)?;

    let leaf_cert = cert_path(data_dir);
    let leaf_key = key_path(data_dir);

    let needs_generation = !leaf_cert.exists()
        || !leaf_key.exists()
        || !is_cert_valid(&leaf_cert);

    if needs_generation {
        generate_wildcard_cert(&ca_cert_path, &ca_key_path, &leaf_cert, &leaf_key)?;
    }

    // Ensure alias files exist (mach-wildcard.crt / mach-wildcard.key)
    let alias_cert = ssl_dir.join(WILDCARD_CERT_ALIAS);
    let alias_key = ssl_dir.join(WILDCARD_KEY_ALIAS);
    if !alias_cert.exists() && leaf_cert.exists() {
        let _ = fs::copy(&leaf_cert, &alias_cert);
    }
    if !alias_key.exists() && leaf_key.exists() {
        let _ = fs::copy(&leaf_key, &alias_key);
    }

    Ok(TlsConfig {
        cert_path: leaf_cert,
        key_path: leaf_key,
        ca_cert_path,
        ca_key_path,
    })
}

/// Generates a self-signed Root CA valid for 10 years (3650 days).
fn generate_root_ca(ca_cert: &Path, ca_key: &Path) -> Result<()> {
    println!("\x1b[1;36m➜\x1b[0m Generating persistent mach Root CA in {}...", ca_cert.display());

    let output = Command::new("/usr/bin/openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-keyout",
            ca_key.to_str().unwrap(),
            "-sha256",
            "-days",
            "3650",
            "-out",
            ca_cert.to_str().unwrap(),
            "-subj",
            ROOT_CA_SUBJECT,
            "-addext",
            "basicConstraints=critical,CA:TRUE",
            "-addext",
            "keyUsage=critical,keyCertSign,cRLSign",
            "-addext",
            "subjectKeyIdentifier=hash",
        ])
        .output()
        .context("Failed to run /usr/bin/openssl to generate Root CA")?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("OpenSSL failed to generate Root CA: {}", err.trim());
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(ca_key, fs::Permissions::from_mode(0o600));
    }

    Ok(())
}

/// Generates a dynamic wildcard leaf certificate signed by the Root CA.
fn generate_wildcard_cert(
    ca_cert: &Path,
    ca_key: &Path,
    leaf_cert: &Path,
    leaf_key: &Path,
) -> Result<()> {
    println!("\x1b[1;36m➜\x1b[0m Generating wildcard TLS certificate for *.mach.local...");

    let ssl_dir = leaf_cert.parent().context("Invalid cert path parent")?;
    let csr_path = ssl_dir.join(".mach-leaf.csr");
    let ext_path = ssl_dir.join(".mach-leaf.ext");
    let leaf_tmp = ssl_dir.join(".mach-leaf.tmp.crt");

    // 1. Generate private key for leaf cert
    let genkey = Command::new("/usr/bin/openssl")
        .args([
            "genrsa",
            "-out",
            leaf_key.to_str().unwrap(),
            "2048",
        ])
        .output()
        .context("Failed to generate leaf private key")?;

    if !genkey.status.success() {
        let err = String::from_utf8_lossy(&genkey.stderr);
        anyhow::bail!("OpenSSL failed to generate leaf private key: {}", err.trim());
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(leaf_key, fs::Permissions::from_mode(0o600));
    }

    // 2. Generate CSR
    let gencsr = Command::new("/usr/bin/openssl")
        .args([
            "req",
            "-new",
            "-key",
            leaf_key.to_str().unwrap(),
            "-out",
            csr_path.to_str().unwrap(),
            "-subj",
            LEAF_SUBJECT,
        ])
        .output()
        .context("Failed to generate leaf CSR")?;

    if !gencsr.status.success() {
        let err = String::from_utf8_lossy(&gencsr.stderr);
        let _ = fs::remove_file(&csr_path);
        anyhow::bail!("OpenSSL failed to generate leaf CSR: {}", err.trim());
    }

    // 3. Write OpenSSL extension config with SANs
    let ext_content = "\
authorityKeyIdentifier=keyid,issuer
basicConstraints=CA:FALSE
keyUsage=digitalSignature,keyEncipherment
extendedKeyUsage=serverAuth,clientAuth
subjectAltName=@alt_names

[alt_names]
DNS.1 = *.mach.local
DNS.2 = mach.local
DNS.3 = localhost
IP.1 = 127.0.0.1
";
    fs::write(&ext_path, ext_content)
        .with_context(|| format!("Failed to write extension file: {}", ext_path.display()))?;

    // 4. Sign leaf cert with Root CA
    let sign = Command::new("/usr/bin/openssl")
        .args([
            "x509",
            "-req",
            "-in",
            csr_path.to_str().unwrap(),
            "-CA",
            ca_cert.to_str().unwrap(),
            "-CAkey",
            ca_key.to_str().unwrap(),
            "-CAcreateserial",
            "-out",
            leaf_tmp.to_str().unwrap(),
            "-days",
            "365",
            "-sha256",
            "-extfile",
            ext_path.to_str().unwrap(),
        ])
        .output()
        .context("Failed to sign leaf certificate with Root CA")?;

    // Clean up temporary CSR and extension file
    let _ = fs::remove_file(&csr_path);
    let _ = fs::remove_file(&ext_path);

    if !sign.status.success() {
        let _ = fs::remove_file(&leaf_tmp);
        let err = String::from_utf8_lossy(&sign.stderr);
        anyhow::bail!("OpenSSL failed to sign leaf certificate: {}", err.trim());
    }

    // 5. Construct fullchain: leaf certificate followed by Root CA certificate
    let leaf_bytes = fs::read(&leaf_tmp).context("Failed to read leaf certificate")?;
    let ca_bytes = fs::read(ca_cert).context("Failed to read CA certificate")?;
    let _ = fs::remove_file(&leaf_tmp);

    let mut fullchain = leaf_bytes;
    if !fullchain.ends_with(b"\n") {
        fullchain.push(b'\n');
    }
    fullchain.extend_from_slice(&ca_bytes);

    fs::write(leaf_cert, fullchain)
        .with_context(|| format!("Failed to write fullchain certificate to {}", leaf_cert.display()))?;

    Ok(())
}

/// Checks whether a certificate exists and is valid for at least another 24 hours.
pub fn is_cert_valid(cert_path: &Path) -> bool {
    if !cert_path.exists() {
        return false;
    }
    let output = Command::new("/usr/bin/openssl")
        .args([
            "x509",
            "-checkend",
            "86400",
            "-noout",
            "-in",
            cert_path.to_str().unwrap(),
        ])
        .output();

    matches!(output, Ok(out) if out.status.success())
}

/// Returns the formatted expiration date (`notAfter`) of a certificate.
pub fn get_cert_expiry(cert_path: &Path) -> Option<String> {
    if !cert_path.exists() {
        return None;
    }
    let output = Command::new("/usr/bin/openssl")
        .args(["x509", "-enddate", "-noout", "-in", cert_path.to_str().unwrap()])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        if let Some(date) = line.strip_prefix("notAfter=") {
            return Some(date.trim().to_string());
        }
    }
    None
}

/// Returns the SHA256 fingerprint of a certificate.
pub fn get_cert_fingerprint(cert_path: &Path) -> Option<String> {
    if !cert_path.exists() {
        return None;
    }
    let output = Command::new("/usr/bin/openssl")
        .args([
            "x509",
            "-fingerprint",
            "-sha256",
            "-noout",
            "-in",
            cert_path.to_str().unwrap(),
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        if let Some(fp) = line.strip_prefix("SHA256 Fingerprint=") {
            return Some(fp.trim().to_string());
        }
    }
    None
}

/// Returns the Subject field of a certificate.
pub fn get_cert_subject(cert_path: &Path) -> Option<String> {
    if !cert_path.exists() {
        return None;
    }
    let output = Command::new("/usr/bin/openssl")
        .args(["x509", "-subject", "-noout", "-in", cert_path.to_str().unwrap()])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        if let Some(subj) = line.strip_prefix("subject=") {
            return Some(subj.trim().to_string());
        }
    }
    None
}

/// Verifies a leaf certificate against a CA certificate using OpenSSL.
pub fn verify_cert_chain(ca_cert_path: &Path, leaf_cert_path: &Path) -> bool {
    if !ca_cert_path.exists() || !leaf_cert_path.exists() {
        return false;
    }
    let output = Command::new("/usr/bin/openssl")
        .args([
            "verify",
            "-CAfile",
            ca_cert_path.to_str().unwrap(),
            leaf_cert_path.to_str().unwrap(),
        ])
        .output();

    matches!(output, Ok(out) if out.status.success())
}

/// Checks whether the mach Root CA is trusted in the macOS Keychain.
pub fn check_keychain_trust(ca_cert_path: &Path) -> KeychainTrustState {
    if !ca_cert_path.exists() {
        return KeychainTrustState::NotTrusted;
    }

    // 1. Direct macOS security verify-cert
    let verify = Command::new("/usr/bin/security")
        .args(["verify-cert", "-c", ca_cert_path.to_str().unwrap(), "-l"])
        .output();

    if let Ok(out) = verify {
        if out.status.success() {
            // Verify if in admin or user store
            if let Ok(dump_admin) = Command::new("/usr/bin/security").args(["dump-trust-settings", "-d"]).output() {
                if String::from_utf8_lossy(&dump_admin.stdout).contains(ROOT_CA_COMMON_NAME) {
                    return KeychainTrustState::TrustedSystem;
                }
            }
            return KeychainTrustState::TrustedUser;
        }
    }

    // 2. Check user domain trust settings
    if let Ok(dump_user) = Command::new("/usr/bin/security").args(["dump-trust-settings"]).output() {
        if String::from_utf8_lossy(&dump_user.stdout).contains(ROOT_CA_COMMON_NAME) {
            return KeychainTrustState::TrustedUser;
        }
    }

    // 3. Check admin domain trust settings
    if let Ok(dump_admin) = Command::new("/usr/bin/security").args(["dump-trust-settings", "-d"]).output() {
        if String::from_utf8_lossy(&dump_admin.stdout).contains(ROOT_CA_COMMON_NAME) {
            return KeychainTrustState::TrustedSystem;
        }
    }

    KeychainTrustState::NotTrusted
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_test_data_dir() -> PathBuf {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("mach-tls-test-{}", ts));
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn test_paths() {
        let dummy = Path::new("/test/mach");
        assert_eq!(ca_cert_path(dummy), PathBuf::from("/test/mach/ssl/mach-root-ca.crt"));
        assert_eq!(ca_key_path(dummy), PathBuf::from("/test/mach/ssl/mach-root-ca.key"));
        assert_eq!(cert_path(dummy), PathBuf::from("/test/mach/ssl/cert.pem"));
        assert_eq!(key_path(dummy), PathBuf::from("/test/mach/ssl/key.pem"));
    }

    #[test]
    fn test_ensure_ca_and_leaf_generation() {
        let test_dir = make_test_data_dir();

        // 1. Ensure Root CA
        let (ca_crt, ca_key) = ensure_ca_cert(&test_dir).expect("ensure_ca_cert failed");
        assert!(ca_crt.exists(), "Root CA certificate must exist");
        assert!(ca_key.exists(), "Root CA key must exist");

        // Verify CA expiry and fingerprint
        let ca_expiry = get_cert_expiry(&ca_crt);
        assert!(ca_expiry.is_some(), "CA expiry should be readable");
        let ca_fp = get_cert_fingerprint(&ca_crt);
        assert!(ca_fp.is_some(), "CA fingerprint should be readable");

        // 2. Ensure Wildcard Leaf Cert
        let config = ensure_tls_cert(&test_dir).expect("ensure_tls_cert failed");
        assert!(config.cert_path.exists(), "Wildcard cert must exist");
        assert!(config.key_path.exists(), "Wildcard key must exist");
        assert!(is_cert_valid(&config.cert_path), "Wildcard cert must be valid");

        // Verify chain
        assert!(
            verify_cert_chain(&ca_crt, &config.cert_path),
            "Chain verification should succeed"
        );

        // Verify idempotency
        let mtime_ca = fs::metadata(&ca_crt).unwrap().modified().unwrap();
        let (ca_crt2, _) = ensure_ca_cert(&test_dir).expect("second ensure_ca_cert failed");
        assert_eq!(ca_crt, ca_crt2);
        let mtime_ca2 = fs::metadata(&ca_crt2).unwrap().modified().unwrap();
        assert_eq!(mtime_ca, mtime_ca2, "Idempotent call should not regenerate existing CA");

        // Clean up
        let _ = fs::remove_dir_all(&test_dir);
    }
}
