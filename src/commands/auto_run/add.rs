use anyhow::Context as _;
use clap::Args;

use crate::commands::Command;
use crate::context::Cx;
use crate::error::Result;
use crate::scheduler::{self, HOUR, Installed, Job, MINUTE};
use crate::stripping;

#[derive(Args, Debug)]
pub struct Add {}

impl Command for Add {
    /// Everything that can be checked here is checked before the scheduler is
    /// touched: a job that would fail every day at noon, unattended, is worse
    /// than an error now.
    fn run(&self, cx: &mut Cx) -> Result<()> {
        let job = Job::for_run(&cx.paths)?;
        stripping::load(&cx.paths.strip_dir).with_context(|| {
            format!(
                "the scheduled run could not load its strip profiles from {}",
                cx.paths.strip_dir.display()
            )
        })?;
        if !cx.paths.db.exists() {
            tracing::warn!(
                "there is no ledger at {} yet; the scheduled run will start an empty one",
                cx.paths.db.display()
            );
        }

        let scheduler = scheduler::for_this_platform()?;
        let verb = match scheduler.install(&job)? {
            Installed::New => "installed",
            Installed::Replaced => "updated",
            Installed::Unchanged => "already installed",
        };

        tracing::info!(
            "{verb}: daily `pman run` at {HOUR:02}:{MINUTE:02} via {}",
            scheduler.describe()
        );
        tracing::info!("it runs: {}", job.command_line());
        Ok(())
    }
}
