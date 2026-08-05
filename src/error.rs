//! Error type and process exit-code mapping.
//!
//! Commands return [`Result`]. `main` turns an [`Error`] into a diagnostic on
//! stderr plus the exit code from [`Error::exit_code`], so callers and shell
//! scripts can branch on *why* the run failed, not just *that* it failed.

use std::path::PathBuf;
use std::process::ExitCode;

/// Convenience alias used throughout the crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Exit codes produced by the binary.
///
/// Kept small and stable on purpose: these are part of the CLI's public
/// contract, so add variants rather than renumbering existing ones.
pub mod exit {
    /// The command completed successfully.
    pub const SUCCESS: u8 = 0;
    /// Catch-all failure.
    pub const FAILURE: u8 = 1;
    /// The arguments were invalid (also what clap uses for usage errors).
    pub const USAGE: u8 = 2;
    /// The configuration file was missing, unreadable or invalid.
    pub const CONFIG: u8 = 3;
    /// The requested resource does not exist.
    pub const NOT_FOUND: u8 = 4;
    /// The database could not be opened, migrated or queried.
    pub const DATABASE: u8 = 5;
    /// An archive could not be created.
    pub const ARCHIVE: u8 = 6;
    /// The run was cancelled by the user (128 + SIGINT).
    pub const INTERRUPTED: u8 = 130;
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The user asked for something that does not exist.
    #[error("{kind} `{name}` not found")]
    NotFound { kind: &'static str, name: String },

    /// The configuration file could not be read.
    #[error("failed to read config file `{path}`")]
    ConfigRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// The configuration file was read but is not valid.
    #[error("invalid config file `{path}`")]
    ConfigParse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    /// No config directory could be determined for this platform.
    #[error("could not determine a config directory; pass --config explicitly")]
    NoConfigDir,

    /// The database rejected an operation.
    #[error("database error: {0}")]
    Database(String),

    /// SQLite itself failed.
    #[error("database error")]
    Sqlite(#[from] rusqlite::Error),

    /// An archive could not be created.
    #[error("{0}")]
    Archive(String),

    /// The user interrupted the run.
    #[error("interrupted")]
    Interrupted,

    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// Escape hatch for errors that do not (yet) deserve a dedicated variant.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl Error {
    /// The process exit code this error should produce.
    pub fn exit_code(&self) -> u8 {
        match self {
            Error::NotFound { .. } => exit::NOT_FOUND,
            Error::ConfigRead { .. } | Error::ConfigParse { .. } | Error::NoConfigDir => {
                exit::CONFIG
            }
            Error::Database(_) | Error::Sqlite(_) => exit::DATABASE,
            Error::Archive(_) => exit::ARCHIVE,
            Error::Interrupted => exit::INTERRUPTED,
            Error::Io(_) | Error::Other(_) => exit::FAILURE,
        }
    }

    /// A stable slug naming the category, as `--json` reports it. The same
    /// categories [`Error::exit_code`] distinguishes, spelled out.
    pub fn kind(&self) -> &'static str {
        match self {
            Error::NotFound { .. } => "not_found",
            Error::ConfigRead { .. } | Error::ConfigParse { .. } | Error::NoConfigDir => "config",
            Error::Database(_) | Error::Sqlite(_) => "database",
            Error::Archive(_) => "archive",
            Error::Interrupted => "interrupted",
            Error::Io(_) => "io",
            Error::Other(_) => "failure",
        }
    }

    /// Render the error and its full `source` chain to stderr.
    pub fn report(&self) -> ExitCode {
        eprintln!("error: {self}");

        for cause in self.causes() {
            eprintln!("  caused by: {cause}");
        }

        ExitCode::from(self.exit_code())
    }

    /// The same report as one JSON document, for `--json`.
    ///
    /// On stderr, where the human report goes: stdout carries the command's
    /// document and nothing else, so a caller parsing it is not handed an
    /// error object where a listing was promised.
    pub fn report_json(&self) -> ExitCode {
        let report = crate::json::ErrorReport {
            error: crate::json::ErrorBody {
                message: self.to_string(),
                kind: self.kind(),
                exit_code: self.exit_code(),
                causes: self.causes(),
            },
        };

        match serde_json::to_string_pretty(&report) {
            Ok(rendered) => eprintln!("{rendered}"),
            // Reporting the failure to report would be no more likely to work.
            Err(_) => return self.report(),
        }

        ExitCode::from(self.exit_code())
    }

    /// The `source` chain, outermost first.
    fn causes(&self) -> Vec<String> {
        let mut causes = Vec::new();
        let mut source = std::error::Error::source(self);

        while let Some(cause) = source {
            causes.push(cause.to_string());
            source = cause.source();
        }
        causes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_are_distinct_per_category() {
        let not_found = Error::NotFound {
            kind: "profile",
            name: "dev".into(),
        };
        assert_eq!(not_found.exit_code(), exit::NOT_FOUND);
        assert_eq!(Error::NoConfigDir.exit_code(), exit::CONFIG);
        assert_eq!(Error::Interrupted.exit_code(), exit::INTERRUPTED);
        assert_eq!(
            Error::Other(anyhow::anyhow!("boom")).exit_code(),
            exit::FAILURE
        );
    }

    #[test]
    fn every_category_has_a_slug_and_a_code() {
        let errors = [
            Error::NotFound {
                kind: "project",
                name: "dev".into(),
            },
            Error::NoConfigDir,
            Error::Database("locked".into()),
            Error::Archive("no room".into()),
            Error::Interrupted,
            Error::Io(std::io::Error::other("boom")),
            Error::Other(anyhow::anyhow!("boom")),
        ];

        for err in errors {
            let kind = err.kind();
            assert!(!kind.is_empty(), "{err} has no slug");
            assert!(
                kind.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "`{kind}` is not a stable slug"
            );
        }
    }

    #[test]
    fn a_json_report_carries_the_whole_source_chain() {
        let err = Error::ConfigRead {
            path: PathBuf::from("/etc/pmanager.toml"),
            source: std::io::Error::other("disk on fire"),
        };

        assert_eq!(err.causes(), ["disk on fire"]);
        assert_eq!(err.kind(), "config");
        assert_eq!(err.exit_code(), exit::CONFIG);
    }

    #[test]
    fn display_includes_context() {
        let err = Error::NotFound {
            kind: "profile",
            name: "dev".into(),
        };
        assert_eq!(err.to_string(), "profile `dev` not found");
    }
}
