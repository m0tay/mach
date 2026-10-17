//! `mach update` / `machctl update` command implementation.

use anyhow::Result;
use std::process::Command;

pub async fn run() -> Result<()> {
    println!("\x1b[1;36m➜\x1b[0m Checking for Mach updates...");
    println!("  Current version: \x1b[1mv{}\x1b[0m", env!("CARGO_PKG_VERSION"));

    // Check if brew is installed and manages mach
    let brew_check = Command::new("brew")
        .args(["list", "mach"])
        .output();

    if let Ok(out) = brew_check {
        if out.status.success() {
            println!("  Installation:    Homebrew formula (\x1b[1;32mdouglaslobo/mach/mach\x1b[0m)");
            println!("  \x1b[1;36m➜\x1b[0m Updating Homebrew formulas...");
            let _ = Command::new("brew")
                .args(["update"])
                .status();

            println!("  \x1b[1;36m➜\x1b[0m Upgrading Mach to latest version...");
            let res = Command::new("brew")
                .args(["upgrade", "mach"])
                .status();

            match res {
                Ok(status) if status.success() => {
                    println!("\x1b[1;32m✓\x1b[0m Mach successfully upgraded to the latest version!");
                }
                _ => {
                    println!("\x1b[1;32m✓\x1b[0m Mach is already up to date.");
                }
            }
            return Ok(());
        }
    }

    println!("\nMach is installed standalone. To update via Homebrew, run:");
    println!("  \x1b[1;32mbrew upgrade mach\x1b[0m");
    println!("Or from repository source:\n  \x1b[1;32m./install.sh\x1b[0m");

    Ok(())
}
