use anyhow::bail;
use clap::Args;

use super::Command;
use crate::context::Cx;
use crate::error::Result;

#[derive(Args, Debug)]
pub struct Reload {}

impl Command for Reload {
    fn run(&self, _cx: &mut Cx) -> Result<()> {
        bail!("`pman reload` is not implemented yet")
    }
}
