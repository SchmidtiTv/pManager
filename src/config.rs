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
use crate::lifecycle::{self, Thresholds};
use crate::scan;

/// Borrow a table of built-in defaults as owned strings.
fn owned(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_string()).collect()
}

/// Qualifier/organization/application triple used to locate platform
/// directories (`~/.config/pmanager` on Linux, `%APPDATA%\pmanager` on
/// Windows, `~/Library/Application Support/pmanager` on macOS).
const APP_NAME: &str = "pmanager";

/// File name looked up inside the platform config directory.
const CONFIG_FILE: &str = "config.toml";

/// Database file name inside the data directory.
const DATABASE_FILE: &str = "pmanager.db";

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Directory where pManager keeps its data. Defaults to the platform data
    /// directory when unset.
    pub data_dir: Option<PathBuf>,

    /// SQLite database file. Defaults to `pmanager.db` inside `data_dir`.
    pub database: Option<PathBuf>,

    /// Days of no edits before a project's dependency trees are deleted.
    /// Defaults to [`lifecycle::DEFAULT_TRIM_DAYS`].
    pub trim_days: Option<u32>,

    /// Days of no edits before a project is compressed and its folder removed.
    /// Defaults to [`lifecycle::DEFAULT_ARCHIVE_DAYS`].
    pub archive_days: Option<u32>,

    /// Directories that pManager deletes when trimming, and leaves out of an
    /// archive. Defaults to [`scan::PURGE_DIRS`].
    pub purge_dirs: Option<Vec<String>>,

    /// Directories not counted when looking for a project's last edit.
    /// Defaults to [`scan::SKIP_DIRS`].
    pub skip_dirs: Option<Vec<String>>,

    /// Where archives are written. Defaults to beside the project.
    pub archive_dir: Option<PathBuf>,

    /// Command `pmanager restore` runs to reinstall dependencies. Defaults to
    /// `npm install` when the restored project has a `package.json`.
    pub install_command: Option<Vec<String>>,

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

    /// How long a project may sit untouched before each step.
    ///
    /// An `archive_days` below `trim_days` is rejected rather than obeyed:
    /// with the archive test coming first, it would mean every project skips
    /// the trim and goes straight to being compressed and deleted. That is a
    /// typo with consequences, so it fails before anything is touched.
    ///
    /// Zero is allowed, and means "as soon as it is a day old" — unusual, but
    /// it is the only way to say it, and someone deliberately writing `0` in a
    /// config file is not making a typo the way `keep_versions = 0` was.
    pub fn thresholds(&self) -> Result<Thresholds> {
        let thresholds = Thresholds {
            trim_days: self.trim_days.unwrap_or(lifecycle::DEFAULT_TRIM_DAYS),
            archive_days: self.archive_days.unwrap_or(lifecycle::DEFAULT_ARCHIVE_DAYS),
        };

        if thresholds.archive_days < thresholds.trim_days {
            return Err(Error::Other(anyhow::anyhow!(
                "`archive_days` ({}) must be at least `trim_days` ({}); \
                 as written, every project would be archived without ever being trimmed",
                thresholds.archive_days,
                thresholds.trim_days
            )));
        }

        Ok(thresholds)
    }

    /// Directories deleted when trimming, and kept out of archives.
    pub fn purge_dirs(&self) -> Vec<String> {
        self.purge_dirs
            .clone()
            .unwrap_or_else(|| owned(scan::PURGE_DIRS))
    }

    /// Directories ignored when dating a project.
    ///
    /// Anything pManager is willing to delete is folded in: a directory that
    /// can be thrown away cannot also be evidence that the project is alive,
    /// and a `purge_dirs` entry that someone forgot to also list here would
    /// quietly keep every project looking fresh.
    pub fn skip_dirs(&self) -> Vec<String> {
        let mut dirs = self
            .skip_dirs
            .clone()
            .unwrap_or_else(|| owned(scan::SKIP_DIRS));

        for dir in self.purge_dirs() {
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
        dirs
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

    /// The command `restore` runs to reinstall dependencies, if any applies.
    ///
    /// With nothing configured this is `npm install`, and only for a project
    /// that has a `package.json` — running a Node install in a Rust project
    /// would be noise at best. Configure `install_command` to say otherwise;
    /// an empty list turns the step off entirely.
    pub fn install_command(&self, project: &Path) -> Option<Vec<String>> {
        match &self.install_command {
            Some(command) if command.is_empty() => None,
            Some(command) => Some(command.clone()),
            None => project
                .join("package.json")
                .is_file()
                .then(|| vec!["npm".to_string(), "install".to_string()]),
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
            "data_dir = '/srv/pmanager'\n\
             trim_days = 30\n\
             archive_days = 90\n\
             purge_dirs = ['node_modules', 'target']\n\
             compression_level = 9\n",
        )
        .unwrap();

        let (loaded, _) = Config::load(Some(&path)).unwrap();
        assert_eq!(loaded.data_dir, Some(PathBuf::from("/srv/pmanager")));
        assert_eq!(
            loaded.thresholds().unwrap(),
            Thresholds {
                trim_days: 30,
                archive_days: 90
            }
        );
        assert_eq!(loaded.purge_dirs(), ["node_modules", "target"]);
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
    fn thresholds_default_to_the_built_in_pair() {
        assert_eq!(
            Config::default().thresholds().unwrap(),
            Thresholds {
                trim_days: lifecycle::DEFAULT_TRIM_DAYS,
                archive_days: lifecycle::DEFAULT_ARCHIVE_DAYS,
            }
        );
    }

    #[test]
    fn an_archive_threshold_below_the_trim_one_is_refused() {
        // Left alone this would archive and delete every stale project without
        // ever trimming one, which is the opposite of what the pair is for.
        let inverted = Config {
            trim_days: Some(90),
            archive_days: Some(30),
            ..Config::default()
        };

        let err = inverted.thresholds().unwrap_err();
        assert!(err.to_string().contains("at least `trim_days`"), "{err}");
    }

    #[test]
    fn anything_purgeable_is_also_ignored_when_dating_a_project() {
        let config = Config {
            purge_dirs: Some(vec!["node_modules".into(), "artifacts".into()]),
            skip_dirs: Some(vec![".git".into()]),
            ..Config::default()
        };

        // `artifacts` was named as deletable but not as ignorable; counting it
        // as a recent edit would keep the project alive forever.
        let skip = config.skip_dirs();
        assert!(skip.contains(&"artifacts".to_string()), "{skip:?}");
        assert!(skip.contains(&"node_modules".to_string()), "{skip:?}");
        assert!(skip.contains(&".git".to_string()), "{skip:?}");
    }

    #[test]
    fn the_install_command_follows_the_project() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::default();

        assert_eq!(config.install_command(dir.path()), None);

        std::fs::write(dir.path().join("package.json"), "{}").unwrap();
        assert_eq!(
            config.install_command(dir.path()),
            Some(vec!["npm".to_string(), "install".to_string()])
        );

        let configured = Config {
            install_command: Some(vec!["pnpm".into(), "install".into()]),
            ..Config::default()
        };
        assert_eq!(
            configured.install_command(dir.path()),
            Some(vec!["pnpm".to_string(), "install".to_string()])
        );

        // An empty list is how the step is turned off.
        let disabled = Config {
            install_command: Some(Vec::new()),
            ..Config::default()
        };
        assert_eq!(disabled.install_command(dir.path()), None);
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
