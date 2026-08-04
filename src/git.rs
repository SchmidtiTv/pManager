//! What git knows about a project. File times say when it was last touched;
//! git says whether the work was ever finished, which is what holds an old but
//! unfinished project back from being archived.
//!
//! Shells out to `git` rather than linking a git implementation. Every command
//! here is read-only.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Field separator in git's output. NUL rather than `|`, because commit
/// subjects contain punctuation.
const SEP: char = '\0';

/// How that separator is written in a `--format` string.
///
/// It has to be git's own escape: a process argument cannot itself contain a
/// NUL byte, so spelling it literally makes the command fail to launch.
const SEP_FORMAT: &str = "%x00";

/// What `git log -1` reports about the tip of the current branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub sha: String,
    pub subject: String,
    pub author: String,
    pub at: SystemTime,
}

/// A project's git state, or the absence of one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    /// Whether this is a git repository at all. Everything else is empty when
    /// it is not — a project without git is perfectly normal, not an error.
    pub repo: bool,
    /// Current branch, or `None` on a detached HEAD.
    pub branch: Option<String>,
    /// The most recent commit, absent in a repository with no commits yet.
    pub head: Option<Commit>,
    /// The upstream this branch tracks, e.g. `origin/main`.
    pub upstream: Option<String>,
    /// Commits on this branch that the upstream does not have.
    pub ahead: u32,
    /// Paths with staged changes.
    pub staged: Vec<String>,
    /// Paths modified but not staged.
    pub unstaged: Vec<String>,
    /// Paths git does not track.
    pub untracked: Vec<String>,
}

impl Status {
    /// Whether the working tree holds changes that are not committed.
    pub fn is_dirty(&self) -> bool {
        !self.staged.is_empty() || !self.unstaged.is_empty() || !self.untracked.is_empty()
    }

    /// Whether there are commits the upstream has never seen.
    pub fn has_unpushed(&self) -> bool {
        self.ahead > 0
    }

    /// Total number of changed paths, for a one-line summary.
    pub fn change_count(&self) -> usize {
        self.staged.len() + self.unstaged.len() + self.untracked.len()
    }

    /// Why this project should not be archived yet, if anything. Nothing is
    /// lost either way — the archive holds the working tree and `.git` alike —
    /// but a folder vanishing out from under unfinished work is a surprise.
    pub fn hold_reason(&self) -> Option<String> {
        if !self.repo {
            return None;
        }

        let mut reasons = Vec::new();
        if self.is_dirty() {
            reasons.push(format!("{} uncommitted change(s)", self.change_count()));
        }
        if self.has_unpushed() {
            reasons.push(format!("{} unpushed commit(s)", self.ahead));
        }

        (!reasons.is_empty()).then(|| reasons.join(", "))
    }
}

/// Inspect the repository at `path`. Never fails: a missing `git` or a folder
/// that is not a repository yields `repo: false`, because the thresholds work
/// without git anyway.
pub fn status(path: &Path) -> Status {
    if !is_repo(path) {
        return Status::default();
    }

    let branch = git(path, &["rev-parse", "--abbrev-ref", "HEAD"])
        .filter(|branch| branch != "HEAD")
        .filter(|branch| !branch.is_empty());

    let upstream = git(
        path,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
    );

    // Only meaningful with an upstream to be ahead *of*; a branch that tracks
    // nothing is reported as such rather than as zero commits behind nothing.
    let ahead = upstream
        .as_ref()
        .and_then(|_| git(path, &["rev-list", "--count", "@{u}..HEAD"]))
        .and_then(|count| count.parse().ok())
        .unwrap_or(0);

    let (staged, unstaged, untracked) = changes(path);

    Status {
        repo: true,
        branch,
        head: head_commit(path),
        upstream,
        ahead,
        staged,
        unstaged,
        untracked,
    }
}

/// Whether `path` is inside a git working tree. `--is-inside-work-tree`, not
/// `--git-dir`, which a folder merely sitting below a repository also answers.
fn is_repo(path: &Path) -> bool {
    git(path, &["rev-parse", "--is-inside-work-tree"]).as_deref() == Some("true")
}

/// The commit at the tip of the current branch.
fn head_commit(path: &Path) -> Option<Commit> {
    let format = format!("--format=%h{SEP_FORMAT}%s{SEP_FORMAT}%ct{SEP_FORMAT}%an");
    let line = git(path, &["log", "-1", &format])?;

    let mut fields = line.split(SEP);
    let sha = fields.next()?.to_string();
    let subject = fields.next()?.to_string();
    let seconds: u64 = fields.next()?.parse().ok()?;
    let author = fields.next()?.to_string();

    Some(Commit {
        sha,
        subject,
        author,
        at: UNIX_EPOCH + Duration::from_secs(seconds),
    })
}

/// Staged, unstaged and untracked paths. `-z` because the newline form quotes
/// unusual filenames.
fn changes(path: &Path) -> (Vec<String>, Vec<String>, Vec<String>) {
    let (mut staged, mut unstaged, mut untracked) = (Vec::new(), Vec::new(), Vec::new());

    let Some(output) = git(path, &["status", "--porcelain", "-z"]) else {
        return (staged, unstaged, untracked);
    };

    // Entries are `XY <path>` separated by NUL, and a rename or copy is
    // followed by a second NUL-terminated field holding the old name.
    let mut fields = output.split(SEP).filter(|field| !field.is_empty());

    while let Some(entry) = fields.next() {
        let mut characters = entry.chars();
        let (Some(index), Some(worktree)) = (characters.next(), characters.next()) else {
            continue;
        };
        let file = entry.get(3..).unwrap_or_default().to_string();

        if index == '?' {
            untracked.push(file);
            continue;
        }

        if matches!(index, 'R' | 'C') {
            // Consume the old name so it is not parsed as its own entry.
            fields.next();
        }
        if index != ' ' {
            staged.push(file.clone());
        }
        if worktree != ' ' {
            unstaged.push(file);
        }
    }

    (staged, unstaged, untracked)
}

/// Run a read-only git command and return its trimmed stdout. `None` covers
/// every way it can fail — no git, no repository, no upstream, no commits.
fn git(path: &Path, arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        // A repository whose remote needs credentials must not stop the sweep
        // to ask for them. Nothing here talks to a remote, but this makes that
        // guarantee explicit rather than incidental.
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(arguments)
        .output()
        .inspect_err(|err| tracing::debug!(error = %err, "could not run git"))
        .ok()?;

    if !output.status.success() {
        tracing::trace!(?arguments, "git command failed");
        return None;
    }

    // Only the trailing newline comes off. `--porcelain` status lines begin
    // with a significant space (" M file" means unstaged), so trimming the
    // front would shift every path by one column.
    let text = String::from_utf8_lossy(&output.stdout);
    Some(text.trim_end_matches(['\n', '\r']).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A repository with one commit, or `None` when git is unavailable.
    fn repository() -> Option<tempfile::TempDir> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();

        let run = |arguments: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(path)
                .args(arguments)
                .output()
                .ok()
                .filter(|output| output.status.success())
        };

        run(&["init", "--initial-branch=main"])?;
        run(&["config", "user.email", "test@example.com"])?;
        run(&["config", "user.name", "Test"])?;
        std::fs::write(path.join("README.md"), "hello").unwrap();
        run(&["add", "."])?;
        run(&["commit", "-m", "Add a readme"])?;

        Some(dir)
    }

    #[test]
    fn a_folder_without_git_is_reported_rather_than_failing() {
        let dir = tempfile::tempdir().unwrap();
        let status = status(dir.path());

        assert!(!status.repo);
        assert!(!status.is_dirty());
        assert_eq!(status.hold_reason(), None);
    }

    #[test]
    fn a_missing_folder_is_not_a_repository() {
        assert!(!status(Path::new("/nonexistent/project")).repo);
    }

    #[test]
    fn a_clean_repository_reports_its_branch_and_head() {
        let Some(dir) = repository() else {
            eprintln!("git unavailable; skipping");
            return;
        };
        let status = status(dir.path());

        assert!(status.repo);
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert!(!status.is_dirty(), "{status:?}");
        assert_eq!(status.hold_reason(), None);

        let head = status.head.expect("one commit was made");
        assert_eq!(head.subject, "Add a readme");
        assert_eq!(head.author, "Test");
        assert!(!head.sha.is_empty());
        assert!(crate::scan::age(head.at) < Duration::from_secs(300));
    }

    #[test]
    fn uncommitted_work_holds_a_project_back() {
        let Some(dir) = repository() else {
            eprintln!("git unavailable; skipping");
            return;
        };
        std::fs::write(dir.path().join("README.md"), "edited").unwrap();
        std::fs::write(dir.path().join("scratch.txt"), "new").unwrap();

        let status = status(dir.path());
        assert_eq!(status.unstaged, vec!["README.md"]);
        assert_eq!(status.untracked, vec!["scratch.txt"]);
        assert!(status.is_dirty());
        assert_eq!(
            status.hold_reason().as_deref(),
            Some("2 uncommitted change(s)")
        );
    }

    #[test]
    fn a_branch_without_an_upstream_is_not_counted_as_ahead() {
        let Some(dir) = repository() else {
            eprintln!("git unavailable; skipping");
            return;
        };
        let status = status(dir.path());

        // Nothing to be ahead of, and nothing is lost by archiving it: `.git`
        // travels into the archive with everything else.
        assert_eq!(status.upstream, None);
        assert_eq!(status.ahead, 0);
        assert!(!status.has_unpushed());
    }

    #[test]
    fn a_subject_containing_punctuation_survives_parsing() {
        let Some(dir) = repository() else {
            eprintln!("git unavailable; skipping");
            return;
        };
        std::fs::write(dir.path().join("a.txt"), "a").unwrap();
        for arguments in [
            vec!["add", "."],
            vec!["commit", "-m", "fix: split a|b, keep %s intact"],
        ] {
            Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(&arguments)
                .output()
                .unwrap();
        }

        let head = status(dir.path()).head.expect("a second commit");
        assert_eq!(head.subject, "fix: split a|b, keep %s intact");
        assert_eq!(head.author, "Test");
    }
}
