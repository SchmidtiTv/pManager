//! Command dispatch.
//!
//! Adding a command is three edits: a new file here implementing [`Command`],
//! one variant on [`Commands`], and one arm in the match below. Forgetting the
//! arm is a compile error, which is the point.

pub mod list;
pub mod project;
pub mod reload;
pub mod restore;
pub mod run;

use clap::Subcommand;

use crate::context::Cx;
use crate::error::Result;

/// Every leaf command implements this. Group modules implement it too, by
/// delegating to their own inner enum.
pub trait Command {
    fn run(&self, cx: &mut Cx) -> Result<()>;
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// List all registered projects
    List(list::List),

    /// Manage project registrations
    Project(project::Project),

    /// Re-read strip profiles from disk
    Reload(reload::Reload),

    /// Evaluate every project and execute what is due
    Run(run::Run),

    /// Restore an archived project
    Restore(restore::Restore),
}

impl Command for Commands {
    fn run(&self, cx: &mut Cx) -> Result<()> {
        match self {
            Commands::List(c) => c.run(cx),
            Commands::Project(c) => c.run(cx),
            Commands::Reload(c) => c.run(cx),
            Commands::Run(c) => c.run(cx),
            Commands::Restore(c) => c.run(cx),
        }
    }
}
