use clap::Args;

use crate::commands::Command;
use crate::context::Cx;
use crate::error::Result;
use crate::scheduler::{self, Removed};

#[derive(Args, Debug)]
pub struct Remove {}

impl Command for Remove {
    fn run(&self, _cx: &mut Cx) -> Result<()> {
        let scheduler = scheduler::for_this_platform()?;

        match scheduler.remove()? {
            Removed::Removed => tracing::info!("removed {}", scheduler.describe()),
            Removed::NotInstalled => tracing::info!("no scheduled run is installed"),
        }
        Ok(())
    }
}
