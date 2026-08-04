//! Measuring a project: when it was last edited, and what of it is
//! regenerable. Nothing here decides anything or writes to disk.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::error::Result;

/// Directories that churn on their own, so an edit inside one is no evidence
/// the project is alive. Not walked when looking for the last edit.
pub const SKIP_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    ".next",
    ".nuxt",
    ".svelte-kit",
    "dist",
    "build",
    "out",
    "target",
    ".venv",
    "venv",
    "__pycache__",
    ".cache",
    ".turbo",
    "coverage",
    "vendor",
];

/// Directories pManager deletes when trimming, and leaves out of archives —
/// both because they rebuild from the files beside them. `.git` is
/// deliberately absent: history is not regenerable.
pub const PURGE_DIRS: &[&str] = &["node_modules"];

/// `package.json`'s name, falling back to the folder name. A broken manifest
/// falls back rather than failing.
pub fn project_name(path: &Path) -> String {
    let folder = || {
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string())
    };

    let Ok(manifest) = std::fs::read_to_string(path.join("package.json")) else {
        return folder();
    };

    serde_json::from_str::<serde_json::Value>(&manifest)
        .ok()
        .as_ref()
        .and_then(|manifest| manifest.get("name"))
        .and_then(serde_json::Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(folder)
}

/// The most recently modified file under a project.
#[derive(Debug, Clone)]
pub struct Edit {
    /// The file itself, for reporting which one it was.
    pub path: PathBuf,
    /// Its modification time.
    pub at: SystemTime,
}

/// A directory that can be deleted and rebuilt.
#[derive(Debug, Clone)]
pub struct Purgeable {
    pub path: PathBuf,
    pub bytes: u64,
    pub files: u64,
}

/// Total size of a set of purgeable directories.
pub fn total_bytes(dirs: &[Purgeable]) -> u64 {
    dirs.iter().map(|dir| dir.bytes).sum()
}

/// The newest file modification under `root`, ignoring `skip` directories.
///
/// `None` means no files outside the skipped ones — an empty checkout, which
/// the policy leaves alone. Unreadable entries are skipped rather than failing
/// the walk.
pub fn last_edit(root: &Path, skip: &[String]) -> Result<Option<Edit>> {
    let mut newest: Option<Edit> = None;

    walk(root, skip, &mut |path, metadata| {
        let Ok(at) = metadata.modified() else {
            return;
        };
        if newest.as_ref().is_none_or(|current| at > current.at) {
            newest = Some(Edit {
                path: path.to_path_buf(),
                at,
            });
        }
    })?;

    Ok(newest)
}

/// Every purge directory under `root`, at any depth — a workspace keeps one
/// per package. Not descended into once found.
pub fn purgeable(root: &Path, purge: &[String]) -> Result<Vec<Purgeable>> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };

        for entry in entries.flatten() {
            // `file_type` does not follow symlinks, so a link to a directory is
            // not walked: pManager must never delete through one.
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_dir() {
                continue;
            }

            let path = entry.path();
            if matches_name(&path, purge) {
                let (files, bytes) = measure(&path);
                found.push(Purgeable { path, bytes, files });
            } else {
                stack.push(path);
            }
        }
    }

    found.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(found)
}

/// File count and total size of a directory tree, ignoring what cannot be read.
pub fn measure(root: &Path) -> (u64, u64) {
    let (mut files, mut bytes) = (0, 0);
    let _ = walk(root, &[], &mut |_, metadata| {
        files += 1;
        bytes += metadata.len();
    });
    (files, bytes)
}

/// How long ago `at` was. Saturates at zero, so a file dated tomorrow reads as
/// "just now" rather than as impossibly old.
pub fn age(at: SystemTime) -> Duration {
    SystemTime::now()
        .duration_since(at)
        .unwrap_or(Duration::ZERO)
}

/// Whether `at` is older than `days`.
pub fn older_than(at: SystemTime, days: u32) -> bool {
    age(at) > Duration::from_secs(u64::from(days) * 86_400)
}

/// Whole days since `at`.
pub fn days_since(at: SystemTime) -> u64 {
    age(at).as_secs() / 86_400
}

/// Unix seconds, as stored in the database. Epoch integers rather than
/// calendar strings: every question asked of them is "how long ago", which
/// keeps a date library out of the tree.
pub fn to_epoch(at: SystemTime) -> i64 {
    match at.duration_since(UNIX_EPOCH) {
        Ok(since) => since.as_secs() as i64,
        // Before 1970: only reachable from a badly wrong clock.
        Err(err) => -(err.duration().as_secs() as i64),
    }
}

/// The point in time a stored epoch value refers to.
pub fn from_epoch(seconds: i64) -> SystemTime {
    if seconds >= 0 {
        UNIX_EPOCH + Duration::from_secs(seconds as u64)
    } else {
        UNIX_EPOCH - Duration::from_secs(seconds.unsigned_abs())
    }
}

/// Now, in the form the database stores.
pub fn now_epoch() -> i64 {
    to_epoch(SystemTime::now())
}

/// A duration in the shape pTag prints it: `3d ago`, `2mo ago`, `1y ago`.
pub fn time_ago(at: SystemTime) -> String {
    let seconds = age(at).as_secs();

    match seconds {
        s if s < 60 => format!("{s}s ago"),
        s if s < 3_600 => format!("{}m ago", s / 60),
        s if s < 86_400 => format!("{}h ago", s / 3_600),
        s if s < 86_400 * 30 => format!("{}d ago", s / 86_400),
        s if s < 86_400 * 365 => format!("{}mo ago", s / (86_400 * 30)),
        s => format!("{}y ago", s / (86_400 * 365)),
    }
}

/// Walk every file under `root`, skipping directories named in `skip`.
/// Iterative, and never follows symlinks, so deep trees and link loops are
/// both safe.
fn walk(
    root: &Path,
    skip: &[String],
    visit: &mut impl FnMut(&Path, &std::fs::Metadata),
) -> Result<()> {
    // The first read is the one that reports a genuinely missing project;
    // failures further down are the unreadable corners described above.
    let mut stack = vec![std::fs::read_dir(root)?];

    while let Some(entries) = stack.pop() {
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let path = entry.path();

            if file_type.is_dir() {
                if matches_name(&path, skip) {
                    tracing::trace!(path = %path.display(), "skipping");
                    continue;
                }
                if let Ok(entries) = std::fs::read_dir(&path) {
                    stack.push(entries);
                }
            } else if file_type.is_file()
                && let Ok(metadata) = entry.metadata()
            {
                visit(&path, &metadata);
            }
        }
    }

    Ok(())
}

/// Whether the final component of `path` is one of `names`.
fn matches_name(path: &Path, names: &[String]) -> bool {
    let Some(name) = path.file_name() else {
        return false;
    };
    names.iter().any(|candidate| name == candidate.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skip_dirs() -> Vec<String> {
        SKIP_DIRS.iter().map(|name| (*name).to_string()).collect()
    }

    fn purge_dirs() -> Vec<String> {
        PURGE_DIRS.iter().map(|name| (*name).to_string()).collect()
    }

    /// Write `contents` to `root/relative`, creating parent directories.
    fn write(root: &Path, relative: &str, contents: &str) -> PathBuf {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
        path
    }

    /// Backdate a file so the walk sees it as old.
    fn backdate(path: &Path, days: u64) {
        let when = SystemTime::now() - Duration::from_secs(days * 86_400);
        let file = std::fs::File::options().write(true).open(path).unwrap();
        file.set_modified(when).unwrap();
    }

    #[test]
    fn a_project_is_named_by_its_manifest_when_it_has_one() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().file_name().unwrap().to_string_lossy();
        assert_eq!(project_name(dir.path()), folder);

        write(
            dir.path(),
            "package.json",
            r#"{"name": "@acme/api", "version": "1.0.0"}"#,
        );
        assert_eq!(project_name(dir.path()), "@acme/api");
    }

    #[test]
    fn a_broken_manifest_falls_back_to_the_folder_name() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();

        for manifest in [
            r#"{"name": }"#,
            r#"{"version": "1.0.0"}"#,
            r#"{"name": ""}"#,
            "",
        ] {
            write(dir.path(), "package.json", manifest);
            assert_eq!(project_name(dir.path()), folder, "for {manifest:?}");
        }
    }

    #[test]
    fn the_newest_edited_file_wins() {
        let dir = tempfile::tempdir().unwrap();
        let old = write(dir.path(), "src/old.ts", "old");
        write(dir.path(), "src/new.ts", "new");
        backdate(&old, 40);

        let edit = last_edit(dir.path(), &skip_dirs()).unwrap().unwrap();
        assert_eq!(edit.path.file_name().unwrap(), "new.ts");
    }

    #[test]
    fn a_dependency_install_does_not_make_a_project_look_alive() {
        let dir = tempfile::tempdir().unwrap();
        let source = write(dir.path(), "src/index.ts", "source");
        backdate(&source, 90);
        // Freshly installed today, and vastly more numerous than the sources.
        write(dir.path(), "node_modules/left-pad/index.js", "dep");
        write(dir.path(), "dist/bundle.js", "built");

        let edit = last_edit(dir.path(), &skip_dirs()).unwrap().unwrap();
        assert_eq!(edit.path.file_name().unwrap(), "index.ts");
        assert!(
            older_than(edit.at, 60),
            "the project should still read stale"
        );
    }

    #[test]
    fn a_project_with_nothing_but_skipped_directories_has_no_last_edit() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "node_modules/left-pad/index.js", "dep");

        assert!(last_edit(dir.path(), &skip_dirs()).unwrap().is_none());
    }

    #[test]
    fn a_missing_project_is_an_error_rather_than_an_empty_walk() {
        let dir = tempfile::tempdir().unwrap();
        assert!(last_edit(&dir.path().join("absent"), &skip_dirs()).is_err());
    }

    #[test]
    fn nested_dependency_trees_are_all_found_and_measured() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "node_modules/a/index.js", "0123456789");
        write(dir.path(), "packages/api/node_modules/b/index.js", "01234");
        write(dir.path(), "src/index.ts", "source");

        let found = purgeable(dir.path(), &purge_dirs()).unwrap();
        assert_eq!(
            found.len(),
            2,
            "a workspace keeps one per package: {found:?}"
        );
        assert_eq!(total_bytes(&found), 15);
        assert_eq!(found.iter().map(|dir| dir.files).sum::<u64>(), 2);
    }

    #[test]
    fn a_dependency_tree_inside_another_is_not_counted_twice() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "node_modules/a/node_modules/b/index.js",
            "nested",
        );

        let found = purgeable(dir.path(), &purge_dirs()).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, dir.path().join("node_modules"));
    }

    #[test]
    fn a_project_without_dependencies_has_nothing_to_purge() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "main.rs", "fn main() {}");

        assert!(purgeable(dir.path(), &purge_dirs()).unwrap().is_empty());
    }

    #[test]
    fn a_future_timestamp_reads_as_new_rather_than_ancient() {
        let tomorrow = SystemTime::now() + Duration::from_secs(86_400);
        assert_eq!(age(tomorrow), Duration::ZERO);
        assert!(!older_than(tomorrow, 1));
        assert_eq!(time_ago(tomorrow), "0s ago");
    }

    #[test]
    fn epoch_values_round_trip() {
        let now = SystemTime::now();
        let seconds = to_epoch(now);

        assert!(
            seconds > 1_700_000_000,
            "expected a plausible epoch, got {seconds}"
        );
        // Whole seconds only, so the round trip is exact to the second.
        assert_eq!(to_epoch(from_epoch(seconds)), seconds);
        assert!(age(from_epoch(now_epoch())) < Duration::from_secs(5));
    }

    #[test]
    fn ages_are_rendered_at_a_sensible_scale() {
        let ago = |seconds: u64| time_ago(SystemTime::now() - Duration::from_secs(seconds));

        assert_eq!(ago(30), "30s ago");
        assert_eq!(ago(3_600 * 5), "5h ago");
        assert_eq!(ago(86_400 * 3), "3d ago");
        assert_eq!(ago(86_400 * 90), "3mo ago");
        assert_eq!(ago(86_400 * 400), "1y ago");
    }
}
