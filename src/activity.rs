//! When a project was last worked on.

use std::path::Path;
use std::time::SystemTime;

use anyhow::Context as _;
use chrono::{DateTime, Utc};
use walkdir::WalkDir;

use crate::error::Result;

/// The newest modification time of any file under `root`, or of `root`
/// itself when it holds no files.
///
/// `.git` is skipped: git touches its own files on fetches and status
/// checks, which is not work on the project. Symlinks are not followed.
/// An entry that cannot be read is logged and skipped, so one locked
/// folder does not stop a project from being registered.
pub fn last_change(root: &Path) -> Result<DateTime<Utc>> {
    let mut newest: Option<DateTime<Utc>> = None;

    let entries = WalkDir::new(root)
        .into_iter()
        .filter_entry(|entry| entry.depth() == 0 || entry.file_name() != ".git");

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                tracing::warn!("skipping unreadable entry: {err}");
                continue;
            }
        };

        if !entry.file_type().is_file() {
            continue;
        }

        match modified(entry.path()) {
            Ok(time) => newest = newest.max(Some(time)),
            Err(err) => tracing::warn!("skipping {}: {err:#}", entry.path().display()),
        }
    }

    match newest {
        Some(newest) => Ok(newest),
        None => modified(root),
    }
}

fn modified(path: &Path) -> Result<DateTime<Utc>> {
    let time: SystemTime = std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .with_context(|| format!("could not read the modification time of {}", path.display()))?;

    Ok(time.into())
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::path::Path;
    use std::time::{Duration, SystemTime};

    use chrono::{DateTime, Utc};

    use super::last_change;

    fn days_ago(days: u64) -> SystemTime {
        SystemTime::now() - Duration::from_secs(days * 24 * 60 * 60)
    }

    fn touch(path: &Path, at: SystemTime) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        File::create(path).unwrap().set_modified(at).unwrap();
    }

    #[test]
    fn the_newest_file_anywhere_in_the_tree_wins() {
        let tmp = tempfile::tempdir().unwrap();
        touch(&tmp.path().join("old.txt"), days_ago(90));
        touch(
            &tmp.path().join("src").join("deep").join("new.rs"),
            days_ago(10),
        );

        let found = last_change(tmp.path()).unwrap();

        assert_eq!(
            (Utc::now() - found).num_days(),
            10,
            "the nested file 10 days old is the newest, got {found}"
        );
    }

    #[test]
    fn git_bookkeeping_is_not_project_activity() {
        let tmp = tempfile::tempdir().unwrap();
        touch(&tmp.path().join("main.rs"), days_ago(90));
        touch(&tmp.path().join(".git").join("FETCH_HEAD"), days_ago(1));

        let found = last_change(tmp.path()).unwrap();

        assert_eq!(
            (Utc::now() - found).num_days(),
            90,
            "a fresh .git/FETCH_HEAD must not make the project look active"
        );
    }

    #[test]
    fn an_empty_project_falls_back_to_the_folder_itself() {
        let tmp = tempfile::tempdir().unwrap();
        let folder: DateTime<Utc> = std::fs::metadata(tmp.path())
            .unwrap()
            .modified()
            .unwrap()
            .into();

        assert_eq!(last_change(tmp.path()).unwrap(), folder);
    }
}
