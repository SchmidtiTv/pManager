//! Where pManager keeps its database and its strip profiles.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

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

/// The form a project path is stored in, so that one directory is always
/// the same bytes in the ledger.
///
/// Canonicalising settles case on Windows, `\` against `/`, `.`/`..`,
/// symlinks and trailing separators. The directory must exist.
pub fn normalise_project_path(path: &Path) -> Result<PathBuf> {
    let canonical = std::fs::canonicalize(path)
        .with_context(|| format!("could not resolve project path {}", path.display()))?;

    if !canonical.is_dir() {
        bail!("{} is not a directory", canonical.display());
    }

    Ok(without_verbatim_prefix(canonical))
}

/// Like [`normalise_project_path`], but for a project that may already be
/// gone from disk, e.g. deleted by hand after registration. A missing path
/// can only be made absolute, not canonical, so it matches the stored path
/// only when spelled the way it was registered.
pub fn normalise_registered_path(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        return normalise_project_path(path);
    }

    let absolute = std::path::absolute(path)
        .with_context(|| format!("could not resolve project path {}", path.display()))?;

    // Rebuilding from components drops `.` segments and trailing separators.
    Ok(without_verbatim_prefix(absolute.components().collect()))
}

/// `canonicalize` on Windows returns `\\?\C:\...` and `\\?\UNC\...`
/// verbatim paths. Users type, and `list` should show, the ordinary form.
fn without_verbatim_prefix(path: PathBuf) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path;
    };

    if let Some(share) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{share}"));
    }

    match text.strip_prefix(r"\\?\") {
        Some(disk) if disk.as_bytes().get(1) == Some(&b':') => PathBuf::from(disk),
        _ => path,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Paths, normalise_project_path, normalise_registered_path, without_verbatim_prefix,
    };
    use std::path::{Path, PathBuf};

    #[test]
    fn two_spellings_of_one_directory_normalise_to_the_same_path() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("project");
        std::fs::create_dir(&project).unwrap();

        let plain = normalise_project_path(&project).unwrap();
        let roundabout =
            normalise_project_path(&project.join("..").join("project").join(".")).unwrap();

        assert_eq!(plain, roundabout, "one directory must be stored one way");
        assert!(plain.is_absolute(), "stored paths are absolute");
    }

    #[test]
    fn a_missing_project_path_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();

        let err = normalise_project_path(&tmp.path().join("nope")).unwrap_err();

        assert!(format!("{err:#}").contains("nope"), "got: {err:#}");
    }

    #[test]
    fn a_file_is_not_a_project() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("file.txt");
        std::fs::write(&file, "").unwrap();

        let err = normalise_project_path(&file).unwrap_err();

        assert!(
            format!("{err:#}").contains("is not a directory"),
            "got: {err:#}"
        );
    }

    #[test]
    fn an_existing_registered_path_is_canonicalised_like_a_new_one() {
        let tmp = tempfile::tempdir().unwrap();

        assert_eq!(
            normalise_registered_path(tmp.path()).unwrap(),
            normalise_project_path(tmp.path()).unwrap()
        );
    }

    #[test]
    fn a_vanished_registered_path_is_still_made_absolute() {
        let tmp = tempfile::tempdir().unwrap();
        let canonical_tmp = normalise_project_path(tmp.path()).unwrap();
        let gone = canonical_tmp.join("gone");

        let with_trailing_dot = normalise_registered_path(&gone.join(".")).unwrap();

        assert_eq!(with_trailing_dot, gone, "`.` and trailing separators go");
    }

    #[test]
    fn windows_verbatim_prefixes_are_removed() {
        assert_eq!(
            without_verbatim_prefix(PathBuf::from(r"\\?\C:\dev\x")),
            PathBuf::from(r"C:\dev\x")
        );
        assert_eq!(
            without_verbatim_prefix(PathBuf::from(r"\\?\UNC\server\share\x")),
            PathBuf::from(r"\\server\share\x")
        );
        assert_eq!(
            without_verbatim_prefix(PathBuf::from("/home/me/x")),
            PathBuf::from("/home/me/x"),
            "ordinary paths are left alone"
        );
    }

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
