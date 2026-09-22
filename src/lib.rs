//! pManager — ages out dormant local projects to reclaim disk space.

pub mod activity;
pub mod archive;
pub mod cli;
pub mod commands;
pub mod context;
pub mod database;
pub mod disk;
pub mod error;
pub mod formats;
pub mod git;
pub mod models;
pub mod pass;
pub mod paths;
pub mod scheduler;
pub mod stripping;

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
    let mut cx = context::Cx::new(paths, cli.global.output);

    cli.command.run(&mut cx)
}
