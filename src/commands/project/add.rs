use std::path::PathBuf;

use anyhow::bail;
use clap::Args;

use crate::commands::Command;
use crate::context::Cx;
use crate::error::Result;

#[derive(Args, Debug)]
pub struct Add {
    /// Path to the project directory
    #[arg(value_name = "PATH")]
    pub path: PathBuf,

    /// Project name; prompted for interactively when omitted
    #[arg(long, value_name = "NAME")]
    pub name: Option<String>,
}

impl Command for Add {
    fn run(&self, _cx: &mut Cx) -> Result<()> {
        bail!("`pman project add` is not implemented yet")
    }
}
