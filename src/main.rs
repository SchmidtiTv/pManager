//! Binary entry point.
//!
//! Parsing, logging setup and exit-code handling live here; everything else is
//! in the `vmanager` library crate.

use std::process::ExitCode;

use clap::Parser;
use vmanager::{Cli, Error, error::exit, logging};

fn main() -> ExitCode {
    // clap prints its own diagnostics and exits with 2 on a usage error.
    let cli = Cli::parse();

    logging::init(&cli.globals);

    match vmanager::run(cli) {
        Ok(()) => ExitCode::from(exit::SUCCESS),
        // `vmanager folder list | head -1` closes stdout early; that is the
        // pipeline working as intended, not a failure worth reporting.
        Err(Error::Io(err)) if err.kind() == std::io::ErrorKind::BrokenPipe => {
            ExitCode::from(exit::SUCCESS)
        }
        Err(err) => err.report(),
    }
}
