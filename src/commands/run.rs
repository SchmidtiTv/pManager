//! `pmanager run` — act on the projects that have aged out.
//!
//! Nothing is deleted without `--yes`; run bare, this only reports what it
//! would do. `pmanager schedule install` registers `run --yes`.
//!
//! One project failing does not stop the others. Failures are collected and
//! turned into a non-zero exit code, which is what makes a failed scheduled run
//! visible in Task Scheduler's history.

use std::io::Write;
use std::path::PathBuf;

use crate::archive::{self, human_bytes};
use crate::commands::Context;
use crate::db::{Database, ProjectRecord};
use crate::error::{Error, Result};
use crate::json;
use crate::lifecycle::{self, Action, Policy, Survey, Thresholds};
use crate::progress::Spinner;
use crate::scan;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Carry out the plan. Without it, nothing is deleted or written
    #[arg(short, long)]
    yes: bool,

    /// Act on this project only; repeatable [default: every tracked project]
    #[arg(short, long, value_name = "PROJECT")]
    project: Vec<String>,

    /// Days of no edits before dependencies are deleted [default: 60]
    #[arg(long, value_name = "DAYS")]
    trim_days: Option<u32>,

    /// Days of no edits before the project is archived [default: 180]
    #[arg(long, value_name = "DAYS")]
    archive_days: Option<u32>,

    /// Days of no edits after which unfinished work stops holding a project
    /// back from being archived [default: 240]
    #[arg(long, value_name = "DAYS")]
    force_archive_days: Option<u32>,

    /// Only trim; never archive
    #[arg(long, conflicts_with = "no_trim")]
    no_archive: bool,

    /// Only archive; never trim
    #[arg(long, conflicts_with = "no_archive")]
    no_trim: bool,

    /// Archive projects that hold uncommitted or unpushed work
    #[arg(short, long)]
    force: bool,

    /// Write the archives but leave the project folders in place
    #[arg(long)]
    keep_source: bool,

    /// Write archives here instead of beside each project
    #[arg(long, value_name = "DIR")]
    archive_dir: Option<PathBuf>,

    /// Show every project, including the ones nothing happens to
    #[arg(short, long)]
    all: bool,

    /// Compression level, 0 (fastest) to 9 (smallest)
    #[arg(short, long, value_parser = clap::value_parser!(u32).range(archive::MIN_LEVEL as i64..=archive::MAX_LEVEL as i64))]
    level: Option<u32>,

    /// Compression threads; more threads are faster and compress slightly worse
    #[arg(short, long, value_parser = clap::value_parser!(u32).range(1..=256))]
    threads: Option<u32>,
}

/// What one sweep did, as `--json` reports it. Collected as the sweep goes and
/// emitted at the end: a JSON document cannot be streamed a project at a time
/// the way the printed lines are.
#[derive(Debug, Default)]
struct Collected {
    projects: Vec<json::RunProject>,
}

impl Collected {
    /// Record one project's outcome. `changed` says whether anything actually
    /// happened, which a plan and an excluded project both answer no to.
    fn push(&mut self, record: &ProjectRecord, survey: &Survey, action: &Action, line: &Outcome) {
        self.projects.push(json::RunProject {
            name: record.name.clone(),
            path: json::path(&record.path),
            action: json::NextRun::new(action, line.description.clone()),
            changed: line.changed,
            last_edit: survey
                .last_edit
                .as_ref()
                .map(|edit| json::Time::at(edit.at)),
            freed_bytes: line.freed,
            archive: line.archive.clone(),
            error: line.error.clone(),
        });
    }
}

/// What became of one project: the sentence the printed output uses, plus what
/// a caller would want to branch on.
#[derive(Debug, Default)]
struct Outcome {
    description: String,
    changed: bool,
    freed: u64,
    archive: Option<json::Archive>,
    error: Option<String>,
}

/// Running totals across one sweep.
#[derive(Debug, Default)]
struct Totals {
    /// Projects whose dependency trees were deleted.
    trimmed: u64,
    /// Projects compressed and removed.
    archived: u64,
    /// Projects marked live again after being worked on.
    revived: u64,
    /// Changes described but not made, because `--yes` was absent.
    planned: u64,
    /// Old enough to archive, but holding unfinished work.
    held: u64,
    /// Tracked, but the folder is gone.
    missing: u64,
    /// Nothing to do.
    untouched: u64,
    failed: u64,
    freed: u64,
}

impl Totals {
    fn as_json(&self) -> json::RunTotals {
        json::RunTotals {
            trimmed: self.trimmed,
            archived: self.archived,
            revived: self.revived,
            planned: self.planned,
            held_back: self.held,
            missing: self.missing,
            untouched: self.untouched,
            failed: self.failed,
            freed_bytes: self.freed,
        }
    }
}

pub fn run(args: &Args, ctx: &Context) -> Result<()> {
    let config = ctx.config()?;
    let db = ctx.open_database()?;

    // Resolved and validated before anything is touched, so a bad number is an
    // error rather than a surprising sweep.
    let configured = config.thresholds()?;
    let thresholds = Thresholds {
        trim_days: args.trim_days.unwrap_or(configured.trim_days),
        archive_days: args.archive_days.unwrap_or(configured.archive_days),
        force_archive_days: args
            .force_archive_days
            .unwrap_or(configured.force_archive_days),
    };
    if thresholds.archive_days < thresholds.trim_days {
        return Err(Error::Other(anyhow::anyhow!(
            "--archive-days ({}) must be at least --trim-days ({})",
            thresholds.archive_days,
            thresholds.trim_days
        )));
    }
    if thresholds.force_archive_days < thresholds.archive_days {
        return Err(Error::Other(anyhow::anyhow!(
            "--force-archive-days ({}) must be at least --archive-days ({})",
            thresholds.force_archive_days,
            thresholds.archive_days
        )));
    }

    let archive_dir = args
        .archive_dir
        .clone()
        .or_else(|| config.archive_dir.clone());
    let policy = Policy {
        thresholds,
        archive_dir: archive_dir.as_deref(),
        force: args.force,
    };

    let level = args.level.unwrap_or(config.compression_level()?);
    let threads = args.threads.unwrap_or(config.compression_threads()?);
    let skip = config.skip_dirs();

    let projects = targets(args, &db)?;
    if projects.is_empty() {
        if ctx.is_json() {
            return json::emit(&json::RunReport {
                applied: args.yes,
                projects: Vec::new(),
                totals: Totals::default().as_json(),
            });
        }
        if !ctx.is_quiet() {
            let mut out = std::io::stdout().lock();
            writeln!(out, "no projects tracked yet")?;
            writeln!(out, "track one with `pmanager project add <PROJECT>`")?;
        }
        return Ok(());
    }

    let mut totals = Totals::default();
    let mut collected = Collected::default();

    for record in &projects {
        // Resolved per project: which directories are rebuildable depends on
        // what the project is built with.
        let purge = config.purge_dirs(&record.path);
        let survey = {
            // Walking a large tree is the first thing that can take long
            // enough to look like a hang, before anything has been printed.
            let _spinner =
                Spinner::start(ctx.shows_progress(), format!("scanning {}", record.name));
            lifecycle::survey(&record.path, &skip, &purge)
        };
        let action = lifecycle::decide(record.state, &survey, record.active_since(), &policy);

        let options = archive::Options {
            level,
            threads,
            // What pManager will delete is what it refuses to store: a
            // dependency tree rebuilds from the files beside it.
            exclude: purge,
            overwrite: args.force,
        };

        let outcome = match visit(
            ctx,
            &db,
            record,
            &survey,
            &action,
            args,
            &options,
            thresholds,
            &mut totals,
        ) {
            Ok(outcome) => outcome,
            Err(err) => {
                // Report and carry on: the next project may be perfectly
                // healthy. Errors go to stderr even under `--quiet`, and the
                // document below carries them too.
                tracing::error!(project = %record.path.display(), error = %err, "failed");
                eprintln!("error: {}: {err}", record.name);
                totals.failed += 1;
                Outcome {
                    description: err.to_string(),
                    error: Some(err.to_string()),
                    ..Outcome::default()
                }
            }
        };

        collected.push(record, &survey, &action, &outcome);
    }

    if ctx.is_json() {
        json::emit(&json::RunReport {
            applied: args.yes,
            projects: collected.projects,
            totals: totals.as_json(),
        })?;
    } else {
        report(ctx, &totals, args)?;
    }

    if totals.failed > 0 {
        return Err(Error::Archive(format!(
            "{} of {} project(s) could not be processed",
            totals.failed,
            projects.len()
        )));
    }

    Ok(())
}

/// The projects this invocation should visit.
fn targets(args: &Args, db: &Database) -> Result<Vec<ProjectRecord>> {
    if args.project.is_empty() {
        return db.projects();
    }

    args.project
        .iter()
        .map(|query| db.find_project(query))
        .collect()
}

/// Carry out — or merely describe — one project's action.
///
/// Returns what became of it. The sentence in there is the one the printed
/// line uses, so `--json`'s `description` and the terminal cannot drift apart.
#[allow(clippy::too_many_arguments)]
fn visit(
    ctx: &Context,
    db: &Database,
    record: &ProjectRecord,
    survey: &Survey,
    action: &Action,
    args: &Args,
    options: &archive::Options,
    thresholds: Thresholds,
    totals: &mut Totals,
) -> Result<Outcome> {
    let mut out = std::io::stdout().lock();
    // Supplementary lines: printed under the project's own, and folded into
    // the outcome's `error` so nothing is visible only in the terminal.
    let mut notes: Vec<String> = Vec::new();

    // These narrow what a run may do without changing how a project is judged.
    let suppressed = match action {
        Action::Trim { .. } => args.no_trim,
        Action::Archive { .. } => args.no_archive,
        _ => false,
    };

    if suppressed || !action.is_change() {
        totals.untouched += 1;
        let reason = match action {
            _ if suppressed => "excluded by this run's flags".to_string(),
            Action::Nothing { reason } => reason.clone(),
            _ => String::new(),
        };
        if args.all && ctx.prints_text() {
            writeln!(out, "{}  {reason}", label(record, survey))?;
        }
        return Ok(Outcome {
            description: reason,
            ..Outcome::default()
        });
    }

    let mut outcome = Outcome::default();
    let mut failure = None;

    outcome.description = match action {
        Action::Missing => {
            totals.missing += 1;
            "folder is gone; `pmanager project prune` clears it out".to_string()
        }

        Action::HeldBack { reason } => {
            totals.held += 1;
            // Saying when the hold lapses turns "held back" from a state
            // the project might sit in forever into a deadline.
            let lapses = match survey.idle_days_from(record.active_since()) {
                Some(idle) => format!(
                    "; archived regardless in {} day(s)",
                    u64::from(thresholds.force_archive_days).saturating_sub(idle)
                ),
                None => String::new(),
            };
            format!("old enough to archive, held back: {reason}{lapses}")
        }

        Action::Revive => {
            if args.yes {
                totals.revived += 1;
                outcome.changed = true;
                db.mark_active(record.id)?;
            } else {
                totals.planned += 1;
            }
            format!(
                "worked on again; {} active",
                if args.yes { "marked" } else { "would mark" }
            )
        }

        Action::Trim { dirs } => {
            let count = dirs.len();

            if args.yes {
                let result = {
                    let _spinner =
                        Spinner::start(ctx.shows_progress(), format!("trimming {}", record.name));
                    lifecycle::trim(dirs)
                };
                totals.freed += result.freed;
                outcome.freed = result.freed;

                // A partial trim still counts; the survivors are found again
                // on the next run.
                if result.removed > 0 {
                    totals.trimmed += 1;
                    outcome.changed = true;
                    db.mark_trimmed(record.id, result.freed)?;
                }

                for (path, err) in &result.failed {
                    notes.push(format!("could not remove {}: {err}", path.display()));
                }
                if result.removed == 0 {
                    failure = Some(Error::Io(std::io::Error::other(format!(
                        "none of the {count} dependency tree(s) could be removed"
                    ))));
                }

                format!(
                    "trimmed {} dependency tree(s), freed {}",
                    result.removed,
                    human_bytes(result.freed)
                )
            } else {
                totals.planned += 1;
                format!(
                    "would trim {count} dependency tree(s), freeing {}",
                    human_bytes(scan::total_bytes(dirs))
                )
            }
        }

        Action::Archive { destination } => {
            if !args.yes {
                totals.planned += 1;
                format!("would archive to {}", destination.display())
            } else {
                // The slow one, and the reason this module has a spinner at
                // all: compression is a single call that can run for minutes.
                // The spinner is dropped — and its line erased — before the
                // result is printed below.
                let summary = {
                    let _spinner =
                        Spinner::start(ctx.shows_progress(), format!("archiving {}", record.name));
                    lifecycle::archive(&record.path, destination, options, args.keep_source)?
                };
                totals.archived += 1;
                outcome.changed = true;

                // Only a deleted folder gives anything back.
                let freed = if args.keep_source {
                    0
                } else {
                    summary.source_bytes.saturating_sub(summary.archive_bytes)
                };
                totals.freed += freed;
                outcome.freed = freed;
                outcome.archive = Some(json::Archive {
                    path: json::path(destination),
                    bytes: summary.archive_bytes,
                    source_bytes: summary.source_bytes,
                    file_count: summary.file_count,
                    present: destination.is_file(),
                });

                db.mark_archived(record.id, &summary)?;

                format!(
                    "archived to {}  {} file(s), {} -> {}{}",
                    destination.display(),
                    summary.file_count,
                    human_bytes(summary.source_bytes),
                    human_bytes(summary.archive_bytes),
                    if args.keep_source {
                        ", folder kept".to_string()
                    } else {
                        format!(", freed {}", human_bytes(freed))
                    }
                )
            }
        }

        Action::Nothing { .. } => unreachable!("handled above"),
    };

    if ctx.prints_text() {
        writeln!(out, "{}  {}", label(record, survey), outcome.description)?;
        for note in &notes {
            writeln!(out, "    {note}")?;
        }
    }
    if !notes.is_empty() {
        outcome.error = Some(notes.join("; "));
    }

    match failure {
        // The trees that did come out are already counted; this is what makes
        // a run that achieved nothing exit non-zero.
        Some(err) => Err(err),
        None => Ok(outcome),
    }
}

/// `name  90d ago` — the left-hand column every line starts with.
fn label(record: &ProjectRecord, survey: &Survey) -> String {
    let age = survey
        .last_edit
        .as_ref()
        .map_or_else(|| "—".to_string(), |edit| scan::time_ago(edit.at));

    format!("{:<24}  {:>9}", record.name, age)
}

fn report(ctx: &Context, totals: &Totals, args: &Args) -> Result<()> {
    if !ctx.prints_text() {
        return Ok(());
    }

    let mut out = std::io::stdout().lock();
    let mut parts = if args.yes {
        vec![
            format!("{} trimmed", totals.trimmed),
            format!("{} archived", totals.archived),
            format!("{} freed", human_bytes(totals.freed)),
        ]
    } else {
        vec![format!("{} project(s) would change", totals.planned)]
    };

    for (count, what) in [
        (totals.held, "held back"),
        (totals.missing, "missing"),
        (totals.failed, "failed"),
    ] {
        if count > 0 {
            parts.push(format!("{count} {what}"));
        }
    }

    writeln!(out, "\n{}", parts.join(", "))?;

    if !args.yes && totals.planned > 0 {
        writeln!(out, "nothing was changed; pass --yes to apply")?;
    }

    Ok(())
}
