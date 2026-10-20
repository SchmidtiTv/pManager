use std::path::PathBuf;

use rusqlite::Row;
use rusqlite::types::FromSqlError;

use crate::database::sql_builder::{Query, SqlBuilder};
use crate::error::Result;

/// The `user_version` of the last pManager v1 schema.
pub const SCHEMA_VERSION: i64 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum V1State {
    Active,
    Trimmed,
    Archived,
}

/// One row of v1's `projects` table, with only the columns v2 can use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V1Project {
    pub name: String,
    pub path: PathBuf,
    pub state: V1State,
    /// Unix seconds.
    pub added_at: i64,
    /// Unix seconds.
    pub archived_at: Option<i64>,
    pub archive_path: Option<PathBuf>,
}

pub const COLUMNS: [&str; 6] = [
    "name",
    "path",
    "state",
    "added_at",
    "archived_at",
    "archive_path",
];

pub fn select_all() -> Result<Query> {
    let query = SqlBuilder::select(&COLUMNS)
        .from_table("projects")
        .order_by_asc(&["name"])
        .build()?;
    Ok(query)
}

pub fn count_pins() -> Query {
    Query {
        sql: "SELECT COUNT(*) FROM project_versions;".to_string(),
        params: Vec::new(),
    }
}

pub fn from_row(row: &Row<'_>) -> rusqlite::Result<V1Project> {
    let path: String = row.get("path")?;
    let state: String = row.get("state")?;
    let archive_path: Option<String> = row.get("archive_path")?;

    Ok(V1Project {
        name: row.get("name")?,
        path: PathBuf::from(path),
        state: state_from(&state)?,
        added_at: row.get("added_at")?,
        archived_at: row.get("archived_at")?,
        archive_path: archive_path.map(PathBuf::from),
    })
}

fn state_from(state: &str) -> rusqlite::Result<V1State> {
    match state {
        "active" => Ok(V1State::Active),
        "trimmed" => Ok(V1State::Trimmed),
        "archived" => Ok(V1State::Archived),
        _ => Err(rusqlite::Error::FromSqlConversionFailure(
            2,
            rusqlite::types::Type::Text,
            Box::new(FromSqlError::Other(
                format!("unknown v1 project state {state:?}").into(),
            )),
        )),
    }
}
