//! `pmanager list` — every tracked project on one screen.
//!
//! Built by surveying the disk, so the numbers are true rather than
//! remembered; the database is consulted only for what the disk cannot say.

use std::io::Write;

use crate::archive::human_bytes;
use crate::commands::Context;
use crate::error::Result;
use crate::git;
use crate::lifecycle::{self, State, Survey};
use crate::scan;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Show only projects in this state
    #[arg(long, value_name = "STATE", value_parser = ["active", "trimmed", "archived"])]
    state: Option<String>,

    /// Show only projects with uncommitted changes
    #[arg(short, long)]
    dirty: bool,

    /// Show only projects with unpushed commits
    #[arg(short, long)]
    unpushed: bool,

    /// Show only projects past the trim threshold
    #[arg(short, long)]
    stale: bool,
}

/// One project, surveyed.
struct Row {
    name: String,
    path: std::path::PathBuf,
    state: State,
    edited: String,
    committed: String,
    notes: Vec<String>,
    survey: Survey,
    stale: bool,
}

pub fn run(args: &Args, ctx: &Context) -> Result<()> {
    let db = ctx.open_database()?;
    let config = ctx.config()?;
    let thresholds = config.thresholds()?;
    let skip = config.skip_dirs();

    let mut out = std::io::stdout().lock();
    let projects = db.projects()?;

    if projects.is_empty() {
        if !ctx.is_quiet() {
            writeln!(out, "no projects tracked yet")?;
            writeln!(out, "track one with `pmanager project add <PROJECT>`")?;
        }
        return Ok(());
    }

    let mut rows = Vec::new();
    let mut freed_total = 0;

    for record in &projects {
        let survey = lifecycle::survey(&record.path, &skip, &config.purge_dirs(&record.path));
        freed_total += record.freed_bytes;

        // An archived project has no files to date, so use when it was archived.
        let edited = match (record.state, &survey.last_edit) {
            (State::Archived, _) => record
                .archived_at
                .map_or_else(|| "—".into(), |at| scan::time_ago(scan::from_epoch(at))),
            (_, Some(edit)) => scan::time_ago(edit.at),
            (_, None) => "—".to_string(),
        };

        let committed = survey
            .git
            .head
            .as_ref()
            .map_or_else(|| "—".to_string(), |head| scan::time_ago(head.at));

        // Measured from the same basis `run` uses, restore floor included, so
        // the listing never marks a project stale that the run will not touch.
        let stale = survey.last_edit.as_ref().is_some_and(|edit| {
            let idle_from = record
                .active_since()
                .map_or(edit.at, |seen| edit.at.max(seen));
            scan::older_than(idle_from, thresholds.trim_days)
        });

        rows.push(Row {
            name: record.name.clone(),
            path: record.path.clone(),
            state: record.state,
            edited,
            committed,
            notes: notes(record, &survey, stale, thresholds.trim_days),
            survey,
            stale,
        });
    }

    let shown: Vec<&Row> = rows.iter().filter(|row| args.shows(row)).collect();

    if shown.is_empty() {
        writeln!(out, "no projects match")?;
        return Ok(());
    }

    let name_width = shown
        .iter()
        .map(|row| row.name.chars().count())
        .max()
        .unwrap_or(4)
        .max(4);

    writeln!(
        out,
        "{:<name_width$}  {:<11}{:<13}{:<10}NOTES",
        "NAME", "EDITED", "COMMITTED", "STATE"
    )?;

    for row in &shown {
        writeln!(
            out,
            "{:<name_width$}  {:<11}{:<13}{:<10}{}",
            row.name,
            row.edited,
            row.committed,
            row.state,
            row.notes.join(", ")
        )?;
    }

    summarize(&mut out, &rows, shown.len(), freed_total)?;
    Ok(())
}

impl Args {
    /// Whether the filters let a row through. With none given, all of them do.
    fn shows(&self, row: &Row) -> bool {
        if let Some(state) = &self.state
            && row.state.as_str() != state
        {
            return false;
        }
        if self.dirty && !row.survey.git.is_dirty() {
            return false;
        }
        if self.unpushed && !row.survey.git.has_unpushed() {
            return false;
        }
        if self.stale && !row.stale {
            return false;
        }
        true
    }
}

/// The clauses at the end of a row.
fn notes(
    record: &crate::db::ProjectRecord,
    survey: &Survey,
    stale: bool,
    trim_days: u32,
) -> Vec<String> {
    let mut notes = Vec::new();

    match record.state {
        State::Archived => {
            if let Some(archive) = &record.archive {
                notes.push(human_bytes(archive.bytes));
                if !archive.path.is_file() {
                    // The archive *is* the project now.
                    notes.push("ARCHIVE MISSING".to_string());
                }
            }
        }
        _ if !survey.present => notes.push("folder missing".to_string()),
        _ => {
            if survey.purgeable_bytes() > 0 {
                notes.push(format!(
                    "{} to reclaim",
                    human_bytes(survey.purgeable_bytes())
                ));
            }
            if stale {
                notes.push(format!("stale >{trim_days}d"));
            }
        }
    }

    let git: &git::Status = &survey.git;
    if git.is_dirty() {
        notes.push(format!("{} uncommitted", git.change_count()));
    }
    if git.has_unpushed() {
        notes.push(format!("{} unpushed", git.ahead));
    }

    if notes.is_empty() {
        notes.push("—".to_string());
    }
    notes
}

/// The counts under the table.
fn summarize(
    out: &mut impl Write,
    rows: &[Row],
    shown: usize,
    freed_total: u64,
) -> std::io::Result<()> {
    let count = |state: State| rows.iter().filter(|row| row.state == state).count();
    let reclaimable: u64 = rows
        .iter()
        .filter(|row| row.state != State::Archived)
        .map(|row| row.survey.purgeable_bytes())
        .sum();

    writeln!(out)?;
    writeln!(
        out,
        "{} project(s){}: {} active, {} trimmed, {} archived",
        rows.len(),
        if shown == rows.len() {
            String::new()
        } else {
            format!(" ({shown} shown)")
        },
        count(State::Active),
        count(State::Trimmed),
        count(State::Archived)
    )?;

    if reclaimable > 0 {
        writeln!(
            out,
            "{} sitting in dependency trees",
            human_bytes(reclaimable)
        )?;
    }
    if freed_total > 0 {
        writeln!(out, "{} reclaimed so far", human_bytes(freed_total))?;
    }

    // Only worth saying while there is something to point at.
    if rows
        .iter()
        .any(|row| !row.path.is_dir() && row.state != State::Archived)
    {
        writeln!(
            out,
            "some folders are missing; `pmanager project prune` clears them out"
        )?;
    }

    Ok(())
}
