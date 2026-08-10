//! `pmanager project` — the registry `pmanager run` watches.
//!
//! A tracked project is one project folder — the one holding `package.json`
//! and the source — not the directory they all live in.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::archive::{self, human_bytes};
use crate::commands::Context;
use crate::error::Result;
use crate::json;
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

    /// Pin a project to a version of a vManager-managed tool
    ///
    /// pManager only remembers the pin; `pmanager use <PROJECT>` is what
    /// tells vManager to actually switch to it.
    SetVersion {
        /// A project's path or name, as shown by `pmanager list`
        #[arg(value_name = "PROJECT")]
        project: String,

        /// A vManager-registered folder, the parent of the tool's versions
        #[arg(value_name = "FOLDER")]
        folder: PathBuf,

        /// The version to pin to
        //
        // Named `target` rather than `version`: clap gives every subcommand
        // its own `--version`/`-V` flag (`propagate_version = true` on the
        // top-level `Cli`, which reaches nested subcommands too), and a
        // positional argument called `version` collides with it — both would
        // share the clap argument id "version".
        #[arg(value_name = "VERSION")]
        target: String,
    },

    /// Remove a project's pin to a vManager-managed tool
    UnsetVersion {
        /// A project's path or name, as shown by `pmanager list`
        #[arg(value_name = "PROJECT")]
        project: String,

        /// A folder shown by `pmanager info <PROJECT>`
        #[arg(value_name = "FOLDER")]
        folder: PathBuf,
    },
}

pub fn run(args: &Args, ctx: &Context) -> Result<()> {
    match &args.command {
        ProjectCommand::Add { projects, name } => add(ctx, projects, name.as_deref()),
        ProjectCommand::Remove { projects } => remove(ctx, projects),
        ProjectCommand::Prune { yes } => prune(ctx, *yes),
        ProjectCommand::SetVersion {
            project,
            folder,
            target,
        } => set_version(ctx, project, folder, target),
        ProjectCommand::UnsetVersion { project, folder } => unset_version(ctx, project, folder),
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
    let mut tracked = Vec::new();

    for project in projects {
        // Canonicalized so two spellings cannot become two registrations.
        let path = archive::canonical_dir(project)?;
        let name = name.map_or_else(|| scan::project_name(&path), str::to_string);
        let (record, inserted) = db.add_project(&path, &name)?;

        // Nothing below this line changes anything; it only says what was
        // registered and what the first run would make of it.
        if ctx.is_quiet() {
            continue;
        }

        // Say up front what the first run would do, rather than surprising.
        let survey =
            crate::lifecycle::survey(&path, &config.skip_dirs(), &config.purge_dirs(&path));
        let action = match survey.last_edit {
            Some(_) => Some(crate::lifecycle::decide(
                record.state,
                &survey,
                record.active_since(),
                &crate::lifecycle::Policy {
                    thresholds: config.thresholds()?,
                    archive_dir: config.archive_dir.as_deref(),
                    force: false,
                },
            )),
            None => None,
        };

        if ctx.is_json() {
            tracked.push(json::TrackedProject {
                name,
                path: json::path(&path),
                added: inserted,
                last_edit: survey
                    .last_edit
                    .as_ref()
                    .map(|edit| json::Time::at(edit.at)),
                next_run: action
                    .as_ref()
                    .map(|action| json::NextRun::new(action, describe(action, &survey))),
            });
            continue;
        }

        writeln!(
            out,
            "{} {name}  {}",
            if inserted { "tracking" } else { "updated" },
            path.display()
        )?;
        match (survey.last_edit.as_ref(), &action) {
            (Some(edit), Some(action)) => writeln!(
                out,
                "  last edited {}, {}",
                scan::time_ago(edit.at),
                describe(action, &survey)
            )?,
            _ => writeln!(out, "  no files found to date it by")?,
        }
    }

    if ctx.is_json() {
        json::emit(&json::Tracked { projects: tracked })?;
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
    let mut untracked = Vec::new();

    for query in projects {
        let record = db.find_project(query)?;
        db.remove_project(record.id)?;

        if ctx.is_json() {
            untracked.push(json::UntrackedProject {
                name: record.name.clone(),
                path: json::path(&record.path),
                archive: record.archive.as_ref().map(json::Archive::new),
            });
            continue;
        }

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

    if ctx.is_json() {
        json::emit(&json::Untracked {
            projects: untracked,
        })?;
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

    // Whether they were actually dropped, which takes `--yes`.
    let untracked = yes && !missing.is_empty();
    if untracked {
        for record in &missing {
            db.remove_project(record.id)?;
        }
    }

    if ctx.is_json() {
        return json::emit(&json::Pruned {
            projects: missing
                .iter()
                .map(|record| json::MissingProject {
                    name: record.name.clone(),
                    path: json::path(&record.path),
                })
                .collect(),
            untracked,
        });
    }

    if missing.is_empty() {
        if !ctx.is_quiet() {
            writeln!(out, "every tracked project is where it should be")?;
        }
        return Ok(());
    }

    for record in &missing {
        writeln!(out, "{}  {}", record.name, record.path.display())?;
    }

    if untracked {
        writeln!(out, "\nuntracked {} missing project(s)", missing.len())?;
    } else {
        writeln!(
            out,
            "\n{} project(s) missing; pass --yes to stop tracking them",
            missing.len()
        )?;
    }
    Ok(())
}

/// Pin a project to a version of whatever `folder` holds.
fn set_version(ctx: &Context, project: &str, folder: &Path, version: &str) -> Result<()> {
    let db = ctx.open_database()?;
    let record = db.find_project(project)?;
    // Canonicalized so the pin survives however the folder was spelled, and
    // so it lines up with what `vmanager use` itself would resolve it to.
    let folder = archive::canonical_dir(folder)?;
    db.set_version(record.id, &folder, version)?;

    if ctx.is_json() {
        return json::emit(&json::VersionPinned {
            project: record.name.clone(),
            folder: json::path(&folder),
            version: version.to_string(),
        });
    }

    if ctx.prints_text() {
        writeln!(
            std::io::stdout().lock(),
            "{} will use {} {version}\n  run `pmanager use {}` to switch vManager to it",
            record.name,
            folder.display(),
            record.name
        )?;
    }
    Ok(())
}

/// Remove a project's pin to `folder`.
fn unset_version(ctx: &Context, project: &str, folder: &Path) -> Result<()> {
    let db = ctx.open_database()?;
    let record = db.find_project(project)?;
    // A folder that no longer exists must still be removable, the same way
    // `project remove` copes with a project whose own folder is gone.
    let folder = archive::canonical_dir(folder).unwrap_or_else(|_| folder.to_path_buf());
    let removed = db.unset_version(record.id, &folder)?;

    if ctx.is_json() {
        return json::emit(&json::VersionUnpinned {
            project: record.name.clone(),
            folder: json::path(&folder),
            removed,
        });
    }

    if ctx.prints_text() {
        let mut out = std::io::stdout().lock();
        if removed {
            writeln!(
                out,
                "{} no longer pins a version for {}",
                record.name,
                folder.display()
            )?;
        } else {
            writeln!(out, "{} had no pin for {}", record.name, folder.display())?;
        }
    }
    Ok(())
}
