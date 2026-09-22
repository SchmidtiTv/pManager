use std::io::IsTerminal as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use chrono::Utc;
use clap::Args;
use dialoguer::Input;

use crate::activity;
use crate::commands::Command;
use crate::context::Cx;
use crate::error::Result;
use crate::git;
use crate::models::project::{self, Project};
use crate::models::state::ProjectState;
use crate::paths;

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
    fn run(&self, cx: &mut Cx) -> Result<()> {
        let path = paths::normalise_project_path(&self.path)?;
        let name = match &self.name {
            Some(name) => checked_name(name)?,
            None => prompt_for_name(&path)?,
        };

        let db = cx.db()?;
        let clashes = db.query(
            &project::select_by_name_or_path(&name, &path)?,
            project::from_row,
        )?;
        if let Some(existing) = clashes.iter().find(|p| p.path == path) {
            bail!(
                "{} is already registered as `{}`",
                path.display(),
                existing.name
            );
        }
        if let Some(existing) = clashes.first() {
            bail!(
                "the name `{name}` is already used by {}",
                existing.path.display()
            );
        }

        let project = Project {
            has_git_repo: git::has_repo(&path),
            last_change_at: activity::last_change(&path)?,
            registered_at: Utc::now(),
            state: ProjectState::Registered,
            name,
            path,
        };
        db.execute(&project::insert(&project)?)?;

        tracing::info!(
            "registered `{}` at {}",
            project.name,
            project.path.display()
        );
        Ok(())
    }
}

fn checked_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        bail!("the project name must not be empty");
    }

    Ok(name.to_string())
}

fn prompt_for_name(path: &Path) -> Result<String> {
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        bail!("no terminal to ask for a project name; pass --name");
    }

    let mut input = Input::<String>::new().with_prompt("Project name");
    if let Some(folder) = path.file_name().and_then(|name| name.to_str()) {
        input = input.default(folder.to_string());
    }

    let name = input
        .validate_with(|name: &String| checked_name(name).map(drop).map_err(|e| e.to_string()))
        .interact_text()
        .context("could not read the project name")?;

    checked_name(&name)
}
