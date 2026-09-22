//! The two questions pManager asks git.
//!
//! Both open the repository at exactly the given folder, never a parent: a
//! subdirectory of a repository is not itself a registered project's repo,
//! and treating it as one would let a sibling's uncommitted work defer this
//! project's archiving.

use std::path::Path;

use anyhow::Context as _;
use git2::{Repository, StatusOptions};

use crate::error::Result;

/// Whether `root` is itself a git repository.
pub fn has_repo(root: &Path) -> bool {
    open(root).is_some()
}

/// Whether the working tree at `root` has uncommitted work.
///
/// A folder that is not a repository is never dirty, which is what puts it on
/// the 120-day archive threshold rather than the 160-day one: the longer hold
/// exists to protect uncommitted work, and there is none to protect.
///
/// Untracked files count. They are work the user has not committed, and
/// archiving is what makes them hard to get back. Ignored files do not: they
/// are the build output and dependency folders this tool exists to remove.
pub fn is_dirty(root: &Path) -> Result<bool> {
    let Some(repository) = open(root) else {
        return Ok(false);
    };

    let mut options = StatusOptions::new();
    options.include_untracked(true).include_ignored(false);

    let statuses = repository
        .statuses(Some(&mut options))
        .with_context(|| format!("could not read the git status of {}", root.display()))?;

    Ok(!statuses.is_empty())
}

fn open(root: &Path) -> Option<Repository> {
    match Repository::open(root) {
        Ok(repository) => Some(repository),
        Err(err) => {
            tracing::debug!("{} is not a git repository: {err}", root.display());
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{has_repo, is_dirty};

    fn init(root: &Path) -> git2::Repository {
        git2::Repository::init(root).unwrap()
    }

    fn commit_everything(repository: &git2::Repository) {
        let mut index = repository.index().unwrap();
        index
            .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();

        let tree = repository.find_tree(index.write_tree().unwrap()).unwrap();
        let who = git2::Signature::now("test", "test@example.com").unwrap();
        repository
            .commit(Some("HEAD"), &who, &who, "seed", &tree, &[])
            .unwrap();
    }

    #[test]
    fn a_folder_without_a_repository_has_none_and_is_never_dirty() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("main.rs"), "fn main() {}").unwrap();

        assert!(!has_repo(tmp.path()));
        assert!(
            !is_dirty(tmp.path()).unwrap(),
            "no repository means no uncommitted work to protect"
        );
    }

    #[test]
    fn a_fully_committed_tree_is_clean() {
        let tmp = tempfile::tempdir().unwrap();
        let repository = init(tmp.path());
        std::fs::write(tmp.path().join("main.rs"), "fn main() {}").unwrap();
        commit_everything(&repository);

        assert!(has_repo(tmp.path()));
        assert!(!is_dirty(tmp.path()).unwrap());
    }

    #[test]
    fn an_uncommitted_edit_makes_the_tree_dirty() {
        let tmp = tempfile::tempdir().unwrap();
        let repository = init(tmp.path());
        std::fs::write(tmp.path().join("main.rs"), "fn main() {}").unwrap();
        commit_everything(&repository);

        std::fs::write(tmp.path().join("main.rs"), "fn main() { todo!() }").unwrap();

        assert!(is_dirty(tmp.path()).unwrap());
    }

    #[test]
    fn an_untracked_file_makes_the_tree_dirty() {
        let tmp = tempfile::tempdir().unwrap();
        let repository = init(tmp.path());
        std::fs::write(tmp.path().join("main.rs"), "fn main() {}").unwrap();
        commit_everything(&repository);

        std::fs::write(tmp.path().join("notes.md"), "unsaved thinking").unwrap();

        assert!(
            is_dirty(tmp.path()).unwrap(),
            "an untracked file is uncommitted work and must defer archiving"
        );
    }

    #[test]
    fn an_ignored_build_folder_does_not_make_the_tree_dirty() {
        let tmp = tempfile::tempdir().unwrap();
        let repository = init(tmp.path());
        std::fs::write(tmp.path().join(".gitignore"), "target/\n").unwrap();
        std::fs::write(tmp.path().join("main.rs"), "fn main() {}").unwrap();
        commit_everything(&repository);

        std::fs::create_dir(tmp.path().join("target")).unwrap();
        std::fs::write(tmp.path().join("target").join("app"), "binary").unwrap();

        assert!(
            !is_dirty(tmp.path()).unwrap(),
            "ignored output is exactly what this tool removes; it is not work"
        );
    }

    #[test]
    fn a_subdirectory_of_a_repository_is_not_itself_a_repository() {
        let tmp = tempfile::tempdir().unwrap();
        init(tmp.path());
        let nested = tmp.path().join("crates").join("inner");
        std::fs::create_dir_all(&nested).unwrap();

        assert!(!has_repo(&nested), "open() must not search parent folders");
    }
}
