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
    pub kind: String,
    pub state: ProjectState,
    pub has_git_repo: bool,
    pub last_change_at: DateTime<Utc>,
    pub registered_at: DateTime<Utc>,
}

pub const COLUMNS: [&str; 8] = [
    "name",
    "path",
    "kind",
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
        .value("kind", Value::Text(project.kind.clone()))
        .value("state", Value::Text(project.state.name().to_string()))
        .value(
            "has_git_repo",
            Value::Integer(i64::from(project.has_git_repo)),
        )
        .value("archive_path", archive_path_value(project))
        .value(
            "last_change_at",
            timestamp::to_value(project.last_change_at),
        )
        .value("registered_at", timestamp::to_value(project.registered_at))
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

pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Project> {
    let path: String = row.get("path")?;
    let state: String = row.get("state")?;
    let archive_path: Option<String> = row.get("archive_path")?;

    Ok(Project {
        name: row.get("name")?,
        path: PathBuf::from(path),
        kind: row.get("kind")?,
        state: state_from(&state, archive_path)?,
        has_git_repo: row.get("has_git_repo")?,
        last_change_at: row.get("last_change_at")?,
        registered_at: row.get("registered_at")?,
    })
}

fn path_value(path: &Path) -> Result<Value> {
    let text = path
        .to_str()
        .with_context(|| format!("project path is not valid UTF-8: {}", path.display()))?;

    Ok(Value::Text(text.to_string()))
}

fn archive_path_value(project: &Project) -> Value {
    match project.state.archive_path() {
        Some(path) => Value::Text(path.to_string()),
        None => Value::Null,
    }
}

fn state_from(state: &str, archive_path: Option<String>) -> rusqlite::Result<ProjectState> {
    let invalid = |reason: String| {
        rusqlite::Error::FromSqlConversionFailure(
            3,
            rusqlite::types::Type::Text,
            Box::new(FromSqlError::Other(reason.into())),
        )
    };

    match (state, archive_path) {
        ("registered", None) => Ok(ProjectState::Registered),
        ("stripped", None) => Ok(ProjectState::Stripped),
        ("archived", Some(archive_path)) => Ok(ProjectState::Archived { archive_path }),
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

    use super::{COLUMNS, Project, from_row, insert};
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
            kind: "rust".to_string(),
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
            archive_path: "C:/archive/pManager.pproject".to_string(),
        });

        db.execute(&insert(&original).unwrap()).unwrap();

        assert_eq!(read_back(&db), vec![original]);
    }

    #[test]
    fn an_unreadable_state_names_the_offending_value() {
        let db = SqlLite::open_in_memory().unwrap();
        // No CHECK constraint here: this stands in for a ledger that a newer
        // version, or a text editor, has written.
        db.execute_batch(
            "CREATE TABLE projects (name TEXT, path TEXT, kind TEXT, state TEXT,
                has_git_repo INTEGER, archive_path TEXT, last_change_at TEXT,
                registered_at TEXT);
             INSERT INTO projects VALUES ('x', 'C:/x', 'rust', 'hibernating', 1, NULL,
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
            "CREATE TABLE projects (name TEXT, path TEXT, kind TEXT, state TEXT,
                has_git_repo INTEGER, archive_path TEXT, last_change_at TEXT,
                registered_at TEXT);
             INSERT INTO projects VALUES ('x', 'C:/x', 'rust', 'registered', 1,
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
