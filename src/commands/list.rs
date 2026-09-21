use chrono::{DateTime, Utc};
use clap::Args;

use super::Command;
use crate::context::Cx;
use crate::error::Result;
use crate::formats::console_data::{Cell, ConsoleData, build_output};
use crate::models::project::{self, Project};

#[derive(Args, Debug)]
pub struct List {}

impl Command for List {
    fn run(&self, cx: &mut Cx) -> Result<()> {
        let projects = cx.db()?.query(&project::select_all()?, project::from_row)?;

        if projects.is_empty() {
            // Not data: stderr, so `pman list` stays pipeable when empty.
            tracing::info!("no projects registered");
        }

        let output = build_output(&console_data(&projects, Utc::now()), cx.view)?;

        if !output.is_empty() {
            println!("{output}");
        }

        Ok(())
    }
}

const HEADERS: [&str; 5] = ["NAME", "KIND", "STATE", "IDLE", "PATH"];

fn console_data(projects: &[Project], now: DateTime<Utc>) -> ConsoleData {
    ConsoleData {
        headers: HEADERS.map(str::to_string).to_vec(),
        rows: projects.iter().map(|p| row(p, now)).collect(),
    }
}

fn row(project: &Project, now: DateTime<Utc>) -> Vec<Cell> {
    vec![
        Cell::Text(project.name.clone()),
        Cell::Text(project.kind.clone()),
        Cell::Text(project.state.name().to_string()),
        Cell::Days((now - project.last_change_at).num_days()),
        Cell::Text(project.path.display().to_string()),
    ]
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::{TimeZone as _, Utc};

    use super::console_data;
    use crate::formats::console_data::Cell;
    use crate::models::project::Project;
    use crate::models::state::ProjectState;

    fn project(name: &str, path: &str, state: ProjectState) -> Project {
        Project {
            name: name.to_string(),
            path: PathBuf::from(path),
            kind: "rust".to_string(),
            state,
            has_git_repo: true,
            last_change_at: Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap(),
            registered_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        }
    }

    #[test]
    fn idle_days_are_measured_from_the_last_change() {
        let now = Utc.with_ymd_and_hms(2026, 6, 11, 0, 0, 0).unwrap();
        let data = console_data(&[project("a", "C:/a", ProjectState::Registered)], now);

        assert_eq!(data.rows[0][3], Cell::Days(10));
    }

    #[test]
    fn every_project_becomes_one_row_matching_the_headers() {
        let now = Utc.with_ymd_and_hms(2026, 6, 11, 0, 0, 0).unwrap();
        let data = console_data(
            &[
                project("a", "C:/a", ProjectState::Registered),
                project("b", "C:/b", ProjectState::Stripped),
            ],
            now,
        );

        assert_eq!(data.rows.len(), 2);
        assert!(data.rows.iter().all(|row| row.len() == data.headers.len()));
    }
}
