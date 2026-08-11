//! `pmanager pins` — every version pin on record, across every tracked
//! project.
//!
//! This is the other half of the vManager/pManager relationship described in
//! `src/vmanager.rs`: pManager remembers which project wants which version;
//! `vmanager run` shells out to `pmanager pins --json` to learn that, so it
//! never archives a version out from under a project that still points at
//! it, even one that has aged out of the newest few vManager would otherwise
//! keep unpacked. This command exists for that call as much as for a human
//! running it directly.

use std::io::Write;

use crate::commands::Context;
use crate::error::Result;
use crate::json;

#[derive(Debug, clap::Args)]
pub struct Args {}

pub fn run(_args: &Args, ctx: &Context) -> Result<()> {
    let db = ctx.open_database()?;
    let pins = db.all_version_pins()?;

    if ctx.is_json() {
        return json::emit(&json::Pins {
            pins: pins
                .iter()
                .map(|pin| json::Pin {
                    folder: json::path(&pin.folder),
                    version: pin.version.clone(),
                    project: pin.project.clone(),
                })
                .collect(),
        });
    }

    if pins.is_empty() {
        if !ctx.is_quiet() {
            let mut out = std::io::stdout().lock();
            writeln!(out, "no project pins a version yet")?;
        }
        return Ok(());
    }

    if !ctx.is_quiet() {
        let mut out = std::io::stdout().lock();
        for pin in &pins {
            writeln!(
                out,
                "{}  {}  {}",
                pin.folder.display(),
                pin.version,
                pin.project
            )?;
        }
        writeln!(out, "\n{} pin(s)", pins.len())?;
    }
    Ok(())
}
