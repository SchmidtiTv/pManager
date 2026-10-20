//! The root parser, its global flags, and log initialisation.

use std::path::{Path, PathBuf};

use clap::{ArgAction, Args, CommandFactory as _, FromArgMatches as _, Parser};
use tracing_subscriber::EnvFilter;

use crate::commands::Commands;
use crate::formats::console_data::ViewType;

#[derive(Parser, Debug)]
#[command(
    name = "pman",
    bin_name = "pman",
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

/// Parse argv. The binary ships as both `pman` and `pmanager`, so the usage
/// line names the one that was invoked, without the `.exe` that clap would
/// take verbatim from argv[0] on Windows.
pub fn parse() -> Cli {
    let mut command = Cli::command();
    if let Some(invoked) = std::env::args_os().next().and_then(|arg0| {
        Path::new(&arg0)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
    }) {
        command = command.bin_name(invoked);
    }

    let matches = command.get_matches_mut();
    Cli::from_arg_matches(&matches).unwrap_or_else(|err| err.format(&mut command).exit())
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

    /// Render command data as an aligned table or as JSON
    #[arg(short, long, global = true, value_enum, default_value_t = ViewType::Table)]
    pub output: ViewType,
}

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

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
