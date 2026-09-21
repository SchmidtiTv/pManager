//! A small SQL builder for the SQLite ledger.
//! ```ignore
//! let query = SqlBuilder::select(&["id", "name"])
//!     .from_table("users")
//!     .and_where("age > ?", vec![Value::Integer(18)])
//!     .order_by_asc(&["name"])
//!     .limit(10)
//!     .build()?;
//!
//! let mut stmt = conn.prepare(&query.sql)?;
//! let rows = stmt.query(rusqlite::params_from_iter(&query.params))?;
//! ```

mod delete;
mod error;
mod filter;
mod ident;
mod insert;
mod query;
mod select;
mod update;

use std::collections::HashSet;

pub use delete::Delete;
pub use error::SqlBuilderError;
pub use insert::Insert;
pub use query::Query;
pub use select::{JoinKind, Select, SelectColumns};
pub use update::Update;

use select::Columns;

/// The entry point. Each constructor starts one kind of statement.
pub struct SqlBuilder;

impl SqlBuilder {
    pub fn select(columns: &[&str]) -> SelectColumns {
        SelectColumns::new(Columns::Named(
            columns.iter().map(|column| column.to_string()).collect(),
        ))
    }

    pub fn select_all() -> SelectColumns {
        SelectColumns::new(Columns::All)
    }

    pub fn insert(table: &str) -> Insert {
        Insert::new(table)
    }

    pub fn update(table: &str) -> Update {
        Update::new(table)
    }

    pub fn delete(table: &str) -> Delete {
        Delete::new(table)
    }
}

fn reject_duplicates(columns: &[String]) -> Result<(), SqlBuilderError> {
    let mut seen: HashSet<&str> = HashSet::with_capacity(columns.len());

    for column in columns {
        if !seen.insert(column.as_str()) {
            return Err(SqlBuilderError::DuplicateColumn {
                column: column.clone(),
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests;
