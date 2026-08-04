//! `vmanager db` — inspect the SQLite database.

use std::io::Write;

use crate::commands::Context;
use crate::db::Database;
use crate::error::Result;

#[derive(Debug, clap::Args)]
pub struct Args {
    #[command(subcommand)]
    command: DbCommand,
}

#[derive(Debug, clap::Subcommand)]
enum DbCommand {
    /// Print the path of the database file
    Path,

    /// Create the database and apply any pending migrations
    Init,

    /// Print schema version and row counts
    Status,
}

pub fn run(args: &Args, ctx: &Context) -> Result<()> {
    match &args.command {
        DbCommand::Path => path(ctx),
        DbCommand::Init => init(ctx),
        DbCommand::Status => status(ctx),
    }
}

/// Print where the database lives without opening (or creating) it.
fn path(ctx: &Context) -> Result<()> {
    let path = ctx.config()?.database_path()?;
    writeln!(std::io::stdout().lock(), "{}", path.display())?;
    Ok(())
}

/// Opening the database is what migrates it, so this is `open` plus a report.
fn init(ctx: &Context) -> Result<()> {
    let db = ctx.open_database()?;

    if !ctx.is_quiet() {
        writeln!(
            std::io::stdout().lock(),
            "database ready at {} (schema v{})",
            db.path().display(),
            db.schema_version()?
        )?;
    }
    Ok(())
}

fn status(ctx: &Context) -> Result<()> {
    let db = ctx.open_database()?;
    let mut out = std::io::stdout().lock();

    writeln!(out, "path      {}", db.path().display())?;
    writeln!(
        out,
        "schema    v{} (latest v{})",
        db.schema_version()?,
        Database::target_version()
    )?;
    writeln!(out, "folders   {}", db.folder_count()?)?;
    writeln!(out, "archives  {}", db.archive_count()?)?;
    Ok(())
}
