use anyhow::bail;
use clap::Args;

use crate::commands::Command;
use crate::context::Cx;
use crate::error::Result;

#[derive(Args, Debug)]
pub struct Cleanup {}

impl Command for Cleanup {
    fn run(&self, _cx: &mut Cx) -> Result<()> {
        bail!("`pman project cleanup` is not implemented yet")
    }
}
