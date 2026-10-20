use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use chrono::{DateTime, Utc};
use clap::Args;

use super::Command;
use crate::activity;
use crate::context::Cx;
use crate::database::schema;
use crate::database::sql_lite::SqlLite;
use crate::error::Result;
use crate::formats::console_data::{Cell, ConsoleData, build_output};
use crate::git;
use crate::models::project::{self, Project};
use crate::models::state::ProjectState;
use crate::models::v1_project::{self, V1Project, V1State};
use crate::pass::execute::check_archive_name;
use crate::paths;

#[derive(Args, Debug)]
pub struct MigrateV1 {
    /// The v1 database; on Windows `%APPDATA%\pmanager\data\pmanager.db`
    #[arg(value_name = "V1_DB")]
    pub from: PathBuf,

    /// Show what would be imported without writing anything
    #[arg(long)]
    pub dry_run: bool,
}

impl Command for MigrateV1 {
    /// The v1 database is only ever read. Every problem is collected before
    /// anything is written, so the import happens completely or not at all.
    fn run(&self, cx: &mut Cx) -> Result<()> {
        let (rows, pins) = read_v1(&self.from)?;

        // Opening the v2 ledger applies its schema, which would break a v1
        // database. On Linux both default to the same file.
        ensure_distinct_files(&self.from, &cx.paths.db)?;

        let mut problems = Vec::new();
        let projects: Vec<Project> = rows
            .iter()
            .filter_map(|row| {
                convert(row)
                    .with_context(|| format!("`{}`", row.name))
                    .map_err(|err| problems.push(format!("{err:#}")))
                    .ok()
            })
            .collect();

        let db = cx.db()?;
        let existing = db.query(&project::select_all()?, project::from_row)?;
        problems.extend(clashes(&projects, &existing));

        if !problems.is_empty() {
            bail!(
                "nothing was imported; fix these in v1 first:\n  {}",
                problems.join("\n  ")
            );
        }

        if pins > 0 {
            tracing::warn!("{pins} vManager version pin(s) are not carried over; v2 has no pins");
        }

        if self.dry_run {
            tracing::info!("dry run: {} project(s) would be imported", projects.len());
        } else {
            import(&projects, db)?;
            tracing::info!(
                "imported {} project(s) from {}",
                projects.len(),
                self.from.display()
            );
        }

        let output = build_output(&console_data(&projects), cx.view)?;
        if !output.is_empty() {
            println!("{output}");
        }

        Ok(())
    }
}

fn read_v1(path: &Path) -> Result<(Vec<V1Project>, i64)> {
    let v1 = SqlLite::open_read_only(path)?;

    let version = schema::user_version(&v1)?;
    if version != v1_project::SCHEMA_VERSION {
        bail!(
            "{} has schema version {version}, but the last pManager v1 schema is version {}",
            path.display(),
            v1_project::SCHEMA_VERSION
        );
    }

    let rows = v1.query(&v1_project::select_all()?, v1_project::from_row)?;
    let pins = v1
        .query(&v1_project::count_pins(), |row| row.get(0))?
        .first()
        .copied()
        .context("counting the v1 version pins returned no row")?;

    Ok((rows, pins))
}

fn ensure_distinct_files(v1: &Path, v2: &Path) -> Result<()> {
    if !v2.exists() {
        return Ok(());
    }

    let canonical = |path: &Path| {
        std::fs::canonicalize(path).with_context(|| format!("could not resolve {}", path.display()))
    };
    if canonical(v1)? == canonical(v2)? {
        bail!(
            "{} is both the v1 database and the v2 ledger; point --db at a separate file",
            v1.display()
        );
    }

    Ok(())
}

fn convert(row: &V1Project) -> Result<Project> {
    let registered_at = from_epoch(row.added_at)?;

    match row.state {
        V1State::Active | V1State::Trimmed => {
            check_archive_name(&row.name)?;
            let path = paths::normalise_project_path(&row.path)
                .context("its folder is gone; remove it in v1 or restore the folder")?;

            Ok(Project {
                name: row.name.clone(),
                has_git_repo: git::has_repo(&path),
                last_change_at: activity::last_change(&path)?,
                registered_at,
                state: if row.state == V1State::Active {
                    ProjectState::Registered
                } else {
                    ProjectState::Stripped
                },
                path,
            })
        }
        V1State::Archived => {
            let archive_path = row
                .archive_path
                .clone()
                .context("archived in v1 but has no archive path")?;
            let archived_at = row
                .archived_at
                .context("archived in v1 but has no archive date")?;

            Ok(Project {
                name: row.name.clone(),
                path: paths::normalise_registered_path(&row.path)?,
                state: ProjectState::Archived { archive_path },
                // Its folder is packed away, so this cannot be checked. Nothing
                // reads it for an archived project.
                has_git_repo: false,
                last_change_at: from_epoch(archived_at)?,
                registered_at,
            })
        }
    }
}

fn from_epoch(seconds: i64) -> Result<DateTime<Utc>> {
    DateTime::from_timestamp(seconds, 0)
        .with_context(|| format!("{seconds} is not a valid unix timestamp"))
}

fn clashes(projects: &[Project], existing: &[Project]) -> Vec<String> {
    let mut problems = Vec::new();

    for (index, project) in projects.iter().enumerate() {
        for earlier in &projects[..index] {
            if earlier.name == project.name {
                problems.push(format!(
                    "`{}` names both {} and {}",
                    project.name,
                    earlier.path.display(),
                    project.path.display()
                ));
            }
            if earlier.path == project.path {
                problems.push(format!(
                    "{} is registered as both `{}` and `{}`",
                    project.path.display(),
                    earlier.name,
                    project.name
                ));
            }
        }

        for registered in existing {
            if registered.name == project.name || registered.path == project.path {
                problems.push(format!(
                    "`{}` at {} clashes with `{}` at {} in the v2 ledger",
                    project.name,
                    project.path.display(),
                    registered.name,
                    registered.path.display()
                ));
            }
        }
    }

    problems
}

fn import(projects: &[Project], db: &SqlLite) -> Result<()> {
    db.execute_batch("BEGIN;")?;
    for project in projects {
        db.execute(&project::insert(project)?)?;
    }
    db.execute_batch("COMMIT;")
}

const HEADERS: [&str; 3] = ["NAME", "STATE", "PATH"];

fn console_data(projects: &[Project]) -> ConsoleData {
    ConsoleData {
        headers: HEADERS.map(str::to_string).to_vec(),
        rows: projects
            .iter()
            .map(|project| {
                vec![
                    Cell::Text(project.name.clone()),
                    Cell::Text(project.state.name().to_string()),
                    Cell::Text(project.path.display().to_string()),
                ]
            })
            .collect(),
    }
}
