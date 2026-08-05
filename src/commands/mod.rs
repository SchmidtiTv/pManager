//! Subcommand implementations.
//!
//! One module per subcommand. Each module exposes a clap `Args` struct — which
//! [`crate::cli::Command`] embeds — and a `run` function taking the shared
//! [`Context`].

pub mod completions;
pub mod info;
pub mod list;
pub mod project;
pub mod restore;
pub mod run;
pub mod schedule;

use std::cell::OnceCell;
use std::path::{Path, PathBuf};

use crate::cli::GlobalArgs;
use crate::config::Config;
use crate::db::Database;
use crate::error::Result;

/// State shared by every subcommand.
///
/// The config file is read on first use rather than up front, so commands that
/// do not need it (`completions`, `--help`) keep working even when the file is
/// missing or malformed.
pub struct Context {
    globals: GlobalArgs,
    config: OnceCell<(Config, PathBuf)>,
}

impl Context {
    pub fn new(globals: GlobalArgs) -> Self {
        Self {
            globals,
            config: OnceCell::new(),
        }
    }

    pub fn globals(&self) -> &GlobalArgs {
        &self.globals
    }

    /// Whether `--quiet` suppressed informational output.
    pub fn is_quiet(&self) -> bool {
        self.globals.quiet
    }

    /// Whether the command should print one JSON document instead of prose.
    pub fn is_json(&self) -> bool {
        self.globals.json
    }

    /// Whether the human-readable output should be printed at all.
    ///
    /// `--quiet` silences it and `--json` replaces it, so every `writeln!` of
    /// prose is guarded by this rather than by either flag alone.
    pub fn prints_text(&self) -> bool {
        !self.globals.quiet && !self.globals.json
    }

    /// The loaded configuration, read from disk on first call.
    pub fn config(&self) -> Result<&Config> {
        self.load().map(|(config, _)| config)
    }

    /// The path the configuration was read from.
    pub fn config_path(&self) -> Result<&Path> {
        self.load().map(|(_, path)| path.as_path())
    }

    /// Open the SQLite database, creating and migrating it if necessary.
    ///
    /// Returned by value rather than cached: a connection is cheap, and
    /// holding one open for the whole process would keep a write lock alive
    /// longer than any single command needs it.
    pub fn open_database(&self) -> Result<Database> {
        Database::open(&self.config()?.database_path()?)
    }

    fn load(&self) -> Result<&(Config, PathBuf)> {
        if let Some(loaded) = self.config.get() {
            return Ok(loaded);
        }

        let loaded = Config::load(self.globals.config.as_deref())?;
        Ok(self.config.get_or_init(|| loaded))
    }
}
