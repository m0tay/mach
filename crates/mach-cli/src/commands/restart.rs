//! `norb restart` command implementation.

use anyhow::Result;
use std::time::Duration;
use tokio::time::sleep;

pub async fn run(name: Option<&str>) -> Result<()> {
    println!("\x1b[1;36m➜\x1b[0m Restarting machine...");
    let _ = super::stop::run(name, false, true).await;
    sleep(Duration::from_millis(1000)).await;
    super::start::run(name).await?;
    Ok(())
}
