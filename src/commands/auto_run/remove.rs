use anyhow::bail;
use clap::Args;

use crate::commands::Command;
use crate::context::Cx;
use crate::error::Result;

#[derive(Args, Debug)]
pub struct Remove {}

impl Command for Remove {
    fn run(&self, _cx: &mut Cx) -> Result<()> {
        bail!("`pman auto-run remove` is not implemented yet")
    }
}
