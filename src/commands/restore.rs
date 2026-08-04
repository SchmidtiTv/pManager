//! `pmanager restore` — bring an archived project back: extract, reinstall
//! dependencies, mark it active.
//!
//! The archive is kept unless `--delete-archive` is given. Deleting the only
//! copy the moment it is unpacked, before anyone has looked at the result,
//! would be the one irreversible step in a design that has none.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::archive::{self, human_bytes};
use crate::commands::Context;
use crate::error::{Error, Result};
use crate::lifecycle::State;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// An archived project's name or original path
    #[arg(value_name = "PROJECT")]
    project: String,

    /// Restore into this directory instead of where it came from
    #[arg(long, value_name = "DIR")]
    into: Option<PathBuf>,

    /// Skip reinstalling dependencies
    #[arg(long)]
    no_install: bool,

    /// Delete the archive once the project is back
    #[arg(long)]
    delete_archive: bool,
}

pub fn run(args: &Args, ctx: &Context) -> Result<()> {
    let db = ctx.open_database()?;
    let record = db.find_project(&args.project)?;

    if record.state != State::Archived {
        return Err(Error::Other(anyhow::anyhow!(
            "`{}` is {}, not archived — its folder is still on disk, there is nothing to extract",
            record.name,
            record.state
        )));
    }

    let archive_record = record.archive.as_ref().ok_or_else(|| {
        Error::Database(format!(
            "`{}` is marked archived but no archive path was recorded",
            record.name
        ))
    })?;

    if !archive_record.path.is_file() {
        return Err(Error::NotFound {
            kind: "archive",
            name: archive_record.path.display().to_string(),
        });
    }

    // An archived project's recorded path is where its folder used to be.
    let into = match &args.into {
        Some(dir) => dir.clone(),
        None => record.path.parent().map(Path::to_path_buf).ok_or_else(|| {
            Error::Other(anyhow::anyhow!(
                "`{}` has no parent directory",
                record.path.display()
            ))
        })?,
    };
    std::fs::create_dir_all(&into)?;

    let mut out = std::io::stdout().lock();
    if !ctx.is_quiet() {
        writeln!(
            out,
            "restoring {} from {} ({})",
            record.name,
            archive_record.path.display(),
            human_bytes(archive_record.bytes)
        )?;
    }

    let restored = archive::extract(&archive_record.path, &into)?;

    // Before the install: a failed `npm install` must not leave the database
    // claiming the project is still archived when the folder is right there.
    db.mark_restored(record.id, &restored)?;

    if !ctx.is_quiet() {
        writeln!(out, "  extracted to {}", restored.display())?;
    }

    let installed = install(ctx, &restored, args.no_install, &mut out)?;

    if args.delete_archive {
        std::fs::remove_file(&archive_record.path)?;
        if !ctx.is_quiet() {
            writeln!(out, "  deleted {}", archive_record.path.display())?;
        }
    } else if !ctx.is_quiet() {
        writeln!(
            out,
            "  archive kept at {}; delete it once you are happy",
            archive_record.path.display()
        )?;
    }

    if !ctx.is_quiet() {
        writeln!(
            out,
            "\n{} is active again{}",
            record.name,
            if installed {
                ""
            } else {
                " (dependencies not installed)"
            }
        )?;
    }

    Ok(())
}

/// Reinstall dependencies, if a command applies. A failure is reported but does
/// not fail the restore — the project is already back, and the install is
/// trivially re-run by hand.
fn install(ctx: &Context, project: &Path, skip: bool, out: &mut impl Write) -> Result<bool> {
    if skip {
        return Ok(false);
    }

    let Some(command) = ctx.config()?.install_command(project) else {
        return Ok(false);
    };

    let (program, arguments) = command.split_first().expect("never configured empty");
    if !ctx.is_quiet() {
        writeln!(out, "  running {}", command.join(" "))?;
    }

    let status = std::process::Command::new(program)
        .args(arguments)
        .current_dir(project)
        .status();

    match status {
        Ok(status) if status.success() => Ok(true),
        Ok(status) => {
            writeln!(out, "  {} exited with {status}", command.join(" "))?;
            Ok(false)
        }
        Err(err) => {
            writeln!(out, "  could not run {program}: {err}")?;
            Ok(false)
        }
    }
}
