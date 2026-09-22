use std::path::{Path, PathBuf};

use anyhow::Context as _;
use chrono::{DateTime, Utc};
use rusqlite::Row;
use rusqlite::types::{FromSqlError, Value};

use crate::database::sql_builder::{Query, SqlBuilder};
use crate::database::timestamp;
use crate::error::Result;
use crate::models::state::ProjectState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub name: String,
    pub path: PathBuf,
    pub state: ProjectState,
    pub has_git_repo: bool,
    pub last_change_at: DateTime<Utc>,
    pub registered_at: DateTime<Utc>,
}

pub const COLUMNS: [&str; 7] = [
    "name",
    "path",
    "state",
    "has_git_repo",
    "archive_path",
    "last_change_at",
    "registered_at",
];

pub fn insert(project: &Project) -> Result<Query> {
    let query = SqlBuilder::insert("projects")
        .value("name", Value::Text(project.name.clone()))
        .value("path", path_value(&project.path)?)
        .value("state", Value::Text(project.state.name().to_string()))
        .value(
            "has_git_repo",
            Value::Integer(i64::from(project.has_git_repo)),
        )
        .value("archive_path", archive_path_value(&project.state)?)
        .value(
            "last_change_at",
            timestamp::to_value(project.last_change_at),
        )
        .value("registered_at", timestamp::to_value(project.registered_at))
        .build()?;

    Ok(query)
}

/// Write back the columns a pass can change, matched on the path.
///
/// `name`, `path` and `registered_at` are deliberately absent: they identify
/// the row and are set once, at registration. Everything a pass learns --
/// the new state, where the archive went, and when the project was last
/// worked on -- is written in this one statement, so a project either moves
/// forward completely or not at all.
pub fn update(project: &Project) -> Result<Query> {
    let query = SqlBuilder::update("projects")
        .set("state", Value::Text(project.state.name().to_string()))
        .set("archive_path", archive_path_value(&project.state)?)
        .set(
            "last_change_at",
            timestamp::to_value(project.last_change_at),
        )
        .and_where("path = ?", vec![path_value(&project.path)?])
        .build()?;

    Ok(query)
}

pub fn select_all() -> Result<Query> {
    let query = SqlBuilder::select(&COLUMNS)
        .from_table("projects")
        .order_by_asc(&["name"])
        .build()?;

    Ok(query)
}

/// Every row that would collide with registering `name` at `path`.
pub fn select_by_name_or_path(name: &str, path: &Path) -> Result<Query> {
    let query = SqlBuilder::select(&COLUMNS)
        .from_table("projects")
        .and_where(
            "name = ? OR path = ?",
            vec![Value::Text(name.to_string()), path_value(path)?],
        )
        .order_by_asc(&["name"])
        .build()?;

    Ok(query)
}

pub fn select_by_path(path: &Path) -> Result<Query> {
    let query = SqlBuilder::select(&COLUMNS)
        .from_table("projects")
        .and_where("path = ?", vec![path_value(path)?])
        .build()?;

    Ok(query)
}

pub fn delete_by_path(path: &Path) -> Result<Query> {
    let query = SqlBuilder::delete("projects")
        .and_where("path = ?", vec![path_value(path)?])
        .build()?;

    Ok(query)
}

pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Project> {
    let path: String = row.get("path")?;
    let state: String = row.get("state")?;
    let archive_path: Option<String> = row.get("archive_path")?;

    Ok(Project {
        name: row.get("name")?,
        path: PathBuf::from(path),
        state: state_from(&state, archive_path)?,
        has_git_repo: row.get("has_git_repo")?,
        last_change_at: row.get("last_change_at")?,
        registered_at: row.get("registered_at")?,
    })
}

fn path_value(path: &Path) -> Result<Value> {
    let text = path
        .to_str()
        .with_context(|| format!("path is not valid UTF-8: {}", path.display()))?;

    Ok(Value::Text(text.to_string()))
}

fn archive_path_value(state: &ProjectState) -> Result<Value> {
    match state.archive_path() {
        Some(path) => path_value(path),
        None => Ok(Value::Null),
    }
}

fn state_from(state: &str, archive_path: Option<String>) -> rusqlite::Result<ProjectState> {
    let invalid = |reason: String| {
        rusqlite::Error::FromSqlConversionFailure(
            2,
            rusqlite::types::Type::Text,
            Box::new(FromSqlError::Other(reason.into())),
        )
    };

    match (state, archive_path) {
        ("registered", None) => Ok(ProjectState::Registered),
        ("stripped", None) => Ok(ProjectState::Stripped),
        ("archived", Some(archive_path)) => Ok(ProjectState::Archived {
            archive_path: PathBuf::from(archive_path),
        }),
        ("registered" | "stripped", Some(path)) => Err(invalid(format!(
            "state {state:?} must not carry an archive_path, found {path:?}"
        ))),
        ("archived", None) => Err(invalid(
            "state \"archived\" requires an archive_path, found NULL".to_string(),
        )),
        _ => Err(invalid(format!("unknown project state {state:?}"))),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::{TimeZone as _, Utc};

    use std::path::Path;

    use super::{
        COLUMNS, Project, delete_by_path, from_row, insert, select_by_name_or_path, select_by_path,
        update,
    };
    use crate::database::schema;
    use crate::database::sql_builder::SqlBuilder;
    use crate::database::sql_lite::SqlLite;
    use crate::models::state::ProjectState;

    fn applied() -> SqlLite {
        let db = SqlLite::open_in_memory().unwrap();
        schema::apply(&db).unwrap();
        db
    }

    fn project(state: ProjectState) -> Project {
        Project {
            name: "pManager".to_string(),
            path: PathBuf::from("C:/dev/rust/pManager"),
            state,
            has_git_repo: true,
            last_change_at: Utc.with_ymd_and_hms(2026, 7, 1, 8, 30, 0).unwrap(),
            registered_at: Utc.with_ymd_and_hms(2026, 6, 1, 9, 0, 0).unwrap(),
        }
    }

    fn read_back(db: &SqlLite) -> Vec<Project> {
        let query = SqlBuilder::select(&COLUMNS)
            .from_table("projects")
            .build()
            .unwrap();
        db.query(&query, from_row).unwrap()
    }

    #[test]
    fn a_registered_project_round_trips() {
        let db = applied();
        let original = project(ProjectState::Registered);

        db.execute(&insert(&original).unwrap()).unwrap();

        assert_eq!(read_back(&db), vec![original]);
    }

    #[test]
    fn an_archived_project_round_trips_with_its_archive_path() {
        let db = applied();
        let original = project(ProjectState::Archived {
            archive_path: PathBuf::from("C:/archive/pManager.pproject"),
        });

        db.execute(&insert(&original).unwrap()).unwrap();

        let read = read_back(&db);
        assert_eq!(read, vec![original]);
        assert_eq!(
            read[0].state.archive_path(),
            Some(Path::new("C:/archive/pManager.pproject")),
            "the archive path must come back as a path, not a string"
        );
    }

    #[test]
    fn a_pass_can_move_a_project_to_archived_in_one_update() {
        let db = applied();
        db.execute(&insert(&project(ProjectState::Registered)).unwrap())
            .unwrap();

        let archived = Project {
            state: ProjectState::Archived {
                archive_path: PathBuf::from("C:/dev/rust/pManager.pproject"),
            },
            last_change_at: Utc.with_ymd_and_hms(2026, 9, 1, 12, 0, 0).unwrap(),
            ..project(ProjectState::Registered)
        };

        let changed = db.execute(&update(&archived).unwrap()).unwrap();

        assert_eq!(changed, 1, "exactly one row matches the path");
        assert_eq!(read_back(&db), vec![archived]);
    }

    #[test]
    fn stripping_clears_nothing_but_moves_the_state() {
        let db = applied();
        db.execute(&insert(&project(ProjectState::Registered)).unwrap())
            .unwrap();

        let stripped = project(ProjectState::Stripped);
        db.execute(&update(&stripped).unwrap()).unwrap();

        let read = read_back(&db);
        assert_eq!(read, vec![stripped]);
        assert_eq!(
            read[0].registered_at,
            Utc.with_ymd_and_hms(2026, 6, 1, 9, 0, 0).unwrap(),
            "an update must not rewrite when the project was registered"
        );
    }

    #[test]
    fn updating_a_path_that_is_not_registered_changes_nothing() {
        let db = applied();
        db.execute(&insert(&project(ProjectState::Registered)).unwrap())
            .unwrap();

        let elsewhere = Project {
            path: PathBuf::from("C:/dev/rust/other"),
            ..project(ProjectState::Stripped)
        };

        assert_eq!(db.execute(&update(&elsewhere).unwrap()).unwrap(), 0);
        assert_eq!(read_back(&db), vec![project(ProjectState::Registered)]);
    }

    #[test]
    fn a_name_or_a_path_collision_is_found() {
        let db = applied();
        let existing = project(ProjectState::Registered);
        db.execute(&insert(&existing).unwrap()).unwrap();

        let by_name = select_by_name_or_path("pManager", Path::new("C:/elsewhere")).unwrap();
        let by_path = select_by_name_or_path("other", Path::new("C:/dev/rust/pManager")).unwrap();
        let neither = select_by_name_or_path("other", Path::new("C:/elsewhere")).unwrap();

        assert_eq!(
            db.query(&by_name, from_row).unwrap(),
            vec![existing.clone()]
        );
        assert_eq!(db.query(&by_path, from_row).unwrap(), vec![existing]);
        assert!(
            db.query(&neither, from_row).unwrap().is_empty(),
            "an unrelated name and path must not collide"
        );
    }

    #[test]
    fn deleting_by_path_removes_only_that_project() {
        let db = applied();
        let kept = Project {
            name: "other".to_string(),
            path: PathBuf::from("C:/dev/rust/other"),
            ..project(ProjectState::Registered)
        };
        db.execute(&insert(&project(ProjectState::Registered)).unwrap())
            .unwrap();
        db.execute(&insert(&kept).unwrap()).unwrap();

        let deleted = db
            .execute(&delete_by_path(Path::new("C:/dev/rust/pManager")).unwrap())
            .unwrap();

        assert_eq!(deleted, 1, "exactly one row matches the path");
        assert_eq!(read_back(&db), vec![kept]);
        assert!(
            db.query(
                &select_by_path(Path::new("C:/dev/rust/pManager")).unwrap(),
                from_row
            )
            .unwrap()
            .is_empty(),
            "the deleted project must no longer be found by its path"
        );
    }

    #[test]
    fn an_unreadable_state_names_the_offending_value() {
        let db = SqlLite::open_in_memory().unwrap();
        // No CHECK constraint here: this stands in for a ledger that a newer
        // version, or a text editor, has written.
        db.execute_batch(
            "CREATE TABLE projects (name TEXT, path TEXT, state TEXT,
                has_git_repo INTEGER, archive_path TEXT, last_change_at TEXT,
                registered_at TEXT);
             INSERT INTO projects VALUES ('x', 'C:/x', 'hibernating', 1, NULL,
                '2026-01-01 00:00:00+00:00', '2026-01-01 00:00:00+00:00');",
        )
        .unwrap();

        let query = SqlBuilder::select(&COLUMNS)
            .from_table("projects")
            .build()
            .unwrap();
        let err = db.query(&query, from_row).unwrap_err();

        assert!(
            format!("{err:#}").contains("hibernating"),
            "the error should name the bad state, got: {err:#}"
        );
    }

    #[test]
    fn a_non_archived_row_carrying_an_archive_path_is_rejected() {
        let db = SqlLite::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE projects (name TEXT, path TEXT, state TEXT,
                has_git_repo INTEGER, archive_path TEXT, last_change_at TEXT,
                registered_at TEXT);
             INSERT INTO projects VALUES ('x', 'C:/x', 'registered', 1,
                'C:/archive/x.pproject',
                '2026-01-01 00:00:00+00:00', '2026-01-01 00:00:00+00:00');",
        )
        .unwrap();

        let query = SqlBuilder::select(&COLUMNS)
            .from_table("projects")
            .build()
            .unwrap();
        let err = db.query(&query, from_row).unwrap_err();

        assert!(
            format!("{err:#}").contains("must not carry an archive_path"),
            "got: {err:#}"
        );
    }
}
