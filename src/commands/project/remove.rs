use std::path::PathBuf;

use anyhow::bail;
use clap::Args;

use crate::commands::Command;
use crate::context::Cx;
use crate::error::Result;

#[derive(Args, Debug)]
pub struct Remove {
    /// Path of the project to unregister
    #[arg(value_name = "PATH")]
    pub path: PathBuf,
}

impl Command for Remove {
    fn run(&self, _cx: &mut Cx) -> Result<()> {
        bail!("`pman project remove` is not implemented yet")
    }
}
