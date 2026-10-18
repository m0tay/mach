//! Cursor-stable, jitter-free terminal progress bar and downloader UX.
//!
//! Designed specifically to eliminate trail-cursor jitter and erratic jumping
//! in modern macOS GPU-accelerated terminal emulators (Ghostty, WezTerm, iTerm2, Alacritty, Warp).
//!
//! Key properties:
//! 1. Cursor is hidden on initialization (`\x1b[?25l`) and guaranteed restored (`\x1b[?25h`)
//!    via `Drop`, panic hooks, and `atexit` signal handlers.
//! 2. Rate-limited rendering throttled to 15-20 updates/sec (60ms minimum between renders).
//! 3. Clean line overwriting via `\r\x1b[2K`.
//! 4. Sleek Unicode block design (`▰▱`) with percentage, bytes transferred, speed, and ETA.

use std::io::{self, IsTerminal, Read, Write};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

static PANIC_HOOK_SET: AtomicBool = AtomicBool::new(false);

/// Ensure the terminal cursor is restored on panic or process exit.
fn ensure_cursor_guards() {
    if !PANIC_HOOK_SET.swap(true, Ordering::SeqCst) {
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            eprint!("\x1b[?25h");
            let _ = io::stderr().flush();
            prev_hook(info);
        }));

        // libc atexit hook to restore cursor even on std::process::exit
        extern "C" fn atexit_restore_cursor() {
            let _ = io::stdout().write_all(b"\x1b[?25h");
            let _ = io::stdout().flush();
        }
        unsafe {
            libc::atexit(atexit_restore_cursor);
        }
    }
}

/// Format bytes into human-readable representation.
pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;

    let b = bytes as f64;
    if b < KB {
        format!("{} B", bytes)
    } else if b < MB {
        format!("{:.1} KB", b / KB)
    } else if b < GB {
        let val = b / MB;
        if val >= 100.0 {
            format!("{:.0} MB", val)
        } else {
            format!("{:.1} MB", val)
        }
    } else {
        format!("{:.2} GB", b / GB)
    }
}

/// Format transfer speed (bytes/sec) into human-readable string.
pub fn format_speed(bytes_per_sec: f64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;

    if bytes_per_sec < KB {
        format!("{:.0} B/s", bytes_per_sec.max(0.0))
    } else if bytes_per_sec < MB {
        format!("{:.1} KB/s", bytes_per_sec / KB)
    } else if bytes_per_sec < GB {
        format!("{:.1} MB/s", bytes_per_sec / MB)
    } else {
        format!("{:.2} GB/s", bytes_per_sec / GB)
    }
}

/// A cursor-stable, rate-limited terminal progress bar.
#[allow(dead_code)]
pub struct ProgressBar {
    title: String,
    subtitle: Option<String>,
    total_bytes: Option<u64>,
    current_bytes: u64,
    start_time: Instant,
    last_render_time: Instant,
    last_render_bytes: u64,
    min_interval: Duration,
    bar_width: usize,
    is_tty: bool,
    finished: bool,
    smoothed_speed: f64,
    to_stderr: bool,
}

impl ProgressBar {
    /// Return the title of the item being downloaded.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Return the subtitle metadata if set.
    pub fn subtitle(&self) -> Option<&str> {
        self.subtitle.as_deref()
    }

    /// Create a new progress bar rendering to stdout.
    pub fn new(title: String, subtitle: Option<String>, total_bytes: Option<u64>) -> Self {
        Self::new_ext(title, subtitle, total_bytes, false)
    }

    /// Create a new progress bar, optionally directing output to stderr.
    ///
    /// - `title`: Name of the item being downloaded (e.g. "Debian 12 (Bookworm)")
    /// - `subtitle`: Optional network metadata (e.g. "Cloudflare CDN · 14ms")
    /// - `total_bytes`: Optional total file size in bytes
    /// - `to_stderr`: If true, render to stderr instead of stdout (for transparent command pipelines)
    pub fn new_ext(
        title: String,
        subtitle: Option<String>,
        total_bytes: Option<u64>,
        to_stderr: bool,
    ) -> Self {
        ensure_cursor_guards();
        let is_tty = if to_stderr {
            io::stderr().is_terminal()
        } else {
            io::stdout().is_terminal()
        };

        if is_tty {
            // Hide cursor immediately to prevent jitter
            if to_stderr {
                eprint!("\x1b[?25l");
                let _ = io::stderr().flush();
            } else {
                print!("\x1b[?25l");
                let _ = io::stdout().flush();
            }
        }

        // Print header line
        if let Some(ref sub) = subtitle {
            if to_stderr {
                eprintln!(
                    "  \x1b[1;36m➜\x1b[0m Downloading \x1b[1m{}\x1b[0m \x1b[90m[{}]\x1b[0m",
                    title, sub
                );
            } else {
                println!(
                    "  \x1b[1;36m➜\x1b[0m Downloading \x1b[1m{}\x1b[0m \x1b[90m[{}]\x1b[0m",
                    title, sub
                );
            }
        } else if to_stderr {
            eprintln!("  \x1b[1;36m➜\x1b[0m Downloading \x1b[1m{}\x1b[0m", title);
        } else {
            println!("  \x1b[1;36m➜\x1b[0m Downloading \x1b[1m{}\x1b[0m", title);
        }

        let now = Instant::now();
        let mut pb = Self {
            title,
            subtitle,
            total_bytes,
            current_bytes: 0,
            start_time: now,
            last_render_time: now,
            last_render_bytes: 0,
            min_interval: Duration::from_millis(60), // ~16 updates/sec max
            bar_width: 24,
            is_tty,
            finished: false,
            smoothed_speed: 0.0,
            to_stderr,
        };

        if pb.is_tty {
            pb.render(now);
        }

        pb
    }

    /// Set current transferred byte count.
    pub fn set_position(&mut self, bytes: u64) {
        if self.finished {
            return;
        }
        self.current_bytes = bytes;
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_render_time);

        // Throttle updates: only render if min_interval has elapsed or if completed
        let reached_end = self.total_bytes.map_or(false, |total| bytes >= total);
        if elapsed >= self.min_interval || reached_end {
            self.render(now);
        }
    }

    /// Increment current transferred bytes.
    pub fn inc(&mut self, delta: u64) {
        self.set_position(self.current_bytes + delta);
    }

    /// Render progress bar to stdout.
    fn render(&mut self, now: Instant) {
        if !self.is_tty {
            return;
        }

        let delta_time = now.duration_since(self.last_render_time).as_secs_f64();
        let delta_bytes = self.current_bytes.saturating_sub(self.last_render_bytes) as f64;
        let instant_speed = if delta_time > 0.001 {
            delta_bytes / delta_time
        } else {
            0.0
        };

        if self.smoothed_speed <= 0.0 {
            self.smoothed_speed = instant_speed;
        } else if delta_time > 0.001 {
            // Exponential smoothing for stable, readable speed display
            self.smoothed_speed = 0.7 * self.smoothed_speed + 0.3 * instant_speed;
        }

        let overall_elapsed = now.duration_since(self.start_time).as_secs_f64();
        if self.smoothed_speed <= 0.0 && overall_elapsed > 0.05 {
            self.smoothed_speed = self.current_bytes as f64 / overall_elapsed;
        }

        let speed_str = format_speed(self.smoothed_speed);

        if let Some(total) = self.total_bytes {
            let total = total.max(1);
            let fraction = (self.current_bytes as f64 / total as f64).clamp(0.0, 1.0);
            let percent = (fraction * 100.0) as u32;
            let filled_count = (fraction * self.bar_width as f64).round() as usize;
            let empty_count = self.bar_width.saturating_sub(filled_count);

            let filled_str = "▰".repeat(filled_count);
            let empty_str = "▱".repeat(empty_count);

            let cur_str = format_bytes(self.current_bytes);
            let tot_str = format_bytes(total);

            let eta_str = if self.smoothed_speed > 1024.0 && total > self.current_bytes {
                let remaining_bytes = (total - self.current_bytes) as f64;
                let eta_sec = remaining_bytes / self.smoothed_speed;
                if eta_sec < 60.0 {
                    format!("{:.0}s", eta_sec.max(0.0))
                } else {
                    format!("{}m {}s", (eta_sec / 60.0) as u64, (eta_sec % 60.0) as u64)
                }
            } else if percent >= 100 {
                "0s".to_string()
            } else {
                "--s".to_string()
            };

            if self.to_stderr {
                eprint!(
                    "\r\x1b[2K    \x1b[1;36m{}\x1b[0m\x1b[90m{}\x1b[0m  \x1b[1m{:3}%\x1b[0m  {:>6} / {:<6}  \x1b[2m({}, {})\x1b[0m",
                    filled_str, empty_str, percent, cur_str, tot_str, speed_str, eta_str
                );
            } else {
                print!(
                    "\r\x1b[2K    \x1b[1;36m{}\x1b[0m\x1b[90m{}\x1b[0m  \x1b[1m{:3}%\x1b[0m  {:>6} / {:<6}  \x1b[2m({}, {})\x1b[0m",
                    filled_str, empty_str, percent, cur_str, tot_str, speed_str, eta_str
                );
            }
        } else {
            // Indeterminate / pulse animation
            let pulse_pos = (overall_elapsed * 12.0) as usize % self.bar_width;
            let mut bar = String::with_capacity(self.bar_width * 3);
            for i in 0..self.bar_width {
                if (i >= pulse_pos && i < pulse_pos + 4)
                    || (pulse_pos + 4 > self.bar_width && i < (pulse_pos + 4) % self.bar_width)
                {
                    bar.push('▰');
                } else {
                    bar.push('▱');
                }
            }
            let cur_str = format_bytes(self.current_bytes);
            if self.to_stderr {
                eprint!(
                    "\r\x1b[2K    \x1b[1;36m{}\x1b[0m  {:>7}  \x1b[2m({})\x1b[0m",
                    bar, cur_str, speed_str
                );
            } else {
                print!(
                    "\r\x1b[2K    \x1b[1;36m{}\x1b[0m  {:>7}  \x1b[2m({})\x1b[0m",
                    bar, cur_str, speed_str
                );
            }
        }

        if self.to_stderr {
            let _ = io::stderr().flush();
        } else {
            let _ = io::stdout().flush();
        }
        self.last_render_time = now;
        self.last_render_bytes = self.current_bytes;
    }

    /// Clear the progress bar line, print clean final summary, and restore cursor.
    pub fn finish_with_message(&mut self, message: &str) {
        if self.finished {
            return;
        }
        self.finished = true;

        if self.is_tty {
            // Clear the active progress line completely
            if self.to_stderr {
                eprint!("\r\x1b[2K");
                eprintln!("  \x1b[1;32m✓\x1b[0m \x1b[1m{}\x1b[0m", message);
                eprint!("\x1b[?25h");
                let _ = io::stderr().flush();
            } else {
                print!("\r\x1b[2K");
                println!("  \x1b[1;32m✓\x1b[0m \x1b[1m{}\x1b[0m", message);
                print!("\x1b[?25h");
                let _ = io::stdout().flush();
            }
        } else if self.to_stderr {
            eprintln!("  ✓ {}", message);
        } else {
            println!("  ✓ {}", message);
        }
    }
}

impl Drop for ProgressBar {
    fn drop(&mut self) {
        if !self.finished {
            if self.is_tty {
                // Erase active progress line
                if self.to_stderr {
                    eprint!("\r\x1b[2K");
                    let _ = io::stderr().flush();
                } else {
                    print!("\r\x1b[2K");
                    let _ = io::stdout().flush();
                }
            }
            self.finished = true;
        }
        if self.is_tty {
            // GUARANTEE cursor is restored even on error, early exit, or panic
            if self.to_stderr {
                eprint!("\x1b[?25h");
                let _ = io::stderr().flush();
            } else {
                print!("\x1b[?25h");
                let _ = io::stdout().flush();
            }
        }
    }
}

/// Stream bytes from a reader to a writer with real-time rate-limited progress updates.
pub fn copy_with_progress<R: Read, W: Write>(
    reader: &mut R,
    writer: &mut W,
    pb: &mut ProgressBar,
) -> io::Result<u64> {
    let mut buf = [0u8; 64 * 1024]; // 64 KB chunks
    let mut total_copied = 0u64;

    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        writer.write_all(&buf[..n])?;
        total_copied += n as u64;
        pb.set_position(total_copied);
    }
    writer.flush()?;
    Ok(total_copied)
}

/// Metadata probe result for remote image URL.
pub struct RemoteMetadata {
    pub content_length: Option<u64>,
    pub provider_tag: String,
}

/// Probe remote image URL using curl to discover Content-Length, CDN name, and connect latency.
///
/// Throttled to 3 seconds max timeout to ensure zero latency overhead.
pub fn probe_remote_url(url: &str) -> RemoteMetadata {
    let output = Command::new("curl")
        .args([
            "-sI",
            "-L",
            "--max-time",
            "3",
            "-w",
            "\n__MACH_METRICS__\ntime_connect=%{time_connect}\n",
            url,
        ])
        .output();

    let Ok(out) = output else {
        return RemoteMetadata {
            content_length: None,
            provider_tag: "Cloud Image CDN".to_string(),
        };
    };

    let text = String::from_utf8_lossy(&out.stdout);

    // 1. Parse Content-Length (last instance across HTTP redirects)
    let mut last_content_len = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(val_str) = trimmed.strip_prefix("Content-Length:").or_else(|| trimmed.strip_prefix("content-length:")) {
            if let Ok(bytes) = val_str.trim().parse::<u64>() {
                // Ignore small redirect HTML pages (< 10 KB)
                if bytes > 10 * 1024 {
                    last_content_len = Some(bytes);
                }
            }
        }
    }

    // 2. Parse connect latency
    let mut latency_ms = None;
    if let Some(metrics_idx) = text.find("__MACH_METRICS__") {
        let metrics_str = &text[metrics_idx..];
        for line in metrics_str.lines() {
            if let Some(time_str) = line.trim().strip_prefix("time_connect=") {
                if let Ok(secs) = time_str.trim().parse::<f64>() {
                    let ms = (secs * 1000.0).round() as u64;
                    latency_ms = Some(ms.max(1));
                }
            }
        }
    }

    // 3. Detect CDN or Cloud infrastructure provider
    let lower = text.to_lowercase();
    let cdn_name = if lower.contains("cf-ray:") || lower.contains("server: cloudflare") {
        "Cloudflare CDN"
    } else if lower.contains("fastly") || lower.contains("x-served-by: cache-") {
        "Fastly CDN"
    } else if lower.contains("x-amz-cf-id") || lower.contains("cloudfront") {
        "CloudFront CDN"
    } else if lower.contains("akamai") || lower.contains("x-akamai") {
        "Akamai CDN"
    } else if lower.contains("canonical") {
        "Canonical CDN"
    } else if lower.contains("debian.org") || lower.contains("fedoraproject.org") {
        "Fast Mirror"
    } else {
        "Fast Mirror"
    };

    let provider_tag = match latency_ms {
        Some(ms) => format!("{cdn_name} · {ms}ms"),
        None => cdn_name.to_string(),
    };

    RemoteMetadata {
        content_length: last_content_len,
        provider_tag,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(format_bytes(142 * 1024 * 1024), "142 MB");
        assert_eq!(format_bytes(210 * 1024 * 1024), "210 MB");
        assert_eq!(format_bytes(2 * 1024 * 1024 * 1024), "2.00 GB");
    }

    #[test]
    fn test_format_speed() {
        assert_eq!(format_speed(0.0), "0 B/s");
        assert_eq!(format_speed(500.0), "500 B/s");
        assert_eq!(format_speed(150.0 * 1024.0), "150.0 KB/s");
        let mb_speed = 45.2 * 1024.0 * 1024.0;
        assert_eq!(format_speed(mb_speed), "45.2 MB/s");
    }

    #[test]
    fn test_copy_with_progress() {
        let input_data = vec![0x42u8; 128 * 1024]; // 128 KB
        let mut reader = std::io::Cursor::new(input_data.clone());
        let mut writer = Vec::new();

        let mut pb = ProgressBar::new(
            "Test Item".to_string(),
            Some("Local CDN · 1ms".to_string()),
            Some(input_data.len() as u64),
        );

        let copied = copy_with_progress(&mut reader, &mut writer, &mut pb).expect("copy failed");
        assert_eq!(copied, input_data.len() as u64);
        assert_eq!(writer, input_data);
        pb.finish_with_message("Test Item ready (128 KB)");
    }

    #[test]
    fn test_progress_bar_drop_safety() {
        // Drop before finish should cleanly restore state without panicking
        let pb = ProgressBar::new(
            "Drop Test".to_string(),
            None,
            Some(1000),
        );
        drop(pb);
    }
}

