//! `norb logs` command implementation.

use anyhow::{bail, Context, Result};
use norb_common::config::NorbConfig;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::time::Duration;
use tokio::time::sleep;

use super::{find_machine, load_all_machines};

pub async fn run(name: Option<&str>, follow: bool, lines: usize) -> Result<()> {
    let cfg = NorbConfig::load()?;
    let target_name = match name {
        Some(n) => n.to_string(),
        None => {
            if let Some(ref def) = cfg.default_machine {
                def.clone()
            } else {
                let machines = load_all_machines(&cfg.machines_dir)?;
                if machines.is_empty() {
                    bail!("no machines found");
                }
                machines[0].config.name.clone()
            }
        }
    };

    let machine = find_machine(&cfg.machines_dir, &target_name)?
        .with_context(|| format!("machine '{target_name}' not found"))?;

    let log_path = cfg.machines_dir.join(&machine.config.name).join("vm.log");
    if !log_path.exists() {
        println!("No log file found for machine '{}'.", machine.config.name);
        return Ok(());
    }

    let file = File::open(&log_path)?;
    let mut reader = BufReader::new(file);

    let all_lines: Vec<String> = reader.by_ref().lines().filter_map(|l| l.ok()).collect();
    let start_idx = if all_lines.len() > lines {
        all_lines.len() - lines
    } else {
        0
    };

    for l in &all_lines[start_idx..] {
        println!("{}", l);
    }

    if follow {
        let mut file = File::open(&log_path)?;
        file.seek(SeekFrom::End(0))?;
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        loop {
            line.clear();
            let bytes = reader.read_line(&mut line)?;
            if bytes > 0 {
                print!("{}", line);
            } else {
                sleep(Duration::from_millis(200)).await;
            }
        }
    }

    Ok(())
}
