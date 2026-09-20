use anyhow::bail;
use clap::Args;

use super::Command;
use crate::context::Cx;
use crate::error::Result;

#[derive(Args, Debug)]
pub struct Restore {
    /// Name of the archived project to restore
    #[arg(value_name = "NAME")]
    pub name: String,
}

impl Command for Restore {
    fn run(&self, _cx: &mut Cx) -> Result<()> {
        bail!("`pman restore` is not implemented yet")
    }
}
