//! The policy: how a project ages out, and what happens at each step.
//!
//! ```text
//!   active  ──(idle for trim_days)──▶  trimmed  ──(archive_days)──▶  archived
//!      ▲                                   │                             │
//!      └───────────(edited again)──────────┘       `pmanager restore` ────┘
//! ```
//!
//! Trimmed means the dependency trees are gone and `npm install` puts them
//! back; archived means the project is a verified `.pArchived` and the folder
//! is not there any more.
//!
//! Uncommitted or unpushed work holds a project back at the archive step —
//! but only for so long. Past `force_archive_days` it is archived anyway:
//! nothing is lost, since the working tree and `.git` both travel into the
//! archive, and a project nobody has touched in eight months is not work in
//! progress any more.
//!
//! Filesystem only — no database, no printing, no clap — so the rules can be
//! tested directly.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::archive::{self, Summary};
use crate::error::{Error, Result};
use crate::git;
use crate::scan::{self, Edit, Purgeable};

/// Where a project is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum State {
    /// In use, or at least recent enough to be left alone.
    #[default]
    Active,
    /// Dependency trees deleted; the project itself is untouched.
    Trimmed,
    /// Compressed to a `.pArchived`; the folder is gone.
    Archived,
}

impl State {
    /// The spelling stored in the database and printed to the user.
    pub fn as_str(self) -> &'static str {
        match self {
            State::Active => "active",
            State::Trimmed => "trimmed",
            State::Archived => "archived",
        }
    }

    /// Parse the stored spelling. An unrecognised value reads as `active`; the
    /// next run re-assesses from the filesystem regardless.
    pub fn parse(raw: &str) -> Self {
        match raw {
            "trimmed" => State::Trimmed,
            "archived" => State::Archived,
            _ => State::Active,
        }
    }
}

impl std::fmt::Display for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `pad`, not `write_str`: the listing puts this in a fixed-width
        // column, and a manual Display that writes directly ignores `{:<10}`.
        f.pad(self.as_str())
    }
}

/// How long a project may sit untouched before each step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Thresholds {
    /// Days of no edits before the dependency trees are deleted.
    pub trim_days: u32,
    /// Days of no edits before the project is compressed and removed.
    pub archive_days: u32,
    /// Days of no edits after which unfinished work no longer holds the
    /// project back — it is archived as if `--force` had been passed.
    pub force_archive_days: u32,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            trim_days: DEFAULT_TRIM_DAYS,
            archive_days: DEFAULT_ARCHIVE_DAYS,
            force_archive_days: DEFAULT_FORCE_ARCHIVE_DAYS,
        }
    }
}

/// Days of no edits before a project is trimmed. pTag's `--archive-days`.
pub const DEFAULT_TRIM_DAYS: u32 = 60;

/// Days of no edits before a project is archived. pTag's `--deep-archive-days`.
pub const DEFAULT_ARCHIVE_DAYS: u32 = 180;

/// Days of no edits before a project is archived even though it holds
/// uncommitted or unpushed work — eight months, counted as 30-day months.
pub const DEFAULT_FORCE_ARCHIVE_DAYS: u32 = 240;

/// What a project looks like on disk right now.
#[derive(Debug, Clone)]
pub struct Survey {
    pub path: PathBuf,
    /// Whether the folder is still there. A registered project can vanish —
    /// moved, deleted by hand, on a drive that is not plugged in.
    pub present: bool,
    /// The newest edit, ignoring the directories that churn on their own.
    pub last_edit: Option<Edit>,
    /// Dependency trees that could be deleted, with their sizes.
    pub purgeable: Vec<Purgeable>,
    /// What git makes of it.
    pub git: git::Status,
}

impl Survey {
    /// Bytes that trimming would reclaim.
    pub fn purgeable_bytes(&self) -> u64 {
        scan::total_bytes(&self.purgeable)
    }

    /// Whole days since the last edit.
    pub fn idle_days(&self) -> Option<u64> {
        self.last_edit
            .as_ref()
            .map(|edit| scan::days_since(edit.at))
    }

    /// The point every threshold is measured from: the last edit, or the
    /// restore that reset the clock, whichever is later. `None` when there is
    /// nothing to date the project by.
    ///
    /// [`decide`] and anything reporting how far along a project is share this
    /// so a countdown cannot disagree with the decision it counts down to.
    pub fn idle_from(&self, active_since: Option<SystemTime>) -> Option<SystemTime> {
        self.last_edit
            .as_ref()
            .map(|edit| active_since.map_or(edit.at, |seen| edit.at.max(seen)))
    }

    /// Whole days on the clock [`idle_from`](Self::idle_from) starts.
    pub fn idle_days_from(&self, active_since: Option<SystemTime>) -> Option<u64> {
        self.idle_from(active_since).map(scan::days_since)
    }
}

/// Look at a project without judging it. An archived project has no folder by
/// design, so it comes back absent.
pub fn survey(path: &Path, skip: &[String], purge: &[String]) -> Survey {
    if !path.is_dir() {
        return Survey {
            path: path.to_path_buf(),
            present: false,
            last_edit: None,
            purgeable: Vec::new(),
            git: git::Status::default(),
        };
    }

    Survey {
        path: path.to_path_buf(),
        present: true,
        last_edit: scan::last_edit(path, skip).unwrap_or_default(),
        purgeable: scan::purgeable(path, purge).unwrap_or_default(),
        git: git::status(path),
    }
}

/// What should happen to a project.
#[derive(Debug, Clone)]
pub enum Action {
    /// Leave it alone, for the stated reason.
    Nothing { reason: String },
    /// Delete these dependency trees.
    Trim { dirs: Vec<Purgeable> },
    /// Compress the project to `destination` and remove the folder.
    Archive { destination: PathBuf },
    /// Old enough to archive, but holding unfinished work. `--force` overrides,
    /// and so does sitting untouched until `force_archive_days`.
    HeldBack { reason: String },
    /// Edited again since it was trimmed: it is a live project once more.
    Revive,
    /// The folder is gone. `pmanager project prune` clears these out.
    Missing,
}

impl Action {
    /// A stable slug naming this action, as `--json` reports it. Kept beside
    /// the enum so a new variant cannot be added without spelling one.
    pub fn as_str(&self) -> &'static str {
        match self {
            Action::Nothing { .. } => "nothing",
            Action::Trim { .. } => "trim",
            Action::Archive { .. } => "archive",
            Action::HeldBack { .. } => "held_back",
            Action::Revive => "revive",
            Action::Missing => "missing",
        }
    }

    /// Whether carrying this out would change anything on disk or in the
    /// database. Used to decide whether a project is worth a line of output.
    pub fn is_change(&self) -> bool {
        !matches!(self, Action::Nothing { .. })
    }

    /// Whether this action deletes something.
    pub fn is_destructive(&self) -> bool {
        matches!(self, Action::Trim { .. } | Action::Archive { .. })
    }
}

/// The settings a decision is made against.
#[derive(Debug, Clone, Copy, Default)]
pub struct Policy<'a> {
    pub thresholds: Thresholds,
    /// Where archives are written, when not beside the project.
    pub archive_dir: Option<&'a Path>,
    /// Archive projects that would otherwise be held back for unfinished work.
    pub force: bool,
}

/// Decide what to do with a project.
///
/// `active_since` raises the floor under the idle clock. It exists because
/// extraction preserves modification times: a project restored from a two-year
/// old archive arrives with two-year-old files, and without this the next run
/// would archive it straight back.
///
/// Age is checked against the archive threshold first, so a long-abandoned
/// project goes straight there rather than waiting out a trim it does not need.
///
/// Unfinished work holds the archive step back, but not indefinitely: past
/// `force_archive_days` the hold expires and the project is archived anyway.
pub fn decide(
    state: State,
    survey: &Survey,
    active_since: Option<SystemTime>,
    policy: &Policy<'_>,
) -> Action {
    if state == State::Archived {
        return Action::Nothing {
            reason: "already archived".into(),
        };
    }

    if !survey.present {
        return Action::Missing;
    }

    // No files outside the dependency trees means nothing to date the project
    // by. Doing nothing is the only defensible reading: an empty checkout is
    // not the same as an old one.
    let Some(idle_from) = survey.idle_from(active_since) else {
        return Action::Nothing {
            reason: "no files to date it by".into(),
        };
    };

    if scan::older_than(idle_from, policy.thresholds.archive_days) {
        // A hold is a courtesy, not a veto. Once a project has sat untouched
        // for `force_archive_days` the unfinished work is as stale as the rest
        // of it, and the archive keeps both the working tree and `.git`.
        let hold_expired = scan::older_than(idle_from, policy.thresholds.force_archive_days);

        if let Some(reason) = survey.git.hold_reason()
            && !policy.force
            && !hold_expired
        {
            return Action::HeldBack { reason };
        }
        return Action::Archive {
            destination: destination(&survey.path, policy.archive_dir),
        };
    }

    if scan::older_than(idle_from, policy.thresholds.trim_days) {
        if survey.purgeable.is_empty() {
            return Action::Nothing {
                reason: if state == State::Trimmed {
                    "trimmed already".into()
                } else {
                    "nothing to trim".into()
                },
            };
        }
        return Action::Trim {
            dirs: survey.purgeable.clone(),
        };
    }

    // Young enough to be a live project. If it was trimmed once, someone has
    // been working on it since, and the record should say so.
    if state == State::Trimmed {
        return Action::Revive;
    }

    Action::Nothing {
        reason: "active".into(),
    }
}

/// Where a project's archive goes: `<name>.pArchived`, beside the project or in
/// `archive_dir` when one is configured.
pub fn destination(project: &Path, archive_dir: Option<&Path>) -> PathBuf {
    let name = project
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".to_string());

    let parent = archive_dir
        .map(Path::to_path_buf)
        .or_else(|| project.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));

    parent.join(format!("{name}.{}", archive::EXTENSION))
}

/// What trimming a project actually managed to delete.
#[derive(Debug, Default)]
pub struct Trimmed {
    /// Directories removed.
    pub removed: usize,
    /// Bytes those directories held.
    pub freed: u64,
    /// Directories that could not be removed, and why. Usually a file still
    /// open in an editor or a dev server.
    pub failed: Vec<(PathBuf, String)>,
}

/// Delete a project's dependency trees. Every directory is attempted even if
/// an earlier one fails; whatever survives is found again on the next run.
pub fn trim(dirs: &[Purgeable]) -> Trimmed {
    let mut result = Trimmed::default();

    for dir in dirs {
        tracing::info!(path = %dir.path.display(), bytes = dir.bytes, "removing dependency tree");
        match std::fs::remove_dir_all(&dir.path) {
            Ok(()) => {
                result.removed += 1;
                result.freed += dir.bytes;
            }
            Err(err) => {
                tracing::warn!(path = %dir.path.display(), error = %err, "could not remove");
                result.failed.push((dir.path.clone(), err.to_string()));
            }
        }
    }

    result
}

/// Compress a project and, unless told otherwise, delete the folder.
///
/// The order is the safety property: write, verify by reading back, and only
/// then delete. A failure anywhere before the last step leaves the project
/// exactly where it was.
pub fn archive(
    project: &Path,
    destination: &Path,
    options: &archive::Options,
    keep_source: bool,
) -> Result<Summary> {
    let summary = archive::create(project, destination, options)?;
    archive::verify(&summary)?;

    if keep_source {
        return Ok(summary);
    }

    tracing::info!(path = %project.display(), "removing archived project");
    std::fs::remove_dir_all(project).map_err(|err| {
        Error::Archive(format!(
            "archived `{}` to `{}` but could not remove the folder: {err}",
            project.display(),
            destination.display()
        ))
    })?;

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn skip() -> Vec<String> {
        scan::skip_dirs()
    }

    /// These tests build Node-shaped projects; which directories a real run
    /// would pick is [`scan::regenerable_dirs`]'s business, tested there.
    fn purge() -> Vec<String> {
        vec!["node_modules".to_string()]
    }

    /// A project whose source file was last edited `days` ago.
    fn project(root: &Path, name: &str, days: u64) -> PathBuf {
        let path = root.join(name);
        std::fs::create_dir_all(path.join("src")).unwrap();

        let source = path.join("src/index.ts");
        std::fs::write(&source, "export const answer = 42;").unwrap();
        let when = SystemTime::now() - Duration::from_secs(days * 86_400);
        std::fs::File::options()
            .write(true)
            .open(&source)
            .unwrap()
            .set_modified(when)
            .unwrap();

        path
    }

    /// Give a project a dependency tree, freshly installed.
    fn install(project: &Path, bytes: usize) {
        let dir = project.join("node_modules/left-pad");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("index.js"), "x".repeat(bytes)).unwrap();
    }

    fn decide_for(path: &Path, state: State) -> Action {
        let survey = survey(path, &skip(), &purge());
        decide(state, &survey, None, &Policy::default())
    }

    fn days_ago(days: u64) -> SystemTime {
        SystemTime::now() - Duration::from_secs(days * 86_400)
    }

    #[test]
    fn a_project_edited_today_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = project(dir.path(), "api", 1);
        install(&path, 100);

        assert!(matches!(
            decide_for(&path, State::Active),
            Action::Nothing { .. }
        ));
    }

    #[test]
    fn a_stale_project_has_its_dependencies_trimmed() {
        let dir = tempfile::tempdir().unwrap();
        let path = project(dir.path(), "api", 90);
        install(&path, 4_096);

        let Action::Trim { dirs } = decide_for(&path, State::Active) else {
            panic!("90 days idle is past the 60 day trim threshold");
        };
        assert_eq!(dirs.len(), 1);
        assert_eq!(scan::total_bytes(&dirs), 4_096);
    }

    #[test]
    fn reinstalling_dependencies_does_not_reset_the_clock() {
        let dir = tempfile::tempdir().unwrap();
        let path = project(dir.path(), "api", 90);
        // Installed just now — but nobody edited the project to do it.
        install(&path, 4_096);

        assert!(
            matches!(decide_for(&path, State::Trimmed), Action::Trim { .. }),
            "node_modules is not evidence of work"
        );
    }

    #[test]
    fn an_already_trimmed_project_has_nothing_left_to_do() {
        let dir = tempfile::tempdir().unwrap();
        let path = project(dir.path(), "api", 90);

        assert!(matches!(
            decide_for(&path, State::Trimmed),
            Action::Nothing { .. }
        ));
    }

    #[test]
    fn a_long_abandoned_project_is_archived() {
        let dir = tempfile::tempdir().unwrap();
        let path = project(dir.path(), "api", 400);

        let Action::Archive { destination } = decide_for(&path, State::Active) else {
            panic!("400 days idle is past the 180 day archive threshold");
        };
        assert_eq!(destination, dir.path().join("api.pArchived"));
    }

    #[test]
    fn archiving_does_not_wait_for_a_trim_that_is_no_longer_needed() {
        let dir = tempfile::tempdir().unwrap();
        let path = project(dir.path(), "api", 400);
        install(&path, 100);

        // pTag made a project pass through `archived` before it could be
        // deep-archived. There is nothing to gain from the extra run: the
        // dependency tree is excluded from the archive either way.
        assert!(matches!(
            decide_for(&path, State::Active),
            Action::Archive { .. }
        ));
    }

    #[test]
    fn an_archived_project_is_not_re_examined() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gone");

        // No folder, and none expected — that is what archived means.
        assert!(matches!(
            decide_for(&path, State::Archived),
            Action::Nothing { .. }
        ));
    }

    #[test]
    fn a_vanished_project_is_reported_rather_than_acted_on() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            decide_for(&dir.path().join("moved-away"), State::Active),
            Action::Missing
        ));
    }

    #[test]
    fn a_project_worked_on_again_comes_back_to_life() {
        let dir = tempfile::tempdir().unwrap();
        let path = project(dir.path(), "api", 2);

        assert!(matches!(decide_for(&path, State::Trimmed), Action::Revive));
    }

    #[test]
    fn a_project_with_no_datable_files_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty");
        std::fs::create_dir_all(path.join("node_modules")).unwrap();
        std::fs::write(path.join("node_modules/x.js"), "dep").unwrap();

        assert!(matches!(
            decide_for(&path, State::Active),
            Action::Nothing { .. }
        ));
    }

    #[test]
    fn unfinished_work_holds_an_old_project_back() {
        let dir = tempfile::tempdir().unwrap();
        // Past the archive threshold, but short of the eight months at which
        // the hold lapses on its own.
        let path = project(dir.path(), "api", 200);

        let mut survey = survey(&path, &skip(), &purge());
        survey.git = git::Status {
            repo: true,
            untracked: vec!["notes.md".into()],
            ..Default::default()
        };

        let action = decide(State::Active, &survey, None, &Policy::default());
        let Action::HeldBack { reason } = action else {
            panic!("an untracked file should hold the project back, got {action:?}");
        };
        assert!(reason.contains("uncommitted"), "{reason}");

        // …until it is overridden.
        let forced = Policy {
            force: true,
            ..Default::default()
        };
        assert!(matches!(
            decide(State::Active, &survey, None, &forced),
            Action::Archive { .. }
        ));
    }

    #[test]
    fn a_hold_lapses_once_the_project_is_old_enough() {
        let dir = tempfile::tempdir().unwrap();
        // Eight months and a day, with work that was never committed.
        let path = project(dir.path(), "api", 241);

        let mut survey = survey(&path, &skip(), &purge());
        survey.git = git::Status {
            repo: true,
            unstaged: vec!["src/index.ts".into()],
            ahead: 3,
            upstream: Some("origin/main".into()),
            ..Default::default()
        };

        let action = decide(State::Active, &survey, None, &Policy::default());
        assert!(
            matches!(action, Action::Archive { .. }),
            "past force_archive_days the unfinished work is as stale as the \
             rest of the project, got {action:?}"
        );
    }

    #[test]
    fn a_hold_still_applies_between_the_two_archive_thresholds() {
        let dir = tempfile::tempdir().unwrap();
        // Past archive_days (180) but not yet force_archive_days (240).
        let path = project(dir.path(), "api", 200);

        let mut survey = survey(&path, &skip(), &purge());
        survey.git = git::Status {
            repo: true,
            untracked: vec!["notes.md".into()],
            ..Default::default()
        };

        let action = decide(State::Active, &survey, None, &Policy::default());
        assert!(matches!(action, Action::HeldBack { .. }), "got {action:?}");
    }

    #[test]
    fn a_restore_delays_the_lapse_along_with_everything_else() {
        let dir = tempfile::tempdir().unwrap();
        let path = project(dir.path(), "api", 400);

        let mut survey = survey(&path, &skip(), &purge());
        survey.git = git::Status {
            repo: true,
            untracked: vec!["notes.md".into()],
            ..Default::default()
        };

        // Restored 200 days ago: old enough to archive, but the eight months
        // run from the restore, so the hold is still in force.
        let action = decide(
            State::Active,
            &survey,
            Some(days_ago(200)),
            &Policy::default(),
        );
        assert!(matches!(action, Action::HeldBack { .. }), "got {action:?}");

        let action = decide(
            State::Active,
            &survey,
            Some(days_ago(250)),
            &Policy::default(),
        );
        assert!(matches!(action, Action::Archive { .. }), "got {action:?}");
    }

    #[test]
    fn the_lapse_threshold_can_be_moved() {
        let dir = tempfile::tempdir().unwrap();
        let path = project(dir.path(), "api", 200);

        let mut survey = survey(&path, &skip(), &purge());
        survey.git = git::Status {
            repo: true,
            untracked: vec!["notes.md".into()],
            ..Default::default()
        };

        let impatient = Policy {
            thresholds: Thresholds {
                force_archive_days: 190,
                ..Thresholds::default()
            },
            ..Default::default()
        };
        assert!(matches!(
            decide(State::Active, &survey, None, &impatient),
            Action::Archive { .. }
        ));

        let patient = Policy {
            thresholds: Thresholds {
                force_archive_days: 3_650,
                ..Thresholds::default()
            },
            ..Default::default()
        };
        assert!(matches!(
            decide(State::Active, &survey, None, &patient),
            Action::HeldBack { .. }
        ));
    }

    #[test]
    fn the_idle_clock_reported_is_the_one_decisions_are_made_on() {
        let dir = tempfile::tempdir().unwrap();
        let path = project(dir.path(), "api", 400);
        let survey = survey(&path, &skip(), &purge());

        assert_eq!(survey.idle_days_from(None), Some(400));
        // A restore raises the floor…
        assert_eq!(survey.idle_days_from(Some(days_ago(30))), Some(30));
        // …but never makes a project look older than its files do.
        assert_eq!(survey.idle_days_from(Some(days_ago(900))), Some(400));
    }

    #[test]
    fn an_archive_directory_redirects_the_destination() {
        let dir = tempfile::tempdir().unwrap();
        let path = project(dir.path(), "api", 400);
        let vault = dir.path().join("vault");

        let survey = survey(&path, &skip(), &purge());
        let policy = Policy {
            archive_dir: Some(&vault),
            ..Default::default()
        };
        let Action::Archive { destination } = decide(State::Active, &survey, None, &policy) else {
            panic!("expected an archive");
        };
        assert_eq!(destination, vault.join("api.pArchived"));
    }

    #[test]
    fn a_just_restored_project_is_not_archived_straight_back() {
        let dir = tempfile::tempdir().unwrap();
        // Extraction preserves modification times, so a restored project
        // arrives with files as old as the day it was archived.
        let path = project(dir.path(), "api", 400);

        let survey = survey(&path, &skip(), &purge());
        assert!(
            matches!(
                decide(State::Active, &survey, None, &Policy::default()),
                Action::Archive { .. }
            ),
            "without the restore time it would go straight back"
        );

        let action = decide(
            State::Active,
            &survey,
            Some(days_ago(1)),
            &Policy::default(),
        );
        assert!(matches!(action, Action::Nothing { .. }), "got {action:?}");
    }

    #[test]
    fn a_restore_only_delays_the_clock_it_does_not_stop_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = project(dir.path(), "api", 400);
        install(&path, 512);

        // Restored 90 days ago and untouched since: past the trim threshold
        // again, but not yet back to the archive one.
        let survey = survey(&path, &skip(), &purge());
        let action = decide(
            State::Active,
            &survey,
            Some(days_ago(90)),
            &Policy::default(),
        );
        assert!(matches!(action, Action::Trim { .. }), "got {action:?}");

        let action = decide(
            State::Active,
            &survey,
            Some(days_ago(200)),
            &Policy::default(),
        );
        assert!(matches!(action, Action::Archive { .. }), "got {action:?}");
    }

    #[test]
    fn an_edit_after_a_restore_still_counts() {
        let dir = tempfile::tempdir().unwrap();
        let path = project(dir.path(), "api", 2);

        // The floor never makes a project look older than its files do.
        let survey = survey(&path, &skip(), &purge());
        let action = decide(
            State::Active,
            &survey,
            Some(days_ago(400)),
            &Policy::default(),
        );
        assert!(matches!(action, Action::Nothing { .. }), "got {action:?}");
    }

    #[test]
    fn trimming_deletes_the_dependency_trees_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let path = project(dir.path(), "api", 90);
        install(&path, 2_048);

        let dirs = scan::purgeable(&path, &purge()).unwrap();
        let result = trim(&dirs);

        assert_eq!(result.removed, 1);
        assert_eq!(result.freed, 2_048);
        assert!(result.failed.is_empty());
        assert!(!path.join("node_modules").exists());
        assert!(path.join("src/index.ts").exists(), "the project stays");
    }

    #[test]
    fn archiving_writes_verifies_and_then_removes() {
        let dir = tempfile::tempdir().unwrap();
        let path = project(dir.path(), "api", 400);
        install(&path, 8_192);
        std::fs::create_dir_all(path.join(".git")).unwrap();
        std::fs::write(path.join(".git/HEAD"), "ref: refs/heads/main").unwrap();

        let destination = destination(&path, None);
        let options = archive::Options {
            exclude: purge(),
            ..Default::default()
        };
        let summary = archive(&path, &destination, &options, false).unwrap();

        assert!(destination.exists());
        assert!(!path.exists(), "the folder should be gone once verified");

        // History is not regenerable, so it travels with the archive; the
        // dependency tree is, so it does not.
        let stored = sevenz_rust2::Archive::open(&destination).unwrap();
        let names: Vec<&str> = stored.files.iter().map(|file| file.name.as_str()).collect();
        assert!(
            names.iter().any(|name| name.contains("HEAD")),
            "expected .git in {names:?}"
        );
        assert!(
            !names.iter().any(|name| name.contains("left-pad")),
            "expected no node_modules in {names:?}"
        );
        assert_eq!(
            summary.file_count,
            stored.files.iter().filter(|f| !f.is_directory).count() as u64
        );
    }

    #[test]
    fn keeping_the_source_leaves_the_project_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = project(dir.path(), "api", 400);

        let destination = destination(&path, None);
        archive(&path, &destination, &archive::Options::default(), true).unwrap();

        assert!(destination.exists());
        assert!(path.exists());
    }

    #[test]
    fn states_round_trip_through_their_stored_spelling() {
        for state in [State::Active, State::Trimmed, State::Archived] {
            assert_eq!(State::parse(state.as_str()), state);
        }
        assert_eq!(State::parse("something else"), State::Active);
    }
}
