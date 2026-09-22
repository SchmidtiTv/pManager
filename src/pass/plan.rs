//! What is due for a project. Pure: no disk, no clock, no database.
//!
//! This is where the product's thresholds live, and nowhere else. Because
//! [`decide`] takes `now` and already-gathered [`Facts`], the whole table
//! below is covered by unit tests with no temporary directory and no
//! back-dated mtimes.

use chrono::{DateTime, Utc};

use crate::models::project::Project;
use crate::models::state::ProjectState;
use crate::pass::facts::Facts;

const STRIP_AFTER_DAYS: i64 = 60;
const ARCHIVE_AFTER_DAYS: i64 = 120;
const ARCHIVE_DIRTY_AFTER_DAYS: i64 = 160;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Nothing { reason: Reason },
    Strip,
    Archive,
    StripAndArchive,
}

/// Why nothing is due. Without it a project still in use and a project idle
/// for 90 days that no profile recognises look identical in the report, and
/// only the second is worth acting on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    Active,
    NoProfile,
    AlreadyArchived,
    Missing,
}

impl Action {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Nothing { .. } => "none",
            Self::Strip => "strip",
            Self::Archive => "archive",
            Self::StripAndArchive => "strip+archive",
        }
    }

    pub fn is_nothing(&self) -> bool {
        matches!(self, Self::Nothing { .. })
    }
}

impl Reason {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::NoProfile => "no profile",
            Self::AlreadyArchived => "already archived",
            Self::Missing => "missing",
        }
    }
}

/// `facts` is `None` for a project whose folder is no longer on disk.
pub fn decide(project: &Project, facts: Option<&Facts>, now: DateTime<Utc>) -> Action {
    if matches!(project.state, ProjectState::Archived { .. }) {
        return Action::Nothing {
            reason: Reason::AlreadyArchived,
        };
    }

    // Nothing can be measured about a folder that is gone, so nothing is
    // guessed either. `pman project cleanup` owns removing the dead row.
    let Some(facts) = facts else {
        return Action::Nothing {
            reason: Reason::Missing,
        };
    };

    let idle = (now - facts.last_change_at).num_days();
    let matched = !facts.matched.is_empty();
    let registered = matches!(project.state, ProjectState::Registered);

    // A dirty tree defers archiving; it never blocks it, and it never holds
    // back a strip, because stripped files are regenerable.
    let archive_at = if facts.dirty {
        ARCHIVE_DIRTY_AFTER_DAYS
    } else {
        ARCHIVE_AFTER_DAYS
    };

    if idle >= archive_at {
        // Stripping first keeps package folders out of the archive. Getting
        // them back is a reinstall, not a restore.
        return if matched && registered {
            Action::StripAndArchive
        } else {
            Action::Archive
        };
    }

    if registered && idle >= STRIP_AFTER_DAYS {
        return if matched {
            Action::Strip
        } else {
            Action::Nothing {
                reason: Reason::NoProfile,
            }
        };
    }

    // Everything left is a project nothing is due for: still in use, or
    // already stripped and not yet old enough to archive.
    Action::Nothing {
        reason: Reason::Active,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::{DateTime, Duration, TimeZone as _, Utc};

    use super::{Action, Reason, decide};
    use crate::models::project::Project;
    use crate::models::state::ProjectState;
    use crate::models::stripping_profile::{Element, ElementType, StrippingProfile};
    use crate::pass::facts::Facts;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap()
    }

    fn project(state: ProjectState) -> Project {
        Project {
            name: "demo".to_string(),
            path: PathBuf::from("C:/dev/demo"),
            state,
            has_git_repo: true,
            last_change_at: now(),
            registered_at: now() - Duration::days(400),
        }
    }

    fn archived() -> Project {
        project(ProjectState::Archived {
            archive_path: PathBuf::from("C:/dev/demo.pproject"),
        })
    }

    fn profile() -> StrippingProfile {
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

    fn facts(idle_days: i64, dirty: bool, matched: bool) -> Facts {
        Facts {
            last_change_at: now() - Duration::days(idle_days),
            dirty,
            matched: if matched { vec![profile()] } else { Vec::new() },
        }
    }

    fn nothing(reason: Reason) -> Action {
        Action::Nothing { reason }
    }

    #[test]
    fn an_archived_project_is_never_touched_again() {
        assert_eq!(
            decide(&archived(), Some(&facts(9_999, false, true)), now()),
            nothing(Reason::AlreadyArchived),
            "the folder is gone and the archive is not a project to age out"
        );
    }

    #[test]
    fn an_archived_project_is_already_archived_not_missing() {
        // Its folder is gone by design, so "missing" would be alarming noise.
        assert_eq!(
            decide(&archived(), None, now()),
            nothing(Reason::AlreadyArchived)
        );
    }

    #[test]
    fn a_folder_that_is_gone_is_reported_not_repaired() {
        assert_eq!(
            decide(&project(ProjectState::Registered), None, now()),
            nothing(Reason::Missing)
        );
    }

    #[test]
    fn a_project_below_the_strip_threshold_is_active() {
        assert_eq!(
            decide(
                &project(ProjectState::Registered),
                Some(&facts(59, false, true)),
                now()
            ),
            nothing(Reason::Active),
            "59 days is one day short of the 60-day strip threshold"
        );
    }

    #[test]
    fn sixty_idle_days_with_a_matching_profile_strips() {
        assert_eq!(
            decide(
                &project(ProjectState::Registered),
                Some(&facts(60, false, true)),
                now()
            ),
            Action::Strip
        );
    }

    #[test]
    fn sixty_idle_days_with_no_matching_profile_says_so() {
        assert_eq!(
            decide(
                &project(ProjectState::Registered),
                Some(&facts(60, false, false)),
                now()
            ),
            nothing(Reason::NoProfile),
            "a cold project no profile recognises is worth a human's attention"
        );
    }

    #[test]
    fn a_stripped_project_waiting_for_the_archive_threshold_has_nothing_due() {
        assert_eq!(
            decide(
                &project(ProjectState::Stripped),
                Some(&facts(90, false, true)),
                now()
            ),
            nothing(Reason::Active),
            "it is already stripped and not yet old enough to archive"
        );
    }

    #[test]
    fn a_clean_project_archives_at_a_hundred_and_twenty_days() {
        assert_eq!(
            decide(
                &project(ProjectState::Stripped),
                Some(&facts(119, false, true)),
                now()
            ),
            nothing(Reason::Active)
        );
        assert_eq!(
            decide(
                &project(ProjectState::Stripped),
                Some(&facts(120, false, true)),
                now()
            ),
            Action::Archive
        );
    }

    #[test]
    fn a_registered_project_crossing_both_thresholds_is_stripped_first() {
        assert_eq!(
            decide(
                &project(ProjectState::Registered),
                Some(&facts(120, false, true)),
                now()
            ),
            Action::StripAndArchive,
            "package folders must never enter the archive"
        );
    }

    #[test]
    fn a_registered_project_with_no_profile_is_archived_as_it_stands() {
        assert_eq!(
            decide(
                &project(ProjectState::Registered),
                Some(&facts(120, false, false)),
                now()
            ),
            Action::Archive,
            "nothing can be stripped, but the age threshold is still due"
        );
    }

    #[test]
    fn a_dirty_tree_defers_archiving_to_a_hundred_and_sixty_days() {
        assert_eq!(
            decide(
                &project(ProjectState::Stripped),
                Some(&facts(159, true, true)),
                now()
            ),
            nothing(Reason::Active),
            "uncommitted work holds the archive back"
        );
        assert_eq!(
            decide(
                &project(ProjectState::Stripped),
                Some(&facts(160, true, true)),
                now()
            ),
            Action::Archive,
            "the hold is a deferral, not a block"
        );
    }

    #[test]
    fn a_dirty_tree_does_not_hold_back_stripping() {
        assert_eq!(
            decide(
                &project(ProjectState::Registered),
                Some(&facts(60, true, true)),
                now()
            ),
            Action::Strip,
            "stripped files are regenerable, so uncommitted work is not at risk"
        );
    }
}
