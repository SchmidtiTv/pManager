//! `pmanager info` — everything known about one project.
//!
//! `list` answers "which of these needs attention"; this answers "why is this
//! one being treated the way it is". The last line says what the next run
//! would do, and when the answer is nothing, why not.

use std::io::Write;

use crate::archive::human_bytes;
use crate::commands::Context;
use crate::error::Result;
use crate::lifecycle::{self, Action, Policy, State};
use crate::scan;

/// Width of the label column.
const LABEL: usize = 16;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// A project's path or name, as shown by `pmanager list`
    #[arg(value_name = "PROJECT")]
    project: String,
}

pub fn run(args: &Args, ctx: &Context) -> Result<()> {
    let db = ctx.open_database()?;
    let config = ctx.config()?;
    let record = db.find_project(&args.project)?;

    let survey = lifecycle::survey(&record.path, &config.skip_dirs(), &config.purge_dirs());
    let policy = Policy {
        thresholds: config.thresholds()?,
        archive_dir: config.archive_dir.as_deref(),
        force: false,
    };
    let action = lifecycle::decide(record.state, &survey, record.active_since(), &policy);

    let mut out = std::io::stdout().lock();
    writeln!(out, "{}  {}", record.name, record.path.display())?;

    let field = |out: &mut dyn Write, label: &str, value: &str| -> std::io::Result<()> {
        writeln!(out, "  {label:<LABEL$}{value}")
    };

    field(&mut out, "state", &state_line(&record))?;
    field(
        &mut out,
        "tracked",
        &scan::time_ago(scan::from_epoch(record.added_at)),
    )?;

    match record.state {
        State::Archived => archive_fields(&mut out, &record, &field)?,
        _ if !survey.present => {
            field(&mut out, "folder", "MISSING — nothing at that path")?;
        }
        _ => live_fields(&mut out, &survey, &field)?,
    }

    if record.freed_bytes > 0 {
        field(&mut out, "reclaimed", &human_bytes(record.freed_bytes))?;
    }
    if let Some(at) = record.restored_at {
        field(
            &mut out,
            "restored",
            &format!(
                "{} — the idle clock runs from here",
                scan::time_ago(scan::from_epoch(at))
            ),
        )?;
    }

    git_fields(&mut out, &survey, &field)?;

    writeln!(out)?;
    field(&mut out, "next run", &next_run(&action, &policy))?;
    Ok(())
}

fn state_line(record: &crate::db::ProjectRecord) -> String {
    let since = match record.state {
        State::Trimmed => record.trimmed_at,
        State::Archived => record.archived_at,
        State::Active => None,
    };

    match since {
        Some(at) => format!(
            "{} since {}",
            record.state,
            scan::time_ago(scan::from_epoch(at))
        ),
        None => record.state.to_string(),
    }
}

/// Fields that only make sense while the project is on disk.
fn live_fields(
    out: &mut impl Write,
    survey: &lifecycle::Survey,
    field: &impl Fn(&mut dyn Write, &str, &str) -> std::io::Result<()>,
) -> Result<()> {
    match &survey.last_edit {
        Some(edit) => {
            // Which file it was is often what jogs the memory.
            let relative = edit
                .path
                .strip_prefix(&survey.path)
                .unwrap_or(&edit.path)
                .display();
            field(
                out,
                "last edit",
                &format!("{}  {relative}", scan::time_ago(edit.at)),
            )?;
        }
        None => field(out, "last edit", "no files found to date it by")?,
    }

    if survey.purgeable.is_empty() {
        field(out, "dependencies", "none on disk")?;
    } else {
        field(
            out,
            "dependencies",
            &format!(
                "{} in {} tree(s)",
                human_bytes(survey.purgeable_bytes()),
                survey.purgeable.len()
            ),
        )?;
        for dir in &survey.purgeable {
            let relative = dir.path.strip_prefix(&survey.path).unwrap_or(&dir.path);
            writeln!(
                out,
                "  {:<LABEL$}  {}  {}",
                "",
                human_bytes(dir.bytes),
                relative.display()
            )?;
        }
    }

    Ok(())
}

/// Fields describing where an archived project went.
fn archive_fields(
    out: &mut impl Write,
    record: &crate::db::ProjectRecord,
    field: &impl Fn(&mut dyn Write, &str, &str) -> std::io::Result<()>,
) -> Result<()> {
    let Some(archive) = &record.archive else {
        field(out, "archive", "no archive recorded")?;
        return Ok(());
    };

    let present = archive.path.is_file();
    field(
        out,
        "archive",
        &format!(
            "{}{}",
            archive.path.display(),
            if present { "" } else { "  — MISSING" }
        ),
    )?;
    field(
        out,
        "compressed",
        &format!(
            "{} from {} in {} file(s)",
            human_bytes(archive.bytes),
            human_bytes(archive.source_bytes),
            archive.file_count
        ),
    )?;

    if present {
        field(
            out,
            "restore with",
            &format!("pmanager restore {}", record.name),
        )?;
    }
    Ok(())
}

/// What git makes of the project.
fn git_fields(
    out: &mut impl Write,
    survey: &lifecycle::Survey,
    field: &impl Fn(&mut dyn Write, &str, &str) -> std::io::Result<()>,
) -> Result<()> {
    let git = &survey.git;
    if !git.repo {
        if survey.present {
            field(out, "git", "not a repository")?;
        }
        return Ok(());
    }

    let branch = git.branch.as_deref().unwrap_or("(detached)");
    let tracking = match (&git.upstream, git.ahead) {
        (Some(upstream), 0) => format!("  → {upstream}"),
        (Some(upstream), ahead) => format!("  → {upstream}, {ahead} ahead"),
        (None, _) => "  (no upstream)".to_string(),
    };
    field(out, "branch", &format!("{branch}{tracking}"))?;

    match &git.head {
        Some(head) => field(
            out,
            "last commit",
            &format!(
                "{}  {}  {}  ({})",
                scan::time_ago(head.at),
                head.sha,
                head.subject,
                head.author
            ),
        )?,
        None => field(out, "last commit", "no commits yet")?,
    }

    for (label, paths) in [
        ("staged", &git.staged),
        ("unstaged", &git.unstaged),
        ("untracked", &git.untracked),
    ] {
        group(out, label, paths)?;
    }

    if !git.is_dirty() {
        field(out, "working tree", "clean")?;
    }
    Ok(())
}

/// A labelled list of paths, truncated so one group cannot bury the rest.
fn group(out: &mut impl Write, label: &str, paths: &[String]) -> std::io::Result<()> {
    const LIMIT: usize = 10;

    if paths.is_empty() {
        return Ok(());
    }

    writeln!(out, "  {:<LABEL$}{} file(s)", label, paths.len())?;
    for path in paths.iter().take(LIMIT) {
        writeln!(out, "  {:<LABEL$}  {path}", "")?;
    }
    if paths.len() > LIMIT {
        writeln!(out, "  {:<LABEL$}  … and {} more", "", paths.len() - LIMIT)?;
    }
    Ok(())
}

/// What `pmanager run` would do to this project, and why.
fn next_run(action: &Action, policy: &Policy<'_>) -> String {
    match action {
        Action::Trim { dirs } => format!(
            "trim {} dependency tree(s), freeing {}",
            dirs.len(),
            human_bytes(scan::total_bytes(dirs))
        ),
        Action::Archive { destination } => {
            format!("archive to {}", destination.display())
        }
        Action::HeldBack { reason } => format!(
            "nothing — past {} days, but held back by {reason} (--force overrides)",
            policy.thresholds.archive_days
        ),
        Action::Revive => "mark it active again".to_string(),
        Action::Missing => "nothing — `pmanager project prune` clears it out".to_string(),
        Action::Nothing { reason } => format!("nothing — {reason}"),
    }
}
