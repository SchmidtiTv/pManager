//! What `--json` prints.
//!
//! Every command writes exactly one document to stdout and nothing else, so a
//! caller can parse the whole of stdout without looking for where the prose
//! ends. Diagnostics keep going to stderr, and a command that fails writes its
//! error there in the same shape — see [`crate::error::Error::report_json`].
//!
//! The rules the schema keeps to, so that parsing one document teaches you how
//! to parse the rest:
//!
//! * **Keys are always present.** A value that does not apply is `null` or an
//!   empty list, never a missing key. Consumers can index rather than probe.
//! * **Sizes are bytes**, as numbers. `1.2 GiB` is a rendering, not a value.
//! * **Times are unix seconds**, alongside the same `3d ago` phrasing the
//!   tables print — machines read `epoch`, people read `ago`.
//! * **Paths are strings**, spelled the way the platform spells them.
//! * **Nothing found is an empty list**, not a sentence. `list` with no
//!   projects is `"projects": []`, so the caller has no special case.
//! * **A tagged action carries its sentence.** `action` is a stable slug to
//!   branch on; `description` is the wording the human output would have used.
//!
//! The structs here are the schema. They are deliberately not the internal
//! types — `Survey`, `ProjectRecord` and friends are free to change shape
//! without silently rewriting what pManager promises to print.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::time::SystemTime;

use serde::Serialize;

use crate::db::ArchiveRecord;
use crate::error::Result;
use crate::git;
use crate::lifecycle::Action;
use crate::scan;

/// Write one document to stdout, pretty-printed and newline-terminated.
///
/// Rendered to a string first so that a closed pipe surfaces as the
/// [`std::io::Error`] it is — `pmanager list --json | head -1` should end as
/// quietly as it does without the flag.
pub fn emit(document: &impl Serialize) -> Result<()> {
    let rendered = serde_json::to_string_pretty(document)
        .map_err(|err| crate::Error::Other(anyhow::anyhow!("could not render JSON: {err}")))?;

    let mut out = std::io::stdout().lock();
    out.write_all(rendered.as_bytes())?;
    out.write_all(b"\n")?;
    Ok(())
}

/// A path, spelled the way the platform spells it.
pub fn path(path: &Path) -> String {
    path.display().to_string()
}

/// `path` relative to `root` when it sits inside it, absolute when it does not.
fn relative(path: &Path, root: &Path) -> String {
    crate::json::path(path.strip_prefix(root).unwrap_or(path))
}

/// A moment, in both the forms this tool deals in.
#[derive(Debug, Clone, Serialize)]
pub struct Time {
    /// Unix seconds.
    pub epoch: i64,
    /// The phrasing the tables use: `3d ago`.
    pub ago: String,
}

impl Time {
    pub fn at(at: SystemTime) -> Self {
        Self {
            epoch: scan::to_epoch(at),
            ago: scan::time_ago(at),
        }
    }

    /// A timestamp as the database stores it.
    pub fn stored(epoch: i64) -> Self {
        Self::at(scan::from_epoch(epoch))
    }
}

/// The most recent edit under a project, and which file it was.
#[derive(Debug, Clone, Serialize)]
pub struct Edit {
    #[serde(flatten)]
    pub at: Time,
    /// Relative to the project.
    pub path: String,
}

impl Edit {
    pub fn new(edit: &scan::Edit, project: &Path) -> Self {
        Self {
            at: Time::at(edit.at),
            path: relative(&edit.path, project),
        }
    }
}

/// A dependency tree that trimming would delete.
#[derive(Debug, Clone, Serialize)]
pub struct Tree {
    /// Relative to the project — a workspace keeps one per package.
    pub path: String,
    pub bytes: u64,
    pub files: u64,
}

impl Tree {
    pub fn all(dirs: &[scan::Purgeable], project: &Path) -> Vec<Self> {
        dirs.iter()
            .map(|dir| Self {
                path: relative(&dir.path, project),
                bytes: dir.bytes,
                files: dir.files,
            })
            .collect()
    }
}

/// Where an archived project went.
#[derive(Debug, Clone, Serialize)]
pub struct Archive {
    pub path: String,
    /// Size of the archive when it was written.
    pub bytes: u64,
    /// Size of the project before compression.
    pub source_bytes: u64,
    pub file_count: u64,
    /// Whether the archive is still on disk. An archived project *is* its
    /// archive, so a `false` here is the one loss pManager cannot undo.
    pub present: bool,
}

impl Archive {
    pub fn new(record: &ArchiveRecord) -> Self {
        Self {
            path: path(&record.path),
            bytes: record.bytes,
            source_bytes: record.source_bytes,
            file_count: record.file_count,
            present: record.path.is_file(),
        }
    }
}

/// The tip of the current branch.
#[derive(Debug, Clone, Serialize)]
pub struct Commit {
    #[serde(flatten)]
    pub at: Time,
    pub sha: String,
    pub subject: String,
    pub author: String,
}

/// What git makes of a project. `repo: false` and everything else empty is a
/// perfectly ordinary project, not an error.
#[derive(Debug, Clone, Serialize)]
pub struct Git {
    pub repo: bool,
    /// `null` on a detached HEAD.
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub dirty: bool,
    pub staged: Vec<String>,
    pub unstaged: Vec<String>,
    pub untracked: Vec<String>,
    pub head: Option<Commit>,
    /// Why this project would be held back from archiving, if anything.
    pub hold_reason: Option<String>,
}

impl Git {
    pub fn new(status: &git::Status) -> Self {
        Self {
            repo: status.repo,
            branch: status.branch.clone(),
            upstream: status.upstream.clone(),
            ahead: status.ahead,
            dirty: status.is_dirty(),
            staged: status.staged.clone(),
            unstaged: status.unstaged.clone(),
            untracked: status.untracked.clone(),
            head: status.head.as_ref().map(|head| Commit {
                at: Time::at(head.at),
                sha: head.sha.clone(),
                subject: head.subject.clone(),
                author: head.author.clone(),
            }),
            hold_reason: status.hold_reason(),
        }
    }
}

/// What a run would do to a project, or did.
#[derive(Debug, Clone, Serialize)]
pub struct NextRun {
    /// A stable slug: `trim`, `archive`, `held_back`, `revive`, `missing` or
    /// `nothing`. Branch on this rather than on the wording.
    pub action: &'static str,
    /// The sentence the human output prints.
    pub description: String,
    /// Why nothing is happening, or what is holding it back.
    pub reason: Option<String>,
}

impl NextRun {
    pub fn new(action: &Action, description: String) -> Self {
        Self {
            action: action.as_str(),
            description,
            reason: match action {
                Action::Nothing { reason } | Action::HeldBack { reason } => Some(reason.clone()),
                _ => None,
            },
        }
    }
}

// ── One document per command ─────────────────────────────────────────────────

/// `pmanager list`
#[derive(Debug, Serialize)]
pub struct Listing {
    /// Only the projects the filters let through.
    pub projects: Vec<ListedProject>,
    pub summary: ListSummary,
}

#[derive(Debug, Serialize)]
pub struct ListedProject {
    pub name: String,
    pub path: String,
    pub state: &'static str,
    /// Whether the folder is on disk. An archived project's is gone by design.
    pub present: bool,
    /// What the EDITED column shows: the last edit, or for an archived project
    /// when it was archived, since there are no files left to date it by.
    pub edited: Option<Time>,
    pub committed: Option<Time>,
    /// Past the trim threshold, measured the way `run` measures it.
    pub stale: bool,
    pub reclaimable_bytes: u64,
    pub archive: Option<Archive>,
    pub git: Git,
    /// The clauses the NOTES column joins with commas.
    pub notes: Vec<String>,
}

/// The counts under the table. Always over every tracked project, not just the
/// ones a filter let through — `shown` says how many that was.
#[derive(Debug, Serialize)]
pub struct ListSummary {
    pub tracked: usize,
    pub shown: usize,
    pub active: usize,
    pub trimmed: usize,
    pub archived: usize,
    /// Sitting in dependency trees, across everything not archived.
    pub reclaimable_bytes: u64,
    /// Reclaimed so far, as recorded.
    pub freed_bytes: u64,
    /// Tracked projects whose folder is gone; `project prune` clears them out.
    pub missing_folders: usize,
}

/// `pmanager info`
#[derive(Debug, Serialize)]
pub struct Info {
    pub name: String,
    pub path: String,
    pub state: &'static str,
    /// When it entered that state; `null` while it is active.
    pub state_since: Option<Time>,
    pub tracked: Time,
    pub present: bool,
    pub last_edit: Option<Edit>,
    /// Days on the clock every threshold is measured against, which is the
    /// last edit or the restore that reset it, whichever is later.
    pub idle_days: Option<u64>,
    /// What pManager thinks it is built with, and so what it may delete.
    pub toolchains: Vec<&'static str>,
    pub dependencies: Dependencies,
    pub reclaimed_bytes: u64,
    /// When it was last restored — the idle clock runs from here.
    pub restored: Option<Time>,
    pub archive: Option<Archive>,
    pub git: Git,
    pub next_run: NextRun,
}

#[derive(Debug, Serialize)]
pub struct Dependencies {
    pub bytes: u64,
    pub trees: Vec<Tree>,
}

/// `pmanager run`
#[derive(Debug, Serialize)]
pub struct RunReport {
    /// Whether `--yes` was given. Without it nothing was changed and every
    /// entry below is a plan.
    pub applied: bool,
    pub projects: Vec<RunProject>,
    pub totals: RunTotals,
}

#[derive(Debug, Serialize)]
pub struct RunProject {
    pub name: String,
    pub path: String,
    #[serde(flatten)]
    pub action: NextRun,
    /// Whether anything actually changed on disk or in the database. False for
    /// a plan, for a project nothing applies to, and for one a flag excluded.
    pub changed: bool,
    pub last_edit: Option<Time>,
    /// Bytes this project gave back, once something was actually done.
    pub freed_bytes: u64,
    /// The archive written, when one was.
    pub archive: Option<Archive>,
    /// Why this project could not be processed. The others still were.
    pub error: Option<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct RunTotals {
    pub trimmed: u64,
    pub archived: u64,
    pub revived: u64,
    /// Changes described but not made, because `--yes` was absent.
    pub planned: u64,
    pub held_back: u64,
    pub missing: u64,
    pub untouched: u64,
    pub failed: u64,
    pub freed_bytes: u64,
}

/// `pmanager project add`
#[derive(Debug, Serialize)]
pub struct Tracked {
    pub projects: Vec<TrackedProject>,
}

#[derive(Debug, Serialize)]
pub struct TrackedProject {
    pub name: String,
    pub path: String,
    /// False when this project was already tracked and the entry was updated.
    pub added: bool,
    pub last_edit: Option<Time>,
    /// `null` when there are no files to date the project by.
    pub next_run: Option<NextRun>,
}

/// `pmanager project remove`
#[derive(Debug, Serialize)]
pub struct Untracked {
    pub projects: Vec<UntrackedProject>,
}

#[derive(Debug, Serialize)]
pub struct UntrackedProject {
    pub name: String,
    pub path: String,
    /// Untracking an archived project abandons its archive; this is where it
    /// was left, while that is still known.
    pub archive: Option<Archive>,
}

/// `pmanager project prune`
#[derive(Debug, Serialize)]
pub struct Pruned {
    /// Tracked projects whose folder is gone.
    pub projects: Vec<MissingProject>,
    /// Whether they were actually untracked, which takes `--yes`.
    pub untracked: bool,
}

#[derive(Debug, Serialize)]
pub struct MissingProject {
    pub name: String,
    pub path: String,
}

/// `pmanager project set-version`
#[derive(Debug, Serialize)]
pub struct VersionPinned {
    pub project: String,
    /// The vManager-registered folder holding the tool's versions.
    pub folder: String,
    pub version: String,
}

/// `pmanager project unset-version`
#[derive(Debug, Serialize)]
pub struct VersionUnpinned {
    pub project: String,
    pub folder: String,
    /// False when there was no pin for this folder to begin with.
    pub removed: bool,
}

/// `pmanager use`
#[derive(Debug, Serialize)]
pub struct UseReport {
    pub project: String,
    pub switches: Vec<VersionSwitch>,
}

#[derive(Debug, Serialize)]
pub struct VersionSwitch {
    pub folder: String,
    pub version: String,
    /// Whether `vmanager use` succeeded for this pin. The others are still
    /// attempted when one fails.
    pub applied: bool,
    pub error: Option<String>,
}

/// `pmanager restore`
#[derive(Debug, Serialize)]
pub struct Restored {
    pub name: String,
    /// Where the project was registered, which is where it came from.
    pub path: String,
    /// Where it was actually extracted to.
    pub restored_to: String,
    pub archive: RestoredArchive,
    /// Whether the reinstall ran and succeeded. A failure here does not fail
    /// the restore — the project is back either way.
    pub installed: bool,
    /// `null` when no install command applied or `--no-install` was given.
    pub install_command: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct RestoredArchive {
    pub path: String,
    pub bytes: u64,
    /// Whether `--delete-archive` removed it.
    pub deleted: bool,
}

/// `pmanager schedule install`
#[derive(Debug, Serialize)]
pub struct Scheduled {
    pub task: String,
    /// Time of day, as `HH:MM`.
    pub at: String,
    pub executable: String,
    /// The command line handed to Task Scheduler.
    pub command: String,
}

/// `pmanager schedule uninstall`
#[derive(Debug, Serialize)]
pub struct Unscheduled {
    pub task: String,
    pub removed: bool,
}

/// `pmanager schedule status`
#[derive(Debug, Serialize)]
pub struct ScheduleStatus {
    pub task: String,
    /// `schtasks`' own fields, parsed from its `LIST` output. Empty rather
    /// than absent when nothing could be parsed — `output` still has it all.
    pub properties: BTreeMap<String, String>,
    /// Exactly what `schtasks` printed.
    pub output: String,
}

impl ScheduleStatus {
    /// Parse `schtasks /FO LIST` output: `Key: Value`, one per line, blank
    /// lines between tasks. Unparseable lines are left to `output`.
    pub fn new(task: &str, output: String) -> Self {
        let properties = output
            .lines()
            .filter_map(|line| line.split_once(':'))
            .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
            .filter(|(key, _)| !key.is_empty())
            .collect();

        Self {
            task: task.to_string(),
            properties,
            output,
        }
    }
}

/// `pmanager completions`
#[derive(Debug, Serialize)]
pub struct Completions {
    pub shell: String,
    /// The script itself, exactly as it would have been written to stdout.
    pub script: String,
}

/// What a failing command writes to stderr.
#[derive(Debug, Serialize)]
pub struct ErrorReport {
    pub error: ErrorBody,
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    /// The message the human output prints after `error: `.
    pub message: String,
    /// A stable slug for the category — the same categories the exit codes
    /// distinguish.
    pub kind: &'static str,
    pub exit_code: u8,
    /// The `source` chain, outermost first.
    pub causes: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_time_carries_both_forms() {
        let when = SystemTime::now() - Duration::from_secs(3 * 86_400);
        let time = Time::at(when);

        assert_eq!(time.ago, "3d ago");
        assert_eq!(time.epoch, scan::to_epoch(when));
        assert_eq!(Time::stored(time.epoch).epoch, time.epoch);
    }

    #[test]
    fn an_action_is_tagged_and_explained() {
        let held = Action::HeldBack {
            reason: "1 uncommitted change(s)".into(),
        };
        let next = NextRun::new(&held, "nothing — held back".into());

        assert_eq!(next.action, "held_back");
        assert_eq!(next.reason.as_deref(), Some("1 uncommitted change(s)"));

        // An action with nothing to explain still carries the key.
        let next = NextRun::new(&Action::Revive, "mark it active again".into());
        assert_eq!(next.action, "revive");
        assert!(next.reason.is_none());
    }

    #[test]
    fn keys_that_do_not_apply_are_null_rather_than_absent() {
        let document = serde_json::to_value(NextRun::new(
            &Action::Missing,
            "nothing — the folder is gone".into(),
        ))
        .unwrap();

        // A consumer indexes `reason` without first asking whether it is there.
        assert!(document.get("reason").is_some());
        assert!(document["reason"].is_null());
    }

    #[test]
    fn schtasks_output_is_parsed_into_fields() {
        let status = ScheduleStatus::new(
            "pManager daily sweep",
            "TaskName:      \\pManager daily sweep\nStatus:        Ready\nnot a field\n".into(),
        );

        assert_eq!(status.properties["Status"], "Ready");
        // The one line that is not a field is not lost, only unparsed.
        assert!(status.output.contains("not a field"));
        assert_eq!(status.properties.len(), 2);
    }

    #[test]
    fn a_tree_is_reported_relative_to_its_project() {
        let project = Path::new("/code/app");
        let trees = Tree::all(
            &[scan::Purgeable {
                path: project.join("app/build"),
                bytes: 4096,
                files: 12,
            }],
            project,
        );

        assert_eq!(trees[0].path, path(Path::new("app/build")));
        assert_eq!(trees[0].bytes, 4096);
    }
}
