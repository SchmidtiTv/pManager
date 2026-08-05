//! Command line surface: the parser, the global flags and the subcommand list.
//!
//! Everything a user can type is declared here; the actual work lives in
//! [`crate::commands`]. Adding a subcommand means adding a variant to
//! [`Command`] and a module under `commands/`.

use std::io::IsTerminal;
use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

/// Long help text shown by `pmanager --help`.
const AFTER_HELP: &str = "\
Getting started:
  pmanager project add C:\\Code\\my-app   Track a project
  pmanager list                          See every project at a glance
  pmanager run                           See what has aged out (changes nothing)
  pmanager run --yes                     Carry it out
  pmanager schedule install              …every day at 03:00
  pmanager completions --help            Complete commands as you type them
  pmanager list --json                   The same, as data for a script

How a project ages:
  active   ──60 days idle──▶  trimmed   dependencies deleted; a reinstall undoes it
  trimmed  ─180 days idle──▶  archived  compressed to .pArchived, folder gone
  archived ──────────────────▶  active  `pmanager restore <PROJECT>`

  Idle means no edited files — dependency installs and builds do not count.
  Uncommitted or unpushed work holds a project back from being archived, but
  only until 240 days idle; past that it is archived anyway, history and all.

  What trimming deletes follows the project: node_modules for Node, target for
  Rust, .venv and __pycache__ for Python, vendor for Go, build and .gradle for
  Java, Kotlin and Android. A project with no recognised manifest has nothing
  deleted.

Configuration (optional; every key has a default):
  Windows           %APPDATA%\\pmanager\\config.toml
  Linux             ~/.config/pmanager/config.toml
  macOS             ~/Library/Application Support/pmanager/config.toml

  trim_days, archive_days, force_archive_days, purge_dirs, skip_dirs,
  archive_dir, install_command, compression_level, compression_threads,
  data_dir, database

Environment:
  PMANAGER_CONFIG   Path to the config file (overridden by --config)
  NO_COLOR          Disable colored output
  RUST_LOG          Fine-grained log filter, e.g. `pmanager=debug`

Exit codes:
  0  success               3  configuration error    6  archive error
  1  failure               4  not found            130  interrupted
  2  invalid arguments     5  database error";

#[derive(Debug, Parser)]
#[command(
    name = "pmanager",
    version,
    about = "pManager command line interface",
    propagate_version = true,
    after_help = AFTER_HELP,
    // Print help instead of a bare usage error when invoked with no arguments.
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(flatten)]
    pub globals: GlobalArgs,

    #[command(subcommand)]
    pub command: Command,
}

/// Flags accepted by every subcommand.
#[derive(Debug, clap::Args)]
pub struct GlobalArgs {
    /// Increase logging verbosity (repeat for more: -v, -vv, -vvv)
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Silence all output except errors
    #[arg(short, long, global = true, conflicts_with = "verbose")]
    pub quiet: bool,

    /// Print one JSON document instead of the human-readable output
    ///
    /// Conflicts with --quiet: silencing the only thing the command was asked
    /// to print would leave nothing to parse.
    #[arg(long, global = true, conflicts_with = "quiet")]
    pub json: bool,

    /// Path to the configuration file
    #[arg(
        short,
        long,
        global = true,
        value_name = "FILE",
        env = "PMANAGER_CONFIG"
    )]
    pub config: Option<PathBuf>,

    /// When to use colored output
    #[arg(long, global = true, value_name = "WHEN", default_value_t = ColorChoice::Auto)]
    pub color: ColorChoice,
}

impl GlobalArgs {
    /// Log level implied by `--verbose` / `--quiet`.
    pub fn log_level(&self) -> tracing::Level {
        if self.quiet {
            return tracing::Level::ERROR;
        }
        match self.verbose {
            0 => tracing::Level::WARN,
            1 => tracing::Level::INFO,
            2 => tracing::Level::DEBUG,
            _ => tracing::Level::TRACE,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
pub enum ColorChoice {
    /// Colorize when stderr is a terminal
    Auto,
    /// Always colorize
    Always,
    /// Never colorize
    Never,
}

impl ColorChoice {
    /// Resolve the choice against the environment and the actual stream.
    ///
    /// `NO_COLOR` wins over `auto` but not over an explicit `--color always`,
    /// which matches the convention at <https://no-color.org>.
    pub fn should_colorize(self) -> bool {
        match self {
            ColorChoice::Always => true,
            ColorChoice::Never => false,
            ColorChoice::Auto => {
                std::env::var_os("NO_COLOR").is_none() && std::io::stderr().is_terminal()
            }
        }
    }
}

impl std::fmt::Display for ColorChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            ColorChoice::Auto => "auto",
            ColorChoice::Always => "always",
            ColorChoice::Never => "never",
        };
        f.write_str(name)
    }
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Act on projects that have aged out; reports only unless given --yes
    Run(crate::commands::run::Args),

    /// Show every tracked project, one line each
    List(crate::commands::list::Args),

    /// Show everything known about one project
    Info(crate::commands::info::Args),

    /// Track and untrack the projects that `pmanager run` watches
    Project(crate::commands::project::Args),

    /// Extract an archived project and put it back to work
    Restore(crate::commands::restore::Args),

    /// Install or remove the scheduled daily run
    Schedule(crate::commands::schedule::Args),

    /// Print the shell completion script for the given shell
    Completions(crate::commands::completions::Args),
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        // Catches conflicting flags, duplicate short options and bad defaults.
        Cli::command().debug_assert();
    }

    #[test]
    fn verbosity_maps_to_log_levels() {
        let parse = |args: &[&str]| {
            Cli::try_parse_from(args)
                .expect("valid arguments")
                .globals
                .log_level()
        };

        assert_eq!(parse(&["pmanager", "list"]), tracing::Level::WARN);
        assert_eq!(parse(&["pmanager", "-v", "list"]), tracing::Level::INFO);
        assert_eq!(parse(&["pmanager", "-vvv", "list"]), tracing::Level::TRACE);
        assert_eq!(
            parse(&["pmanager", "--quiet", "list"]),
            tracing::Level::ERROR
        );
    }

    #[test]
    fn verbose_and_quiet_conflict() {
        assert!(Cli::try_parse_from(["pmanager", "-v", "-q", "list"]).is_err());
    }

    #[test]
    fn json_is_accepted_by_every_subcommand() {
        let invocations: &[&[&str]] = &[
            &["run"],
            &["list"],
            &["info", "api"],
            &["project", "prune"],
            &["project", "add", "."],
            &["restore", "api"],
            &["schedule", "status"],
            &["completions", "bash"],
        ];

        for invocation in invocations {
            let mut args = vec!["pmanager"];
            args.extend_from_slice(invocation);
            args.push("--json");

            let cli = Cli::try_parse_from(&args).unwrap_or_else(|err| {
                panic!("`{}` should take --json: {err}", invocation.join(" "))
            });
            assert!(cli.globals.json);
        }
    }

    #[test]
    fn json_and_quiet_conflict() {
        // Silencing the document would leave nothing to parse.
        assert!(Cli::try_parse_from(["pmanager", "list", "--json", "-q"]).is_err());
    }

    #[test]
    fn globals_are_accepted_after_the_subcommand() {
        let cli = Cli::try_parse_from(["pmanager", "list", "--color", "never"])
            .expect("globals should propagate to subcommands");
        assert_eq!(cli.globals.color, ColorChoice::Never);
    }

    #[test]
    fn explicit_color_choice_ignores_terminal_detection() {
        assert!(ColorChoice::Always.should_colorize());
        assert!(!ColorChoice::Never.should_colorize());
    }
}
