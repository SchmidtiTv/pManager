//! One `pman run`: look at every registered project, decide what is due, and
//! carry it out.
//!
//! The pass splits into a read half and a write half with a plain value in
//! between:
//!
//! ```text
//! stripping::load ──┐
//! project::select_all ──┤
//!                       ▼
//!   facts::gather   reads disk and git, mutates nothing
//!                       ▼
//!   plan::decide    pure: no I/O, no clock
//!                       ▼
//!   execute::apply  the only code that deletes -- skipped by --dry-run
//!                       ▼
//!   report::console_data → build_output → stdout
//! ```
//!
//! `--dry-run` is the absence of one step. The flag is read in
//! `commands/run.rs`, travels no further than [`execute_pass`], and nothing
//! below it takes a mode parameter.

pub mod execute;
pub mod facts;
pub mod plan;
pub mod report;

use anyhow::bail;
use chrono::{DateTime, Utc};

use crate::context::Cx;
use crate::error::{self, Result};
use crate::formats::console_data::build_output;
use crate::models::project::{self, Project};
use crate::pass::facts::Facts;
use crate::pass::report::{Outcome, Planned};
use crate::stripping;

pub fn execute_pass(cx: &mut Cx, dry_run: bool) -> Result<()> {
    // An invalid profile fails the whole pass before any project is touched.
    // Carrying on with no profiles would silently archive projects unstripped
    // and produce `.pproject` files many times larger than they should be,
    // with no signal that anything was wrong.
    let profiles = stripping::load(&cx.paths.strip_dir)?;

    let db = cx.db()?;
    let projects = db.query(&project::select_all()?, project::from_row)?;
    let now = Utc::now();

    let mut planned: Vec<Planned> = Vec::with_capacity(projects.len());
    let mut failed: Vec<String> = Vec::new();

    for project in &projects {
        // One project failing must not stop the pass: `pman run` usually runs
        // unattended, and a folder locked by an editor would otherwise block
        // every project after it. A failure leaves that row untouched, so the
        // next pass retries from a known state.
        let mut note = |err: anyhow::Error| {
            error::report(&err);
            failed.push(project.name.clone());
        };

        // Reading the project can fail on its own, before there is an action
        // to name. The row still has to exist, so it carries no action and
        // the idle days the ledger last knew about.
        let facts = match facts::gather(project, &profiles) {
            Ok(facts) => facts,
            Err(err) => {
                note(err.context(format!("could not examine `{}`", project.name)));
                planned.push(Planned {
                    name: project.name.clone(),
                    idle_days: idle_days(project, None, now),
                    action: plan::Action::Nothing {
                        reason: plan::Reason::Missing,
                    },
                    outcome: Outcome::Failed,
                });
                continue;
            }
        };

        let action = plan::decide(project, facts.as_ref(), now);

        // `--dry-run` is the absence of the next step, and nothing below
        // takes a mode parameter.
        let outcome = match facts {
            Some(ref facts) if !action.is_nothing() && !dry_run => {
                match execute::apply(project, &action, facts, db) {
                    Ok(freed) => Outcome::Done { freed },
                    Err(err) => {
                        note(err);
                        Outcome::Failed
                    }
                }
            }
            _ => Outcome::Planned,
        };

        planned.push(Planned {
            name: project.name.clone(),
            idle_days: idle_days(project, facts.as_ref(), now),
            action,
            outcome,
        });
    }

    if projects.is_empty() {
        tracing::info!("no projects registered");
    }

    let output = build_output(&report::console_data(&planned), cx.view)?;
    if !output.is_empty() {
        println!("{output}");
    }

    // Nothing is swallowed and no failure is downgraded to a warning: the
    // table above is printed first so the report is complete either way.
    if !failed.is_empty() {
        bail!(
            "{} of {} projects failed: {}",
            failed.len(),
            projects.len(),
            failed.join(", ")
        );
    }

    Ok(())
}

/// Idle days come from what was measured on disk, falling back to the ledger
/// for a project whose folder is gone -- the stored value is the last thing
/// anyone knew about it, and an empty column would say less.
fn idle_days(project: &Project, facts: Option<&Facts>, now: DateTime<Utc>) -> i64 {
    let last_change = facts.map_or(project.last_change_at, |facts| facts.last_change_at);

    (now - last_change).num_days()
}
