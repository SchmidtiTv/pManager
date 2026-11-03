//! The `project` group. Same shape as the root: an enum plus a match.

pub mod add;
pub mod cleanup;
pub mod remove;
pub mod restore;
pub mod status;

use std::path::Path;

use clap::{Args, Subcommand};

use super::Command;
use crate::context::Cx;
use crate::database::sql_lite::SqlLite;
use crate::error::{NotRegistered, Result};
use crate::models::project;

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

    /// Drop registered or stripped rows whose folder no longer exists
    Cleanup(cleanup::Cleanup),

    /// Show a project's state and its next due action
    Status(status::Status),

    /// Unpack an archived project back to its original path
    Restore(restore::Restore),
}

impl Command for Project {
    fn run(&self, cx: &mut Cx) -> Result<()> {
        match &self.command {
            ProjectCommand::Add(c) => c.run(cx),
            ProjectCommand::Remove(c) => c.run(cx),
            ProjectCommand::Cleanup(c) => c.run(cx),
            ProjectCommand::Status(c) => c.run(cx),
            ProjectCommand::Restore(c) => c.run(cx),
        }
    }
}

/// `path` must already be normalised with `paths::normalise_registered_path`.
fn find_registered(db: &SqlLite, path: &Path) -> Result<project::Project> {
    db.query(&project::select_by_path(path)?, project::from_row)?
        .into_iter()
        .next()
        .ok_or_else(|| {
            NotRegistered {
                path: path.to_path_buf(),
            }
            .into()
        })
}
