//! Diagnostic logging setup.
//!
//! Logs go to stderr so that stdout stays clean for machine-readable output;
//! `vmanager folder list > folders.txt` should never pick up a log line.

use crate::cli::GlobalArgs;

use tracing_subscriber::EnvFilter;

/// Environment variable that overrides the `-v`/`-q` derived filter.
const FILTER_ENV: &str = "RUST_LOG";

/// Install the global tracing subscriber.
///
/// Called once from `main`; a second call is a no-op rather than a panic so
/// that tests and embedders can be relaxed about ordering.
pub fn init(globals: &GlobalArgs) {
    let filter = match EnvFilter::try_from_env(FILTER_ENV) {
        Ok(filter) => filter,
        Err(_) => EnvFilter::new(globals.log_level().to_string()),
    };

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(globals.color.should_colorize())
        .with_writer(std::io::stderr)
        .without_time()
        .with_target(globals.verbose >= 2)
        .try_init();
}
