//! One pass, one table. The rows are built here and rendered by
//! `formats::build_output`, exactly as `list` does; a pass never formats its
//! own columns.

use crate::formats::console_data::{Cell, ConsoleData};
use crate::pass::plan::Action;

const HEADERS: [&str; 5] = ["NAME", "IDLE", "ACTION", "FREED", "RESULT"];

/// One project's row: what was decided, and what came of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    pub name: String,
    pub idle_days: i64,
    pub action: Action,
    pub outcome: Outcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Decided but deliberately not carried out: a dry run.
    Planned,
    Done {
        freed: u64,
    },
    Failed,
}

pub fn console_data(planned: &[Planned]) -> ConsoleData {
    ConsoleData {
        headers: HEADERS.map(str::to_string).to_vec(),
        rows: planned.iter().map(row).collect(),
    }
}

fn row(planned: &Planned) -> Vec<Cell> {
    vec![
        Cell::Text(planned.name.clone()),
        Cell::Days(planned.idle_days),
        Cell::Text(planned.action.name().to_string()),
        Cell::Bytes(freed(planned)),
        Cell::Text(result(planned)),
    ]
}

/// Space actually reclaimed, which is why a dry run reports zero: it
/// reclaimed nothing. What differs by action is what counts -- for a strip,
/// the size of the removed entries; for an archive, the source folder less
/// the `.pproject` that replaced it; for both, the two together. `execute`
/// works that out and hands the total here.
fn freed(planned: &Planned) -> u64 {
    match planned.outcome {
        Outcome::Done { freed } => freed,
        Outcome::Planned | Outcome::Failed => 0,
    }
}

/// A row where nothing was due shows *why*, so a project still in use and one
/// no profile recognises do not read the same.
///
/// A failure outranks that reason. A project can fail before there is an
/// action to name -- while its facts are being gathered -- and reporting the
/// placeholder reason there would print a reassuring `active` for a project
/// the pass could not even look at.
fn result(planned: &Planned) -> String {
    if planned.outcome == Outcome::Failed {
        return "failed".to_string();
    }

    if let Action::Nothing { reason } = planned.action {
        return reason.name().to_string();
    }

    match planned.outcome {
        Outcome::Planned => "dry-run".to_string(),
        Outcome::Done { .. } => "ok".to_string(),
        Outcome::Failed => unreachable!("handled above"),
    }
}

#[cfg(test)]
mod tests {
    use super::{Outcome, Planned, console_data};
    use crate::formats::console_data::Cell;
    use crate::pass::plan::{Action, Reason};

    fn planned(action: Action, outcome: Outcome) -> Planned {
        Planned {
            name: "demo".to_string(),
            idle_days: 130,
            action,
            outcome,
        }
    }

    fn cells(action: Action, outcome: Outcome) -> Vec<Cell> {
        console_data(&[planned(action, outcome)]).rows.remove(0)
    }

    #[test]
    fn every_row_matches_the_headers() {
        let data = console_data(&[
            planned(Action::Strip, Outcome::Done { freed: 1 }),
            planned(
                Action::Nothing {
                    reason: Reason::Active,
                },
                Outcome::Planned,
            ),
        ]);

        assert_eq!(data.rows.len(), 2);
        assert!(data.rows.iter().all(|row| row.len() == data.headers.len()));
    }

    #[test]
    fn a_completed_action_reports_ok_and_what_it_reclaimed() {
        assert_eq!(
            cells(Action::StripAndArchive, Outcome::Done { freed: 2048 }),
            vec![
                Cell::Text("demo".to_string()),
                Cell::Days(130),
                Cell::Text("strip+archive".to_string()),
                Cell::Bytes(2048),
                Cell::Text("ok".to_string()),
            ]
        );
    }

    #[test]
    fn a_dry_run_shows_the_same_action_but_claims_no_space() {
        let cells = cells(Action::Archive, Outcome::Planned);

        assert_eq!(cells[2], Cell::Text("archive".to_string()));
        assert_eq!(
            cells[3],
            Cell::Bytes(0),
            "a dry run reclaims nothing, so FREED stays honest at zero"
        );
        assert_eq!(cells[4], Cell::Text("dry-run".to_string()));
    }

    #[test]
    fn a_failure_is_visible_in_the_row_not_only_on_stderr() {
        let cells = cells(Action::Strip, Outcome::Failed);

        assert_eq!(cells[3], Cell::Bytes(0));
        assert_eq!(cells[4], Cell::Text("failed".to_string()));
    }

    #[test]
    fn a_failure_before_there_is_an_action_still_reads_as_a_failure() {
        let cells = cells(
            Action::Nothing {
                reason: Reason::Active,
            },
            Outcome::Failed,
        );

        assert_eq!(
            cells[4],
            Cell::Text("failed".to_string()),
            "`active` would reassure about a project the pass could not read"
        );
    }

    #[test]
    fn a_row_with_nothing_due_carries_its_reason_instead_of_a_result() {
        for (reason, expected) in [
            (Reason::Active, "active"),
            (Reason::NoProfile, "no profile"),
            (Reason::AlreadyArchived, "already archived"),
            (Reason::Missing, "missing"),
        ] {
            let cells = cells(Action::Nothing { reason }, Outcome::Planned);

            assert_eq!(cells[2], Cell::Text("none".to_string()));
            assert_eq!(cells[4], Cell::Text(expected.to_string()));
        }
    }
}
