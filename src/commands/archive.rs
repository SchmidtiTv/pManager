//! `vmanager archive` — create `.7z` archives and track the ones created.

use std::io::Write;
use std::path::PathBuf;

use crate::archive::{self, Options, Summary, human_bytes};
use crate::commands::Context;
use crate::db::ArchiveRecord;
use crate::error::Result;

#[derive(Debug, clap::Args)]
pub struct Args {
    #[command(subcommand)]
    command: ArchiveCommand,
}

#[derive(Debug, clap::Subcommand)]
enum ArchiveCommand {
    /// Compress a folder into a .7z archive
    Create {
        /// Folder to archive
        #[arg(value_name = "FOLDER")]
        source: PathBuf,

        /// Archive to write [default: <FOLDER>.7z in the current directory]
        #[arg(short, long, value_name = "FILE")]
        output: Option<PathBuf>,

        /// Compression level, 0 (fastest) to 9 (smallest)
        #[arg(short, long, default_value_t = archive::DEFAULT_LEVEL,
              value_parser = clap::value_parser!(u32).range(archive::MIN_LEVEL as i64..=archive::MAX_LEVEL as i64))]
        level: u32,

        /// Compression threads; more threads are faster but compress slightly worse
        #[arg(short, long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..=256))]
        threads: u32,

        /// Skip files and folders with this name; repeatable
        #[arg(short = 'x', long, value_name = "NAME")]
        exclude: Vec<String>,

        /// Overwrite the archive if it already exists
        #[arg(short, long)]
        force: bool,

        /// Do not record the archive in the database
        #[arg(long)]
        no_record: bool,
    },

    /// List archives recorded in the database
    List {
        /// Maximum number of archives to show
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: u32,
    },

    /// Remove an archive's record; the file itself is left alone
    Forget {
        /// Row id shown by `vmanager archive list`
        id: i64,
    },

    /// Drop records whose archive file no longer exists
    Prune,
}

pub fn run(args: &Args, ctx: &Context) -> Result<()> {
    match &args.command {
        ArchiveCommand::Create {
            source,
            output,
            level,
            threads,
            exclude,
            force,
            no_record,
        } => create(
            ctx,
            source,
            output.as_deref(),
            &Options {
                level: *level,
                threads: *threads,
                exclude: exclude.clone(),
                overwrite: *force,
            },
            *no_record,
        ),
        ArchiveCommand::List { limit } => list(ctx, *limit),
        ArchiveCommand::Forget { id } => forget(ctx, *id),
        ArchiveCommand::Prune => prune(ctx),
    }
}

fn create(
    ctx: &Context,
    source: &std::path::Path,
    output: Option<&std::path::Path>,
    options: &Options,
    no_record: bool,
) -> Result<()> {
    let dest = match output {
        Some(path) => path.to_path_buf(),
        None => archive::default_output_path(source)?,
    };

    let summary = archive::create(source, &dest, options)?;

    if !no_record {
        record(ctx, &summary);
    }

    if !ctx.is_quiet() {
        let mut out = std::io::stdout().lock();
        writeln!(out, "{}", summary.archive.display())?;
        writeln!(
            out,
            "  {} files, {} -> {}{}",
            summary.file_count,
            human_bytes(summary.source_bytes),
            human_bytes(summary.archive_bytes),
            summary
                .ratio()
                .map(|ratio| format!(" ({ratio:.1}% of original)"))
                .unwrap_or_default(),
        )?;
    }

    Ok(())
}

/// Save the run to the database.
///
/// The archive is already on disk at this point, so a database problem is
/// reported as a warning rather than failing the command — losing the record
/// is not worth discarding a completed archive.
fn record(ctx: &Context, summary: &Summary) {
    let recorded = ctx
        .open_database()
        .and_then(|db| db.record_archive(summary));

    match recorded {
        Ok(id) => tracing::info!(id, "recorded archive"),
        Err(err) => tracing::warn!(
            error = %err,
            "archive created but could not be recorded in the database"
        ),
    }
}

fn list(ctx: &Context, limit: u32) -> Result<()> {
    let db = ctx.open_database()?;
    let records = db.recent_archives(limit)?;

    let mut out = std::io::stdout().lock();
    if records.is_empty() {
        if !ctx.is_quiet() {
            writeln!(out, "no archives recorded yet")?;
        }
        return Ok(());
    }

    let id_width = records
        .iter()
        .map(|record| record.id.to_string().len())
        .max()
        .unwrap_or(2)
        .max(2);

    for record in &records {
        writeln!(
            out,
            "{:>id_width$}  {}  {:>5} files  {:>9}  {}",
            record.id,
            record.created_at,
            record.file_count,
            human_bytes(record.archive_bytes),
            record.archive_path.display(),
        )?;
    }

    Ok(())
}

fn forget(ctx: &Context, id: i64) -> Result<()> {
    let db = ctx.open_database()?;
    let record = db.archive(id)?;
    db.forget_archive(id)?;

    if !ctx.is_quiet() {
        writeln!(
            std::io::stdout().lock(),
            "forgot {} ({})",
            id,
            record.archive_path.display()
        )?;
    }
    Ok(())
}

fn prune(ctx: &Context) -> Result<()> {
    let db = ctx.open_database()?;
    let pruned: Vec<ArchiveRecord> = db.prune_missing()?;

    if !ctx.is_quiet() {
        let mut out = std::io::stdout().lock();
        for record in &pruned {
            writeln!(
                out,
                "pruned {} ({})",
                record.id,
                record.archive_path.display()
            )?;
        }
        writeln!(out, "{} record(s) pruned", pruned.len())?;
    }

    Ok(())
}
