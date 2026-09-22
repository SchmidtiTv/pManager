use clap::Args;

use super::Command;
use crate::context::Cx;
use crate::error::Result;
use crate::pass;

#[derive(Args, Debug)]
pub struct Run {
    /// Report what would happen without stripping or archiving anything
    #[arg(long)]
    pub dry_run: bool,
}

impl Command for Run {
    fn run(&self, cx: &mut Cx) -> Result<()> {
        pass::execute_pass(cx, self.dry_run)
    }
}
