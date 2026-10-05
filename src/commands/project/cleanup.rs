use std::io::ErrorKind;
use std::path::Path;

use anyhow::{Context as _, bail};
use clap::Args;

use crate::commands::Command;
use crate::context::Cx;
use crate::database::sql_lite::SqlLite;
use crate::error::{self, Result};
use crate::formats::console_data::{Cell, ConsoleData, build_output};
use crate::models::project::{self, Project};

#[derive(Args, Debug)]
pub struct Cleanup {}

impl Command for Cleanup {
    fn run(&self, cx: &mut Cx) -> Result<()> {
        let db = cx.db()?;
        let projects = db.query(&project::select_all()?, project::from_row)?;

        let mut rows = Vec::with_capacity(projects.len());
        let mut failed: Vec<String> = Vec::new();

        for project in &projects {
            let result = clean(project, db).unwrap_or_else(|err| {
                error::report(&err.context(format!("could not clean up `{}`", project.name)));
                failed.push(project.name.clone());
                Outcome::Failed
            });
            rows.push((project, result));
        }

        let removed = rows.iter().filter(|(_, r)| *r == Outcome::Removed).count();
        if projects.is_empty() {
            tracing::info!("no projects registered");
        } else if !failed.is_empty() {
            tracing::info!(
                "removed {removed} stale registration(s); {} could not be cleaned up",
                failed.len()
            );
        } else if removed == 0 {
            tracing::info!("no stale registrations; {} remain", projects.len());
        } else {
            tracing::info!(
                "removed {removed} stale registration(s); {} remain",
                projects.len() - removed
            );
        }

        let output = build_output(&console_data(&rows), cx.view)?;
        if !output.is_empty() {
            println!("{output}");
        }

        if !failed.is_empty() {
            bail!(
                "{} of {} projects could not be cleaned up: {}",
                failed.len(),
                projects.len(),
                failed.join(", ")
            );
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Removed,
    Kept,
    Failed,
}

impl Outcome {
    fn name(self) -> &'static str {
        match self {
            Self::Removed => "removed",
            Self::Kept => "kept",
            Self::Failed => "failed",
        }
    }
}

fn clean(project: &Project, db: &SqlLite) -> Result<Outcome> {
    if project.state.archive_path().is_some() || !is_gone(&project.path)? {
        return Ok(Outcome::Kept);
    }

    match db.execute(&project::delete_active_by_path(&project.path)?)? {
        1 => Ok(Outcome::Removed),
        changed => bail!("expected to remove one row, removed {changed}"),
    }
}

fn is_gone(path: &Path) -> Result<bool> {
    match std::fs::metadata(path) {
        Ok(_) => Ok(false),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(true),
        Err(err) => Err(err).with_context(|| format!("could not check {}", path.display())),
    }
}

const HEADERS: [&str; 4] = ["NAME", "STATE", "PATH", "RESULT"];

fn console_data(rows: &[(&Project, Outcome)]) -> ConsoleData {
    ConsoleData {
        headers: HEADERS.map(str::to_string).to_vec(),
        rows: rows
            .iter()
            .map(|(project, outcome)| {
                vec![
                    Cell::Text(project.name.clone()),
                    Cell::Text(project.state.name().to_string()),
                    Cell::Text(project.path.display().to_string()),
                    Cell::Text(outcome.name().to_string()),
                ]
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::is_gone;

    #[test]
    fn an_existing_folder_is_not_gone() {
        let tmp = tempfile::tempdir().unwrap();

        assert!(!is_gone(tmp.path()).unwrap());
    }

    #[test]
    fn a_missing_folder_is_gone() {
        let tmp = tempfile::tempdir().unwrap();

        assert!(is_gone(&tmp.path().join("nope")).unwrap());
    }

    #[test]
    fn a_file_where_the_folder_was_is_not_gone() {
        // Not a project any more, but not proof of anything either: removing
        // the row is for paths that are plainly gone.
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("file.txt");
        std::fs::write(&file, "").unwrap();

        assert!(!is_gone(&file).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_that_cannot_be_checked_is_an_error_not_gone() {
        use std::os::unix::fs::PermissionsExt as _;

        let tmp = tempfile::tempdir().unwrap();
        let locked = tmp.path().join("locked");
        std::fs::create_dir_all(locked.join("inner")).unwrap();
        let original = std::fs::metadata(&locked).unwrap().permissions();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();

        let result = is_gone(&locked.join("inner"));
        std::fs::set_permissions(&locked, original).unwrap();

        let err = result.unwrap_err();
        assert!(format!("{err:#}").contains("inner"), "got: {err:#}");
    }
}
