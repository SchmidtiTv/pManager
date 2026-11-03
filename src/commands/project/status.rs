use std::path::PathBuf;

use chrono::{DateTime, Local, NaiveDate, Utc};
use clap::Args;

use super::find_registered;
use crate::commands::Command;
use crate::context::Cx;
use crate::error::Result;
use crate::formats::console_data::{Cell, ConsoleData, build_output};
use crate::models::project::Project;
use crate::pass::facts::{self, Facts};
use crate::pass::plan::{self, Next};
use crate::paths;
use crate::stripping;

#[derive(Args, Debug)]
pub struct Status {
    /// Path the project was registered at; it may already be archived
    #[arg(value_name = "PATH")]
    pub path: PathBuf,
}

impl Command for Status {
    fn run(&self, cx: &mut Cx) -> Result<()> {
        let path = paths::normalise_registered_path(&self.path)?;
        let project = find_registered(cx.db()?, &path)?;
        let profiles = stripping::load(&cx.paths.strip_dir)?;

        let facts = facts::gather(&project, &profiles)?;
        let now = Utc::now();
        let next = plan::next_action(&project, facts.as_ref(), now);

        let output = build_output(&console_data(&project, facts.as_ref(), next, now), cx.view)?;
        println!("{output}");
        Ok(())
    }
}

const HEADERS: [&str; 9] = [
    "NAME",
    "STATE",
    "LAST CHANGE AT",
    "IDLE",
    "DIRTY",
    "NEXT ACTION",
    "NEXT ACTION AT",
    "REASON",
    "PATH",
];

fn console_data(
    project: &Project,
    facts: Option<&Facts>,
    next: Next,
    now: DateTime<Utc>,
) -> ConsoleData {
    let last_change_at = facts.map_or(project.last_change_at, |facts| facts.last_change_at);
    let (action, at, reason) = match next {
        Next::Due { action, at } => (action.name(), Cell::Date(local_date(at)), Cell::Empty),
        Next::Nothing { reason } => ("none", Cell::Empty, Cell::Text(reason.name().to_string())),
    };

    ConsoleData {
        headers: HEADERS.map(str::to_string).to_vec(),
        rows: vec![vec![
            Cell::Text(project.name.clone()),
            Cell::Text(project.state.name().to_string()),
            Cell::Date(local_date(last_change_at)),
            Cell::Days((now - last_change_at).num_days()),
            facts.map_or(Cell::Empty, |facts| Cell::Bool(facts.dirty)),
            Cell::Text(action.to_string()),
            at,
            reason,
            Cell::Text(project.path.display().to_string()),
        ]],
    }
}

fn local_date(at: DateTime<Utc>) -> NaiveDate {
    at.with_timezone(&Local).date_naive()
}
