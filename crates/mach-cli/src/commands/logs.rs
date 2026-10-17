//! `mach logs` command implementation.

use anyhow::{bail, Context, Result};
use mach_common::config::MachConfig;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::time::Duration;
use tokio::time::sleep;

use super::{find_machine, load_all_machines};

const CHUNK_SIZE: usize = 64 * 1024;

/// Locate the byte offset corresponding to the start of the last `lines` lines
/// in `reader` using backward chunk seeks in O(1) memory.
pub fn find_start_offset<R: Read + Seek>(reader: &mut R, lines: usize) -> std::io::Result<u64> {
    if lines == 0 {
        return reader.seek(SeekFrom::End(0));
    }
    let file_len = reader.seek(SeekFrom::End(0))?;
    if file_len == 0 {
        return Ok(0);
    }

    let mut buffer = [0u8; CHUNK_SIZE];
    let mut cursor = file_len;
    let mut newline_count = 0;

    while cursor > 0 && newline_count < lines {
        let block_size = std::cmp::min(cursor, CHUNK_SIZE as u64) as usize;
        let read_start = cursor - block_size as u64;
        reader.seek(SeekFrom::Start(read_start))?;
        reader.read_exact(&mut buffer[..block_size])?;

        for i in (0..block_size).rev() {
            let file_pos = read_start + i as u64;
            // Skip a trailing newline at the very end of the file
            if file_pos == file_len - 1 && buffer[i] == b'\n' {
                continue;
            }
            if buffer[i] == b'\n' {
                newline_count += 1;
                if newline_count == lines {
                    return Ok(file_pos + 1);
                }
            }
        }
        cursor = read_start;
    }

    // Fewer than `lines` lines exist; start from beginning of file
    Ok(0)
}

pub async fn run(name: Option<&str>, follow: bool, lines: usize) -> Result<()> {
    let cfg = MachConfig::load()?;
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

    let mut file = File::open(&log_path)?;
    let start_offset = find_start_offset(&mut file, lines)?;
    file.seek(SeekFrom::Start(start_offset))?;

    let mut reader = BufReader::new(file);
    let mut line = String::new();

    loop {
        line.clear();
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 {
            break;
        }
        print!("{}", line);
    }

    if follow {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_empty_log() {
        let mut cursor = Cursor::new(b"");
        assert_eq!(find_start_offset(&mut cursor, 10).unwrap(), 0);
    }

    #[test]
    fn test_zero_lines() {
        let mut cursor = Cursor::new(b"line 1\nline 2\n");
        assert_eq!(find_start_offset(&mut cursor, 0).unwrap(), 14);
    }

    #[test]
    fn test_trailing_newline() {
        let content = b"alpha\nbeta\ngamma\n";
        let mut cursor = Cursor::new(content);
        // last 1 line: "gamma\n" starts at offset 11
        assert_eq!(find_start_offset(&mut cursor, 1).unwrap(), 11);
        // last 2 lines: "beta\ngamma\n" starts at offset 6
        assert_eq!(find_start_offset(&mut cursor, 2).unwrap(), 6);
        // last 3 lines: "alpha\nbeta\ngamma\n" starts at offset 0
        assert_eq!(find_start_offset(&mut cursor, 3).unwrap(), 0);
        // more lines than present: starts at offset 0
        assert_eq!(find_start_offset(&mut cursor, 10).unwrap(), 0);
    }

    #[test]
    fn test_no_trailing_newline() {
        let content = b"one\ntwo\nthree";
        let mut cursor = Cursor::new(content);
        // last 1 line: "three" starts at offset 8
        assert_eq!(find_start_offset(&mut cursor, 1).unwrap(), 8);
        // last 2 lines: "two\nthree" starts at offset 4
        assert_eq!(find_start_offset(&mut cursor, 2).unwrap(), 4);
        // last 3 lines: starts at offset 0
        assert_eq!(find_start_offset(&mut cursor, 3).unwrap(), 0);
    }

    #[test]
    fn test_large_log_chunk_boundary() {
        // Create > 128KB of logs to test crossing 64KB chunk boundaries
        let mut data = Vec::new();
        for i in 0..3000 {
            data.extend_from_slice(format!("log entry #{:05}\n", i).as_bytes());
        }
        let mut cursor = Cursor::new(&data);
        let offset = find_start_offset(&mut cursor, 5).unwrap();
        let tail_slice = &data[offset as usize..];
        let tail_str = std::str::from_utf8(tail_slice).unwrap();
        let lines: Vec<&str> = tail_str.lines().collect();
        assert_eq!(lines.len(), 5);
        assert_eq!(lines[0], "log entry #02995");
        assert_eq!(lines[4], "log entry #02999");
    }
}
