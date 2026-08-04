//! Configuration file handling.
//!
//! Values are layered lowest to highest precedence:
//!
//! 1. the defaults in [`Config`],
//! 2. the TOML config file,
//! 3. environment variables (`PMANAGER_*`, declared in [`crate::cli`]),
//! 4. command line flags.
//!
//! A missing config file is not an error — it simply leaves the defaults in
//! place — but an unreadable or malformed one is.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{Error, Result};

/// Qualifier/organization/application triple used to locate platform
/// directories (`~/.config/pmanager` on Linux, `%APPDATA%\pmanager` on
/// Windows, `~/Library/Application Support/pmanager` on macOS).
const APP_NAME: &str = "pmanager";

/// File name looked up inside the platform config directory.
const CONFIG_FILE: &str = "config.toml";

/// Database file name inside the data directory.
const DATABASE_FILE: &str = "pmanager.db";

/// How many versions of a tool stay unpacked when `keep_versions` is unset.
pub const DEFAULT_KEEP_VERSIONS: u32 = 2;

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Directory where pManager keeps its data. Defaults to the platform data
    /// directory when unset.
    pub data_dir: Option<PathBuf>,

    /// SQLite database file. Defaults to `pmanager.db` inside `data_dir`.
    pub database: Option<PathBuf>,

    /// Versions of each registered folder to keep unpacked. Defaults to
    /// [`DEFAULT_KEEP_VERSIONS`].
    pub keep_versions: Option<u32>,

    /// LZMA2 level used by `pmanager run`. Defaults to
    /// [`crate::archive::DEFAULT_LEVEL`].
    pub compression_level: Option<u32>,

    /// Compression threads used by `pmanager run`. Defaults to 1.
    pub compression_threads: Option<u32>,
}

impl Config {
    /// The config file path, honouring an explicit `--config`/`PMANAGER_CONFIG`
    /// override before falling back to the platform config directory.
    pub fn resolve_path(explicit: Option<&Path>) -> Result<PathBuf> {
        match explicit {
            Some(path) => Ok(path.to_path_buf()),
            None => Ok(Self::default_dir()?.join(CONFIG_FILE)),
        }
    }

    /// Load the config file, falling back to defaults when it does not exist.
    ///
    /// Returns the config together with the path it was (or would have been)
    /// read from, so commands can report where a value came from.
    pub fn load(explicit: Option<&Path>) -> Result<(Self, PathBuf)> {
        let path = Self::resolve_path(explicit)?;

        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!(path = %path.display(), "no config file, using defaults");
                return Ok((Self::default(), path));
            }
            Err(source) => {
                return Err(Error::ConfigRead {
                    path: path.clone(),
                    source,
                });
            }
        };

        let config = toml::from_str(&raw).map_err(|source| Error::ConfigParse {
            path: path.clone(),
            source,
        })?;

        tracing::debug!(path = %path.display(), "loaded config");
        Ok((config, path))
    }

    /// How many versions to keep unpacked.
    ///
    /// Zero is rejected rather than clamped: it would mean "archive every
    /// version", which is never what someone means by a *keep* count, and
    /// silently correcting it would hide the typo.
    pub fn keep_versions(&self) -> Result<u32> {
        match self.keep_versions {
            Some(0) => Err(Error::Other(anyhow::anyhow!(
                "`keep_versions` must be at least 1"
            ))),
            Some(keep) => Ok(keep),
            None => Ok(DEFAULT_KEEP_VERSIONS),
        }
    }

    /// LZMA2 compression level for `pmanager run`.
    pub fn compression_level(&self) -> Result<u32> {
        match self.compression_level {
            Some(level) if level > crate::archive::MAX_LEVEL => Err(Error::Other(anyhow::anyhow!(
                "`compression_level` must be between {} and {}",
                crate::archive::MIN_LEVEL,
                crate::archive::MAX_LEVEL
            ))),
            Some(level) => Ok(level),
            None => Ok(crate::archive::DEFAULT_LEVEL),
        }
    }

    /// Compression threads for `pmanager run`.
    pub fn compression_threads(&self) -> Result<u32> {
        match self.compression_threads {
            Some(0) => Err(Error::Other(anyhow::anyhow!(
                "`compression_threads` must be at least 1"
            ))),
            Some(threads) => Ok(threads),
            None => Ok(1),
        }
    }

    /// The data directory, resolved against the platform default when unset.
    pub fn data_dir(&self) -> Result<PathBuf> {
        match &self.data_dir {
            Some(dir) => Ok(dir.clone()),
            None => Ok(directories::ProjectDirs::from("", "", APP_NAME)
                .ok_or(Error::NoConfigDir)?
                .data_dir()
                .to_path_buf()),
        }
    }

    /// The SQLite database path, resolved against `data_dir` when unset.
    pub fn database_path(&self) -> Result<PathBuf> {
        match &self.database {
            Some(path) => Ok(path.clone()),
            None => Ok(self.data_dir()?.join(DATABASE_FILE)),
        }
    }

    /// Platform config directory for this application.
    fn default_dir() -> Result<PathBuf> {
        Ok(directories::ProjectDirs::from("", "", APP_NAME)
            .ok_or(Error::NoConfigDir)?
            .config_dir()
            .to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_path_wins_over_platform_default() {
        let explicit = Path::new("/etc/pmanager/custom.toml");
        assert_eq!(
            Config::resolve_path(Some(explicit)).unwrap(),
            PathBuf::from("/etc/pmanager/custom.toml")
        );
    }

    #[test]
    fn default_path_lives_in_the_config_dir() {
        let path = Config::resolve_path(None).unwrap();

        // The layout below the config dir is platform-specific — Windows adds
        // its own `config` subfolder — so assert on the parts that hold
        // everywhere rather than on a fixed tail.
        assert_eq!(path.file_name().unwrap(), "config.toml", "got {path:?}");
        assert!(
            path.components().any(|part| part.as_os_str() == "pmanager"),
            "config path should sit under a pmanager directory, got {path:?}"
        );
    }

    #[test]
    fn missing_file_yields_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("absent.toml");

        let (config, reported) = Config::load(Some(&path)).unwrap();
        assert_eq!(config, Config::default());
        assert_eq!(reported, path);
    }

    #[test]
    fn a_hand_written_file_is_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "data_dir = '/srv/pmanager'\nkeep_versions = 4\ncompression_level = 9\n",
        )
        .unwrap();

        let (loaded, _) = Config::load(Some(&path)).unwrap();
        assert_eq!(loaded.data_dir, Some(PathBuf::from("/srv/pmanager")));
        assert_eq!(loaded.keep_versions().unwrap(), 4);
        assert_eq!(loaded.compression_level().unwrap(), 9);
        // Keys the file left out keep their defaults.
        assert_eq!(loaded.compression_threads().unwrap(), 1);
    }

    #[test]
    fn database_defaults_to_a_file_inside_the_data_dir() {
        let config = Config {
            data_dir: Some(PathBuf::from("/srv/pmanager")),
            database: None,
            ..Config::default()
        };
        assert_eq!(
            config.database_path().unwrap(),
            PathBuf::from("/srv/pmanager/pmanager.db")
        );

        let explicit = Config {
            data_dir: Some(PathBuf::from("/srv/pmanager")),
            database: Some(PathBuf::from("/elsewhere/db.sqlite")),
            ..Config::default()
        };
        assert_eq!(
            explicit.database_path().unwrap(),
            PathBuf::from("/elsewhere/db.sqlite")
        );
    }

    #[test]
    fn keep_versions_defaults_and_rejects_zero() {
        assert_eq!(
            Config::default().keep_versions().unwrap(),
            DEFAULT_KEEP_VERSIONS
        );

        let explicit = Config {
            keep_versions: Some(4),
            ..Config::default()
        };
        assert_eq!(explicit.keep_versions().unwrap(), 4);

        // Keeping zero versions would archive every copy of a tool, which is
        // never what the key means — better to complain than to guess.
        let zero = Config {
            keep_versions: Some(0),
            ..Config::default()
        };
        let err = zero.keep_versions().unwrap_err();
        assert!(err.to_string().contains("at least 1"), "{err}");
    }

    #[test]
    fn compression_settings_are_validated() {
        let too_high = Config {
            compression_level: Some(crate::archive::MAX_LEVEL + 1),
            ..Config::default()
        };
        assert!(too_high.compression_level().is_err());

        let no_threads = Config {
            compression_threads: Some(0),
            ..Config::default()
        };
        assert!(no_threads.compression_threads().is_err());

        let valid = Config {
            compression_level: Some(0),
            compression_threads: Some(8),
            ..Config::default()
        };
        assert_eq!(valid.compression_level().unwrap(), 0);
        assert_eq!(valid.compression_threads().unwrap(), 8);
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "dta_dir = \"/typo\"\n").unwrap();

        let err = Config::load(Some(&path)).unwrap_err();
        assert!(matches!(err, Error::ConfigParse { .. }));
    }
}
