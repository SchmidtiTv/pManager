use anyhow::bail;
use clap::Args;

use super::Command;
use crate::context::Cx;
use crate::error::Result;

#[derive(Args, Debug)]
pub struct List {}

impl Command for List {
    fn run(&self, _cx: &mut Cx) -> Result<()> {
        bail!("`pman list` is not implemented yet")
    }
}
