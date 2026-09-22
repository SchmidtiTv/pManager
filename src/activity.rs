//! When a project was last worked on.

use std::path::Path;
use std::time::SystemTime;

use anyhow::Context as _;
use chrono::{DateTime, Utc};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use walkdir::WalkDir;

use crate::error::Result;

/// The file a project uses to say which of its contents are not work on it.
pub const IGNORE_FILE: &str = ".pmanagerignore";

/// The newest modification time of any file under `root`, or of `root`
/// itself when it holds no files.
///
/// `.git` is skipped: git touches its own files on fetches and status
/// checks, which is not work on the project. Anything matched by the
/// project's `.pmanagerignore` is skipped too. Symlinks are not followed.
/// An entry that cannot be read is logged and skipped, so one locked
/// folder does not stop a project from being registered.
///
/// This only decides what counts as *activity*. It never decides what is
/// stripped or archived; those come from the strip profiles.
pub fn last_change(root: &Path) -> Result<DateTime<Utc>> {
    let ignored = ignore_rules(root)?;
    let mut newest: Option<DateTime<Utc>> = None;

    let entries = WalkDir::new(root).into_iter().filter_entry(|entry| {
        entry.depth() == 0
            || (entry.file_name() != ".git"
                && !ignored
                    .matched(entry.path(), entry.file_type().is_dir())
                    .is_ignore())
    });

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

/// The gitignore-syntax rules from `<root>/.pmanagerignore`, or a matcher
/// that ignores nothing when the project has no such file.
///
/// A malformed pattern fails the whole call rather than being dropped: the
/// file exists to hold a folder back from counting as activity, and silently
/// ignoring the line that does it would archive a project the user believed
/// was protected.
fn ignore_rules(root: &Path) -> Result<Gitignore> {
    let file = root.join(IGNORE_FILE);
    if !file.is_file() {
        return Ok(Gitignore::empty());
    }

    let mut builder = GitignoreBuilder::new(root);
    if let Some(err) = builder.add(&file) {
        return Err(err).with_context(|| format!("could not read {}", file.display()));
    }

    builder
        .build()
        .with_context(|| format!("{} contains an unusable pattern", file.display()))
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
    fn a_pmanagerignore_pattern_stops_a_file_counting_as_activity() {
        let tmp = tempfile::tempdir().unwrap();
        touch(&tmp.path().join("main.rs"), days_ago(90));
        touch(&tmp.path().join("build").join("app"), days_ago(1));
        touch(&tmp.path().join(super::IGNORE_FILE), days_ago(90));
        std::fs::write(tmp.path().join(super::IGNORE_FILE), "build/\n").unwrap();
        std::fs::File::options()
            .write(true)
            .open(tmp.path().join(super::IGNORE_FILE))
            .unwrap()
            .set_modified(days_ago(90))
            .unwrap();

        let found = last_change(tmp.path()).unwrap();

        assert_eq!(
            (Utc::now() - found).num_days(),
            90,
            "a freshly written file under an ignored folder must not count"
        );
    }

    #[test]
    fn an_unusable_ignore_pattern_fails_loudly_rather_than_being_dropped() {
        let tmp = tempfile::tempdir().unwrap();
        touch(&tmp.path().join("main.rs"), days_ago(90));
        // An unclosed alternate group: `ignore` rejects the line rather than
        // matching nothing with it.
        std::fs::write(tmp.path().join(super::IGNORE_FILE), "build/{\n").unwrap();

        let err = last_change(tmp.path()).unwrap_err();

        assert!(
            format!("{err:#}").contains(super::IGNORE_FILE),
            "the error must name the file the user has to fix, got: {err:#}"
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
