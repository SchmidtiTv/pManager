//! pManager — ages out dormant local projects to reclaim disk space.

pub mod cli;
pub mod context;
pub mod error;
pub mod paths;

use std::process::ExitCode;

use clap::Parser;

/// Process entry point. Never returns an error; it reports and converts.
pub fn run() -> ExitCode {
    match try_run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            error::report(&err);
            ExitCode::FAILURE
        }
    }
}

fn try_run() -> error::Result {
    let cli = cli::Cli::parse();
    cli::init_tracing(cli.global.verbose, cli.global.quiet);
    Ok(())
}
