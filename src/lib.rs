//! pManager — ages out dormant local projects to reclaim disk space.

pub mod cli;
pub mod commands;
pub mod context;
pub mod database;
pub mod error;
pub mod paths;
pub mod scheduler;

use std::process::ExitCode;

use clap::Parser;

use crate::commands::Command as _;

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

    let paths = paths::Paths::resolve(cli.global.db.clone(), cli.global.strip_dir.clone())?;
    let mut cx = context::Cx::new(paths);

    cli.command.run(&mut cx)
}
