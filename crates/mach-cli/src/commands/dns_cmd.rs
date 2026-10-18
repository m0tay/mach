//! DNS command interface for Mach.

use anyhow::Result;
use std::time::Duration;

use crate::dns;

pub fn enable() -> Result<()> {
    setup()
}

pub fn disable() -> Result<()> {
    println!("\x1b[1;36m➜\x1b[0m Disabling Mach local DNS (*.mach.local)...");
    match dns::remove_resolver() {
        Ok(_) => {
            println!("  \x1b[1;32m✓\x1b[0m Successfully removed /etc/resolver/mach.local.");
        }
        Err(e) => {
            println!("  \x1b[1;33m!\x1b[0m Could not remove /etc/resolver/mach.local: {e}");
        }
    }
    Ok(())
}

pub fn setup() -> Result<()> {
    println!("\x1b[1;36m➜\x1b[0m Enabling Mach local DNS (*.mach.local)...");

    let (already_configured, hint) = dns::check_or_setup_resolver();
    if already_configured {
        println!("  \x1b[1;32m✓\x1b[0m macOS resolver is already configured at /etc/resolver/mach.local.");
    } else {
        println!("  \x1b[1;36m➜\x1b[0m Configuring /etc/resolver/mach.local (may prompt for sudo)...");
        match dns::install_resolver() {
            Ok(_) => {
                println!("  \x1b[1;32m✓\x1b[0m Successfully configured /etc/resolver/mach.local.");
            }
            Err(e) => {
                println!("  \x1b[1;33m!\x1b[0m Could not automatically configure /etc/resolver/mach.local: {e}");
                println!("    Please run the following command in your terminal:\n");
                if let Some(cmd) = hint {
                    println!("      \x1b[1;32m{}\x1b[0m\n", cmd);
                }
            }
        }
    }

    // Verify UDP responder on 127.0.0.1:5354
    let responding = dns::probe_dns_responder(Duration::from_millis(300));
    if responding {
        println!("  \x1b[1;32m✓\x1b[0m DNS responder is active and responding on 127.0.0.1:5354.");
    } else {
        println!("  \x1b[1;33m!\x1b[0m DNS responder on 127.0.0.1:5354 is not currently running.");
        println!("    It will start automatically when a machine boots, or run \x1b[1;34mmach daemon start\x1b[0m.");
    }

    println!("\n  macOS routes all *.mach.local domains to your Mach machines & containers.");
    Ok(())
}

pub fn status() -> Result<()> {
    println!("\x1b[1mMach DNS Status\x1b[0m");
    println!("───────────────────────────────────────────────────");

    // 1. macOS Resolver (/etc/resolver/mach.local)
    let (configured, hint) = dns::check_or_setup_resolver();
    if configured {
        println!("  \x1b[1;32m●\x1b[0m macOS DNS Resolver:     \x1b[1;32mConfigured\x1b[0m (/etc/resolver/mach.local)");
    } else {
        println!("  \x1b[1;33m○\x1b[0m macOS DNS Resolver:     \x1b[1;33mNot Configured\x1b[0m (/etc/resolver/mach.local)");
        if let Some(cmd) = hint {
            println!("    \x1b[2mRun:\x1b[0m \x1b[1;34m{}\x1b[0m", cmd);
        }
    }

    // 2. DNS Responder UDP (127.0.0.1:5354)
    let responding = dns::probe_dns_responder(Duration::from_millis(300));
    if responding {
        println!("  \x1b[1;32m●\x1b[0m DNS Responder (UDP):    \x1b[1;32mActive & Responding\x1b[0m (127.0.0.1:5354)");
    } else {
        println!("  \x1b[1;33m○\x1b[0m DNS Responder (UDP):    \x1b[1;33mNot Responding\x1b[0m (127.0.0.1:5354)");
        println!("    \x1b[2mStarts automatically when a VM starts, or run:\x1b[0m \x1b[1;34mmach daemon start\x1b[0m");
    }

    println!("───────────────────────────────────────────────────");
    if configured && responding {
        println!("\x1b[1;32m✓ Local DNS (*.mach.local) is fully operational.\x1b[0m\n");
    } else if !configured {
        println!("Run \x1b[1;34mmachctl dns enable\x1b[0m to configure the macOS resolver.\n");
    }

    Ok(())
}
