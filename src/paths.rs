//! Where pManager keeps its database and its strip profiles.

use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::error::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// The SQLite ledger.
    pub db: PathBuf,
    /// Directory holding `profiles.json` and the `*.profile.json` files.
    pub strip_dir: PathBuf,
}

impl Paths {
    /// Resolve against the real user data directory and the running executable.
    pub fn resolve(db: Option<PathBuf>, strip_dir: Option<PathBuf>) -> Result<Self> {
        let data_dir = dirs::data_dir().context("could not determine the user data directory")?;

        let exe = std::env::current_exe().context("could not determine the executable path")?;
        let exe_dir = exe
            .parent()
            .context("executable path has no parent directory")?
            .to_path_buf();

        Ok(Self::resolve_with(db, strip_dir, &data_dir, &exe_dir))
    }

    /// The pure half, so defaults are testable without touching `$HOME`.
    ///
    /// The strip directory sits beside the executable, matching the installed
    /// layout. Under `cargo run` that is `target/debug/strip`, which `cargo
    /// clean` destroys — hence `--strip-dir` and `PMANAGER_STRIP_DIR`.
    pub fn resolve_with(
        db: Option<PathBuf>,
        strip_dir: Option<PathBuf>,
        data_dir: &Path,
        exe_dir: &Path,
    ) -> Self {
        Self {
            db: db.unwrap_or_else(|| data_dir.join("pmanager").join("pmanager.db")),
            strip_dir: strip_dir.unwrap_or_else(|| exe_dir.join("strip")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Paths;
    use std::path::{Path, PathBuf};

    #[test]
    fn defaults_derive_from_data_dir_and_exe_dir() {
        let paths = Paths::resolve_with(None, None, Path::new("/data"), Path::new("/opt/pman"));
        assert_eq!(paths.db, PathBuf::from("/data/pmanager/pmanager.db"));
        assert_eq!(paths.strip_dir, PathBuf::from("/opt/pman/strip"));
    }

    #[test]
    fn explicit_overrides_win() {
        let paths = Paths::resolve_with(
            Some(PathBuf::from("/tmp/my.db")),
            Some(PathBuf::from("/tmp/strip")),
            Path::new("/data"),
            Path::new("/opt/pman"),
        );
        assert_eq!(paths.db, PathBuf::from("/tmp/my.db"));
        assert_eq!(paths.strip_dir, PathBuf::from("/tmp/strip"));
    }

    #[test]
    fn overrides_are_independent() {
        let paths = Paths::resolve_with(
            Some(PathBuf::from("/tmp/my.db")),
            None,
            Path::new("/data"),
            Path::new("/opt/pman"),
        );
        assert_eq!(paths.db, PathBuf::from("/tmp/my.db"));
        assert_eq!(paths.strip_dir, PathBuf::from("/opt/pman/strip"));
    }
}
