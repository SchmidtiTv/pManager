//! `vmanager config` — inspect and create the configuration file.

use std::io::Write;

use crate::commands::Context;
use crate::config::Config;
use crate::error::Result;

#[derive(Debug, clap::Args)]
pub struct Args {
    #[command(subcommand)]
    command: ConfigCommand,
}

#[derive(Debug, clap::Subcommand)]
enum ConfigCommand {
    /// Print the effective configuration as TOML
    Show,

    /// Print the path of the configuration file
    Path,

    /// Write a configuration file containing the defaults
    Init {
        /// Overwrite an existing configuration file
        #[arg(short, long)]
        force: bool,
    },
}

pub fn run(args: &Args, ctx: &Context) -> Result<()> {
    match &args.command {
        ConfigCommand::Show => show(ctx),
        ConfigCommand::Path => path(ctx),
        ConfigCommand::Init { force } => init(ctx, *force),
    }
}

/// Print the values actually in effect. Written to stdout so it can be piped.
fn show(ctx: &Context) -> Result<()> {
    let mut out = std::io::stdout().lock();
    write!(out, "{}", ctx.config()?.effective()?.to_toml())?;
    Ok(())
}

/// Print the config file path, whether or not the file exists.
fn path(ctx: &Context) -> Result<()> {
    let mut out = std::io::stdout().lock();
    writeln!(out, "{}", ctx.config_path()?.display())?;
    Ok(())
}

/// Create a commented config file the user can edit.
fn init(ctx: &Context, force: bool) -> Result<()> {
    let path = Config::resolve_path(ctx.globals().config.as_deref())?;
    Config::write_template(&path, force)?;

    if !ctx.is_quiet() {
        writeln!(std::io::stdout().lock(), "wrote {}", path.display())?;
    }
    Ok(())
}
