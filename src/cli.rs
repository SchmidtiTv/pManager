//! The root parser, its global flags, and log initialisation.

use std::path::PathBuf;

use clap::{ArgAction, Args, Parser};
use tracing_subscriber::EnvFilter;

use crate::commands::Commands;

#[derive(Parser, Debug)]
#[command(
    name = "pman",
    version,
    about = "Ages out dormant local projects to reclaim disk space",
    propagate_version = true
)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Args, Debug)]
pub struct GlobalArgs {
    /// Increase log verbosity (-v debug, -vv trace)
    #[arg(short, long, global = true, action = ArgAction::Count)]
    pub verbose: u8,

    /// Log errors only
    #[arg(short, long, global = true, conflicts_with = "verbose")]
    pub quiet: bool,

    /// Override the database path
    #[arg(long, global = true, value_name = "PATH")]
    pub db: Option<PathBuf>,

    /// Override the strip-profile directory
    #[arg(long, global = true, value_name = "PATH", env = "PMANAGER_STRIP_DIR")]
    pub strip_dir: Option<PathBuf>,
}

/// Install the log subscriber. `RUST_LOG` wins over the flags when set.
///
/// Writes to stderr so that command output on stdout stays pipeable, and uses
/// `try_init` so repeated calls inside one test process are harmless.
pub fn init_tracing(verbose: u8, quiet: bool) {
    let fallback = if quiet {
        "error"
    } else {
        match verbose {
            0 => "info",
            1 => "debug",
            _ => "trace",
        }
    };

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(fallback));

    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_target(false)
        .with_env_filter(filter)
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::CommandFactory;

    /// clap's own validator for malformed derive setups: conflicting argument
    /// names, duplicate shorts, bad defaults. Rerun as every command is added.
    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
