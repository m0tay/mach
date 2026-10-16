//! DNS command interface for Norb.

use anyhow::Result;
use crate::dns;

pub fn setup() -> Result<()> {
    let (configured, hint) = dns::check_or_setup_resolver();
    if configured {
        println!("\x1b[1;32m✓\x1b[0m macOS resolver is already configured at /etc/resolver/norb.local.");
        println!("  DNS responder listens on 127.0.0.1:5354.");
    } else {
        println!("\x1b[1;33m!\x1b[0m Configuring macOS resolver requires root privileges.");
        println!("  Run the following command in your terminal:\n");
        if let Some(cmd) = hint {
            println!("    \x1b[1;32m{}\x1b[0m\n", cmd);
        }
        println!("  Once configured, macOS will automatically route all *.norb.local domains to your Norb machines.");
    }
    Ok(())
}

pub fn status() -> Result<()> {
    let (configured, _) = dns::check_or_setup_resolver();
    if configured {
        println!("\x1b[1;32m●\x1b[0m macOS DNS Resolver: \x1b[1;32mConfigured\x1b[0m (/etc/resolver/norb.local)");
    } else {
        println!("\x1b[1;33m○\x1b[0m macOS DNS Resolver: \x1b[1;33mNot Configured\x1b[0m");
        println!("  Run \x1b[1;34mnorbctl dns setup\x1b[0m for configuration instructions.");
    }
    Ok(())
}
