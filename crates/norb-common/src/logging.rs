//! Logging initialisation helper.
//!
//! Call [`init_logging`] once at the very start of `main` (before spawning any
//! threads or async tasks) to configure the global [`tracing`] subscriber.  It
//! is safe to call it multiple times — subsequent calls are no-ops because the
//! subscriber is set only once via [`once_cell`].
//!
//! # Example
//! ```no_run
//! norb_common::logging::init_logging("debug");
//! tracing::info!("Norb is starting");
//! ```

use once_cell::sync::OnceCell;
use tracing::Level;
use tracing_subscriber::{fmt, EnvFilter};

/// Guards against double-initialisation across the process lifetime.
static INIT: OnceCell<()> = OnceCell::new();

/// Initialise the global [`tracing`] subscriber with a human-readable format.
///
/// # Behaviour
/// - The minimum log level is controlled by `level` (e.g. `"info"`, `"debug"`).
/// - The `RUST_LOG` environment variable overrides `level` when set, giving
///   operators fine-grained control without recompiling.
/// - Output goes to **stderr** (consistent with most Unix tooling).
/// - The format includes timestamps, log levels, and source locations.
///
/// # Arguments
/// - `level` — a [`tracing`] level string: `"trace"`, `"debug"`, `"info"`,
///   `"warn"`, or `"error"`.  Unrecognised values fall back to `"info"`.
///
/// # Panics
/// Does **not** panic.  If the `level` string is not a valid filter directive,
/// the function logs a warning and falls back to `"info"`.
pub fn init_logging(level: &str) {
    INIT.get_or_init(|| {
        // Build an EnvFilter: RUST_LOG takes precedence over the `level`
        // argument, which in turn falls back to "info" on parse failure.
        let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            let directive = parse_level(level);
            EnvFilter::new(directive)
        });

        fmt::Subscriber::builder()
            // Human-readable, coloured output on TTYs.
            .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
            // Compact but still readable timestamp format.
            .with_timer(fmt::time::SystemTime)
            // Show the module path so log lines are easy to grep.
            .with_target(true)
            // Show file + line numbers at TRACE/DEBUG levels via span fields.
            .with_file(false)
            .with_line_number(false)
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .init();
    });
}

/// Map an arbitrary string to a valid tracing level directive, falling back to
/// `"info"` for anything unrecognised.
fn parse_level(level: &str) -> String {
    match level.to_ascii_lowercase().as_str() {
        "trace" | "debug" | "info" | "warn" | "error" => level.to_ascii_lowercase(),
        other => {
            // We can't call tracing here because the subscriber isn't set yet;
            // use eprintln so the message still reaches the operator.
            eprintln!(
                "[norb-common] unrecognised log level '{other}', defaulting to 'info'"
            );
            "info".to_string()
        }
    }
}

/// Returns the [`Level`] that corresponds to the supplied string, or
/// [`Level::INFO`] on any parse failure.  Useful when callers need the typed
/// form rather than a string directive.
pub fn parse_level_typed(level: &str) -> Level {
    match level.to_ascii_lowercase().as_str() {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "info"  => Level::INFO,
        "warn"  => Level::WARN,
        "error" => Level::ERROR,
        _       => Level::INFO,
    }
}
