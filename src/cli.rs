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
///
/// The filter always carries a global `error` floor, so that a `RUST_LOG`
/// which scopes down some unrelated dependency (e.g. `RUST_LOG=git2=debug`)
/// can never silence pManager's own error reporting -- that reporting is the
/// CLI's only channel for telling the user why it failed.
///
/// Note this floor cannot be provided by `EnvFilter::builder`'s
/// `with_default_directive`: that default is only applied when the parsed
/// directive string is *entirely* empty. `RUST_LOG=git2=debug` parses to one
/// non-empty (but narrowly scoped) directive, so the default is skipped and
/// every other target -- including pManager's own -- is left with no
/// matching directive at all, which `EnvFilter` treats as disabled. Instead,
/// an explicit `error` directive is always prepended to whatever `RUST_LOG`
/// contains; a later, more permissive directive for the same (global) target
/// still wins, because `EnvFilter` keeps only the most recently added
/// directive per target.
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

    let directives = match std::env::var("RUST_LOG") {
        Ok(rust_log) => format!("error,{rust_log}"),
        Err(_) => fallback.to_string(),
    };

    let filter = EnvFilter::builder()
        .with_default_directive(tracing::level_filters::LevelFilter::ERROR.into())
        .parse_lossy(directives);

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
