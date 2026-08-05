//! `pmanager project` — the registry `pmanager run` watches.
//!
//! A tracked project is one project folder — the one holding `package.json`
//! and the source — not the directory they all live in.

use std::io::Write;
use std::path::PathBuf;

use crate::archive::{self, human_bytes};
use crate::commands::Context;
use crate::error::Result;
use crate::lifecycle::State;
use crate::scan;

#[derive(Debug, clap::Args)]
pub struct Args {
    #[command(subcommand)]
    command: ProjectCommand,
}

#[derive(Debug, clap::Subcommand)]
enum ProjectCommand {
    /// Track a project folder
    Add {
        /// The project folder itself, the one holding its source
        #[arg(value_name = "PROJECT", required = true)]
        projects: Vec<PathBuf>,

        /// Name to show it under [default: package.json's name, else the folder]
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
    },

    /// Stop tracking a project; nothing on disk is touched
    Remove {
        /// A project's path or name, as shown by `pmanager list`
        #[arg(value_name = "PROJECT", required = true)]
        projects: Vec<String>,
    },

    /// Stop tracking projects whose folder is gone
    Prune {
        /// Remove them; without this the command only reports
        #[arg(short, long)]
        yes: bool,
    },
}

pub fn run(args: &Args, ctx: &Context) -> Result<()> {
    match &args.command {
        ProjectCommand::Add { projects, name } => add(ctx, projects, name.as_deref()),
        ProjectCommand::Remove { projects } => remove(ctx, projects),
        ProjectCommand::Prune { yes } => prune(ctx, *yes),
    }
}

fn add(ctx: &Context, projects: &[PathBuf], name: Option<&str>) -> Result<()> {
    if projects.len() > 1 && name.is_some() {
        return Err(crate::Error::Other(anyhow::anyhow!(
            "--name applies to one project; add them one at a time to name them"
        )));
    }

    let db = ctx.open_database()?;
    let config = ctx.config()?;
    let mut out = std::io::stdout().lock();

    for project in projects {
        // Canonicalized so two spellings cannot become two registrations.
        let path = archive::canonical_dir(project)?;
        let name = name.map_or_else(|| scan::project_name(&path), str::to_string);
        let (record, inserted) = db.add_project(&path, &name)?;

        if ctx.is_quiet() {
            continue;
        }

        writeln!(
            out,
            "{} {name}  {}",
            if inserted { "tracking" } else { "updated" },
            path.display()
        )?;

        // Say up front what the first run would do, rather than surprising.
        let survey =
            crate::lifecycle::survey(&path, &config.skip_dirs(), &config.purge_dirs(&path));
        match survey.last_edit.as_ref() {
            Some(edit) => {
                let action = crate::lifecycle::decide(
                    record.state,
                    &survey,
                    record.active_since(),
                    &crate::lifecycle::Policy {
                        thresholds: config.thresholds()?,
                        archive_dir: config.archive_dir.as_deref(),
                        force: false,
                    },
                );
                writeln!(
                    out,
                    "  last edited {}, {}",
                    scan::time_ago(edit.at),
                    describe(&action, &survey)
                )?;
            }
            None => writeln!(out, "  no files found to date it by")?,
        }
    }

    Ok(())
}

/// One clause saying what the next run would do to this project.
fn describe(action: &crate::lifecycle::Action, survey: &crate::lifecycle::Survey) -> String {
    use crate::lifecycle::Action;

    match action {
        Action::Trim { dirs } => format!(
            "the next run would free {}",
            human_bytes(scan::total_bytes(dirs))
        ),
        Action::Archive { destination } => {
            format!("the next run would archive it to {}", destination.display())
        }
        Action::HeldBack { reason } => {
            format!("old enough to archive, but held back: {reason}")
        }
        Action::Missing => "the folder is gone".to_string(),
        Action::Revive | Action::Nothing { .. } => {
            if survey.purgeable.is_empty() {
                "nothing for the next run to do".to_string()
            } else {
                format!(
                    "{} in dependencies to reclaim once it goes stale",
                    human_bytes(survey.purgeable_bytes())
                )
            }
        }
    }
}

fn remove(ctx: &Context, projects: &[String]) -> Result<()> {
    let db = ctx.open_database()?;
    let mut out = std::io::stdout().lock();

    for query in projects {
        let record = db.find_project(query)?;
        db.remove_project(record.id)?;

        if !ctx.is_quiet() {
            writeln!(out, "untracked {}  {}", record.name, record.path.display())?;

            // Untracking an archived project abandons its archive, so say where
            // it is while that is still known.
            if let Some(archive) = &record.archive {
                writeln!(
                    out,
                    "  its archive is still at {} ({})",
                    archive.path.display(),
                    human_bytes(archive.bytes)
                )?;
            }
        }
    }

    Ok(())
}

/// Drop registrations whose folder is gone. An archived project is supposed to
/// have none, so it is never pruned.
fn prune(ctx: &Context, yes: bool) -> Result<()> {
    let db = ctx.open_database()?;
    let mut out = std::io::stdout().lock();

    let missing: Vec<_> = db
        .projects()?
        .into_iter()
        .filter(|record| record.state != State::Archived && !record.path.is_dir())
        .collect();

    if missing.is_empty() {
        if !ctx.is_quiet() {
            writeln!(out, "every tracked project is where it should be")?;
        }
        return Ok(());
    }

    for record in &missing {
        writeln!(out, "{}  {}", record.name, record.path.display())?;
    }

    if !yes {
        writeln!(
            out,
            "\n{} project(s) missing; pass --yes to stop tracking them",
            missing.len()
        )?;
        return Ok(());
    }

    for record in &missing {
        db.remove_project(record.id)?;
    }
    writeln!(out, "\nuntracked {} missing project(s)", missing.len())?;
    Ok(())
}
