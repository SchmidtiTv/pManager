//! Command dispatch.
//!
//! Adding a command is three edits: a new file here implementing [`Command`],
//! one variant on [`Commands`], and one arm in the match below. Forgetting the
//! arm is a compile error, which is the point.

pub mod list;
pub mod reload;

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

    /// Re-read strip profiles from disk
    Reload(reload::Reload),
}

impl Command for Commands {
    fn run(&self, cx: &mut Cx) -> Result<()> {
        match self {
            Commands::List(c) => c.run(cx),
            Commands::Reload(c) => c.run(cx),
        }
    }
}
