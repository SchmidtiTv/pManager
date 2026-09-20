//! The `auto-run` group. clap derives the hyphenated name from `AutoRun`.

pub mod add;
pub mod remove;

use clap::{Args, Subcommand};

use super::Command;
use crate::context::Cx;
use crate::error::Result;

#[derive(Args, Debug)]
pub struct AutoRun {
    #[command(subcommand)]
    pub command: AutoRunCommand,
}

#[derive(Subcommand, Debug)]
pub enum AutoRunCommand {
    /// Install the scheduled background run
    Add(add::Add),

    /// Remove the scheduled background run
    Remove(remove::Remove),
}

impl Command for AutoRun {
    fn run(&self, cx: &mut Cx) -> Result<()> {
        match &self.command {
            AutoRunCommand::Add(c) => c.run(cx),
            AutoRunCommand::Remove(c) => c.run(cx),
        }
    }
}
