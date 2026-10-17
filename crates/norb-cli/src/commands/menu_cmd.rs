//! Menu command dispatcher.

use anyhow::Result;

pub async fn run() -> Result<()> {
    crate::menu::run_menu_bar()
}
