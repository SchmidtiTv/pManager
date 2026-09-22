//! Everything a decision needs, read off the untouched tree.
//!
//! Gathering reads disk and git and mutates nothing. That ordering is what
//! keeps a pass honest: a strip that removes a tracked file makes the
//! repository dirty, and because dirtiness was already measured it cannot
//! move the project from the 120-day threshold to the 160-day one within the
//! same pass. The same goes for the idle clock -- `last_change_at` is read
//! before any mutation, so the tool's own writes never reset it.

use chrono::{DateTime, Utc};

use crate::activity;
use crate::error::Result;
use crate::git;
use crate::models::project::Project;
use crate::models::stripping_profile::StrippingProfile;
use crate::stripping::detection;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    pub last_change_at: DateTime<Utc>,
    pub dirty: bool,
    pub matched: Vec<StrippingProfile>,
}

/// Measure `project` against the profiles that are loaded.
///
/// `None` means the folder is no longer there. That is not an error: a
/// project deleted by hand is a row for `pman project cleanup`, not a failed
/// pass.
pub fn gather(project: &Project, profiles: &[StrippingProfile]) -> Result<Option<Facts>> {
    if !project.path.is_dir() {
        return Ok(None);
    }

    Ok(Some(Facts {
        last_change_at: activity::last_change(&project.path)?,
        // Recomputed every pass. The schema comment on `has_git_repo` says
        // why it is never stored: a copy is stale the moment it is written.
        dirty: git::is_dirty(&project.path)?,
        matched: detection::matching(&project.path, profiles),
    }))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use chrono::{Duration, Utc};

    use super::gather;
    use crate::models::project::Project;
    use crate::models::state::ProjectState;
    use crate::models::stripping_profile::{Element, ElementType, StrippingProfile};

    fn project(path: &Path) -> Project {
        Project {
            name: "demo".to_string(),
            path: path.to_path_buf(),
            state: ProjectState::Registered,
            has_git_repo: false,
            last_change_at: Utc::now() - Duration::days(999),
            registered_at: Utc::now() - Duration::days(999),
        }
    }

    fn python() -> StrippingProfile {
        StrippingProfile {
            language: "python".to_string(),
            detection: vec![Element {
                element_type: ElementType::File,
                element: PathBuf::from("pyproject.toml"),
            }],
            strippable: vec![Element {
                element_type: ElementType::Folder,
                element: PathBuf::from(".venv"),
            }],
        }
    }

    #[test]
    fn a_folder_that_is_gone_yields_no_facts_rather_than_an_error() {
        let tmp = tempfile::tempdir().unwrap();

        let gathered = gather(&project(&tmp.path().join("gone")), &[python()]).unwrap();

        assert_eq!(gathered, None);
    }

    #[test]
    fn the_ledgers_stale_last_change_is_replaced_by_what_is_on_disk() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("main.py"), "print()").unwrap();

        let facts = gather(&project(tmp.path()), &[]).unwrap().unwrap();

        assert!(
            (Utc::now() - facts.last_change_at).num_days() == 0,
            "the file was just written; the 999-day-old ledger value must not win"
        );
        assert!(!facts.dirty, "a folder with no repository is never dirty");
        assert!(facts.matched.is_empty());
    }

    #[test]
    fn a_detected_profile_is_carried_into_the_facts() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("pyproject.toml"), "[project]").unwrap();

        let facts = gather(&project(tmp.path()), &[python()]).unwrap().unwrap();

        assert_eq!(facts.matched, vec![python()]);
    }
}
