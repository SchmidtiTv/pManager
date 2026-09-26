//! When a project was last worked on.

use std::path::Path;
use std::time::SystemTime;

use anyhow::Context as _;
use chrono::{DateTime, Utc};
use walkdir::WalkDir;

use crate::activity::ignore_file::IgnoreFile;
use crate::error::Result;

/// The newest modification time of any file under `root`, or of `root`
/// itself when it holds no files.
///
/// Two things are skipped. `.git`, because git touches its own files on
/// fetches and status checks, which is not work on the project; and anything
/// the project's [`IgnoreFile`] matches. Symlinks are not followed. An entry
/// that cannot be read is logged and skipped, so one locked folder does not
/// stop a project from being registered.
///
/// This only decides what counts as *activity*. It never decides what is
/// stripped or archived; those come from the strip profiles.
pub fn last_change(root: &Path) -> Result<DateTime<Utc>> {
    let ignored = IgnoreFile::load(root)?;
    let mut newest: Option<DateTime<Utc>> = None;

    // `filter_entry` prunes rather than filters: an ignored folder is never
    // descended into, so ignoring `node_modules` costs one check instead of
    // walking everything inside it. The depth check keeps `root` itself in the
    // walk, which a pattern matching its folder name would otherwise exclude.
    let entries = WalkDir::new(root).into_iter().filter_entry(|entry| {
        entry.depth() == 0
            || (entry.file_name() != ".git"
                && !ignored.ignores(entry.path(), entry.file_type().is_dir()))
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
    use crate::activity::ignore_file::IgnoreFile;

    fn days_ago(days: u64) -> SystemTime {
        SystemTime::now() - Duration::from_secs(days * 24 * 60 * 60)
    }

    fn touch(path: &Path, at: SystemTime) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        File::create(path).unwrap().set_modified(at).unwrap();
    }

    fn write_rules(root: &Path, contents: &str, at: SystemTime) {
        let file = IgnoreFile::path(root);
        std::fs::write(&file, contents).unwrap();
        File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(at)
            .unwrap();
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
        write_rules(tmp.path(), "build/\n", days_ago(90));

        let found = last_change(tmp.path()).unwrap();

        assert_eq!(
            (Utc::now() - found).num_days(),
            90,
            "a freshly written file under an ignored folder must not count"
        );
    }

    #[test]
    fn an_ignored_folder_is_never_descended_into() {
        let tmp = tempfile::tempdir().unwrap();
        touch(&tmp.path().join("main.rs"), days_ago(90));
        // Nothing below an ignored folder counts, however deep it sits and
        // whatever its own name would have matched.
        touch(
            &tmp.path().join("build").join("deep").join("main.rs"),
            days_ago(1),
        );
        write_rules(tmp.path(), "build/\n", days_ago(90));

        let found = last_change(tmp.path()).unwrap();

        assert_eq!((Utc::now() - found).num_days(), 90);
    }

    #[test]
    fn a_negated_pattern_keeps_its_file_counting() {
        let tmp = tempfile::tempdir().unwrap();
        touch(&tmp.path().join("main.rs"), days_ago(90));
        touch(&tmp.path().join("debug.log"), days_ago(50));
        touch(&tmp.path().join("keep.log"), days_ago(10));
        write_rules(tmp.path(), "*.log\n!keep.log\n", days_ago(90));

        let found = last_change(tmp.path()).unwrap();

        assert_eq!(
            (Utc::now() - found).num_days(),
            10,
            "keep.log was taken back out of the ignore set, so it still counts"
        );
    }

    #[test]
    fn an_unusable_ignore_pattern_fails_loudly_rather_than_being_dropped() {
        let tmp = tempfile::tempdir().unwrap();
        touch(&tmp.path().join("main.rs"), days_ago(90));
        write_rules(tmp.path(), "build/{\n", days_ago(90));

        let err = last_change(tmp.path()).unwrap_err();

        assert!(
            format!("{err:#}").contains(IgnoreFile::NAME),
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
