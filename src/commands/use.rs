//! `pmanager use` — switch vManager to the versions a project is pinned to.
//!
//! pManager stores no version files or PATH entries of its own: switching a
//! version is entirely vManager's job (see vManager's `current` link,
//! documented in its own README). This command is only the trigger —
//! `pmanager project set-version` says which version, this tells vManager to
//! actually make a path resolve there. See [`crate::vmanager`].

use std::io::Write;

use crate::commands::Context;
use crate::error::{Error, Result};
use crate::json;
use crate::vmanager;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// A project's path or name, as shown by `pmanager list`
    #[arg(value_name = "PROJECT")]
    project: String,
}

pub fn run(args: &Args, ctx: &Context) -> Result<()> {
    let db = ctx.open_database()?;
    let record = db.find_project(&args.project)?;
    let pins = db.versions_for_project(record.id)?;

    if pins.is_empty() {
        return Err(Error::Other(anyhow::anyhow!(
            "`{}` has no pinned versions; set one with \
             `pmanager project set-version {} <FOLDER> <VERSION>`",
            record.name,
            record.name
        )));
    }

    if !vmanager::is_available() {
        return Err(Error::Other(anyhow::anyhow!(
            "vmanager is not on PATH; install it to switch versions"
        )));
    }

    let mut out = std::io::stdout().lock();
    let mut switches = Vec::with_capacity(pins.len());
    let mut failed = 0;

    for pin in &pins {
        let outcome = vmanager::switch(&pin.folder, &pin.version);
        match &outcome {
            Ok(()) => {
                if ctx.prints_text() {
                    writeln!(out, "{}  -> {}", pin.folder.display(), pin.version)?;
                }
            }
            Err(err) => {
                failed += 1;
                eprintln!("error: {}: {err}", pin.folder.display());
            }
        }
        switches.push(json::VersionSwitch {
            folder: json::path(&pin.folder),
            version: pin.version.clone(),
            applied: outcome.is_ok(),
            error: outcome.err().map(|err| err.to_string()),
        });
    }

    if ctx.is_json() {
        json::emit(&json::UseReport {
            project: record.name.clone(),
            switches,
        })?;
    }

    if failed > 0 {
        return Err(Error::Other(anyhow::anyhow!(
            "{failed} of {} version(s) could not be switched",
            pins.len()
        )));
    }
    Ok(())
}
