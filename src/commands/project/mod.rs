//! The `project` group. Same shape as the root: an enum plus a match.

pub mod add;
pub mod cleanup;
pub mod remove;

use clap::{Args, Subcommand};

use super::Command;
use crate::context::Cx;
use crate::error::Result;

#[derive(Args, Debug)]
pub struct Project {
    #[command(subcommand)]
    pub command: ProjectCommand,
}

#[derive(Subcommand, Debug)]
pub enum ProjectCommand {
    /// Register a project
    Add(add::Add),

    /// Unregister a project
    Remove(remove::Remove),

    /// Drop rows whose path no longer exists
    Cleanup(cleanup::Cleanup),
}

impl Command for Project {
    fn run(&self, cx: &mut Cx) -> Result<()> {
        match &self.command {
            ProjectCommand::Add(c) => c.run(cx),
            ProjectCommand::Remove(c) => c.run(cx),
            ProjectCommand::Cleanup(c) => c.run(cx),
        }
    }
}
