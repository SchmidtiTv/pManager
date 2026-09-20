use anyhow::bail;
use clap::Args;

use super::Command;
use crate::context::Cx;
use crate::error::Result;

#[derive(Args, Debug)]
pub struct Run {
    /// Report what would happen without stripping or archiving anything
    #[arg(long)]
    pub dry_run: bool,
}

impl Command for Run {
    fn run(&self, _cx: &mut Cx) -> Result<()> {
        bail!("`pman run` is not implemented yet")
    }
}
