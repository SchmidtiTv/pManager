use anyhow::bail;
use clap::Args;

use crate::commands::Command;
use crate::context::Cx;
use crate::error::Result;

#[derive(Args, Debug)]
pub struct Add {}

impl Command for Add {
    fn run(&self, _cx: &mut Cx) -> Result<()> {
        bail!("`pman auto-run add` is not implemented yet")
    }
}
