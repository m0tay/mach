//! Menu command dispatcher for Norb.
//!
//! Launches the native Apple Silicon macOS menu bar status item (` Norb`).

use anyhow::Result;

/// Execute the macOS menu bar status item application.
pub async fn run() -> Result<()> {
    crate::menu::run_menu_bar()
}
