use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use chrono::Utc;
use clap::Args;

use super::find_registered;
use crate::archive;
use crate::commands::Command;
use crate::context::Cx;
use crate::error::Result;
use crate::formats::console_data::{Cell, ConsoleData, build_output};
use crate::models::project::{self, Project};
use crate::models::state::ProjectState;
use crate::paths;

#[derive(Args, Debug)]
pub struct Restore {
    /// Path the project was registered at
    #[arg(value_name = "PATH")]
    pub path: PathBuf,
}

impl Command for Restore {
    /// The `.pproject` is deleted last, after the ledger says `registered`:
    /// a crash or a failed delete leaves a redundant archive, never a lost
    /// project.
    fn run(&self, cx: &mut Cx) -> Result<()> {
        let path = paths::normalise_registered_path(&self.path)?;
        let db = cx.db()?;
        let project = find_registered(db, &path)?;

        let Some(archive_path) = project.state.archive_path() else {
            bail!(
                "`{}` is {}, not archived; there is nothing to restore",
                project.name,
                project.state.name()
            );
        };
        if exists(&path)? {
            bail!(
                "{} already exists; restore never overwrites a folder",
                path.display()
            );
        }
        if !archive_path.is_file() {
            bail!(
                "the archive of `{}` is missing at {}",
                project.name,
                archive_path.display()
            );
        }

        let staging = staging(&path);
        if exists(&staging)? {
            bail!(
                "{} is left over from an interrupted restore; remove it and try again",
                staging.display()
            );
        }

        archive::unpack(archive_path, &staging)?;
        let unpacked = archive::entry_count(&staging)
            .with_context(|| format!("could not walk {} after unpacking", staging.display()))?;
        archive::verify(archive_path, unpacked)?;

        std::fs::rename(&staging, &path).with_context(|| {
            format!(
                "could not move {} into place at {}",
                staging.display(),
                path.display()
            )
        })?;

        // Registered, not stripped: dependencies reinstalled after the restore
        // must be stripped again before the next archive.
        let restored = Project {
            state: ProjectState::Registered,
            last_change_at: Utc::now(),
            ..project.clone()
        };
        db.execute(&project::update(&restored)?)
            .with_context(|| format!("could not record the restore of `{}`", project.name))?;

        // The project is fully restored at this point, so a leftover archive
        // is a warning, not a failed restore.
        if let Err(err) = std::fs::remove_file(archive_path) {
            tracing::warn!(
                "restored `{}` but could not delete its archive {}; delete it by hand: {err}",
                project.name,
                archive_path.display()
            );
        }

        tracing::info!("restored `{}` to {}", project.name, path.display());

        let output = build_output(&console_data(&restored), cx.view)?;
        println!("{output}");
        Ok(())
    }
}

fn exists(path: &Path) -> Result<bool> {
    path.try_exists()
        .with_context(|| format!("could not check {}", path.display()))
}

fn staging(path: &Path) -> PathBuf {
    let mut name = OsString::from(path);
    name.push(".part");
    PathBuf::from(name)
}

const HEADERS: [&str; 3] = ["NAME", "STATE", "PATH"];

fn console_data(project: &Project) -> ConsoleData {
    ConsoleData {
        headers: HEADERS.map(str::to_string).to_vec(),
        rows: vec![vec![
            Cell::Text(project.name.clone()),
            Cell::Text(project.state.name().to_string()),
            Cell::Text(project.path.display().to_string()),
        ]],
    }
}
