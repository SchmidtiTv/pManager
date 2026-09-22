use std::path::PathBuf;

use anyhow::bail;
use clap::Args;

use crate::commands::Command;
use crate::context::Cx;
use crate::error::Result;
use crate::models::project;
use crate::paths;

#[derive(Args, Debug)]
pub struct Remove {
    /// Path of the project to unregister
    #[arg(value_name = "PATH")]
    pub path: PathBuf,
}

impl Command for Remove {
    /// Only the ledger row goes. pManager never deletes a project's files,
    /// or its archive, on the way out.
    fn run(&self, cx: &mut Cx) -> Result<()> {
        let path = paths::normalise_registered_path(&self.path)?;

        let db = cx.db()?;
        let Some(project) = db
            .query(&project::select_by_path(&path)?, project::from_row)?
            .into_iter()
            .next()
        else {
            bail!("no project is registered at {}", path.display());
        };

        db.execute(&project::delete_by_path(&path)?)?;

        if let Some(archive_path) = project.state.archive_path() {
            tracing::warn!(
                "`{}` was archived; its archive {} is left in place",
                project.name,
                archive_path.display()
            );
        }
        tracing::info!(
            "unregistered `{}` at {}; its files were not touched",
            project.name,
            path.display()
        );
        Ok(())
    }
}
