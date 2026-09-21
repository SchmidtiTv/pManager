//! INSERT.

use rusqlite::types::Value;

use super::error::SqlBuilderError;
use super::ident::{ident, ident_list, placeholders};
use super::query::Query;

pub struct Insert {
    table: String,
    assignments: Vec<(String, Value)>,
}

impl Insert {
    pub(super) fn new(table: &str) -> Self {
        Self {
            table: table.to_string(),
            assignments: Vec::new(),
        }
    }

    /// Insertion order is preserved, so columns and placeholders stay aligned.
    pub fn value(mut self, column: &str, value: Value) -> Self {
        self.assignments.push((column.to_string(), value));
        self
    }

    pub fn build(&self) -> Result<Query, SqlBuilderError> {
        if self.assignments.is_empty() {
            return Err(SqlBuilderError::NoInsertValues);
        }

        let columns: Vec<String> = self
            .assignments
            .iter()
            .map(|(column, _)| column.clone())
            .collect();
        super::reject_duplicates(&columns)?;

        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({});",
            ident(&self.table)?,
            ident_list(&columns)?,
            placeholders(columns.len())
        );

        let params = self
            .assignments
            .iter()
            .map(|(_, value)| value.clone())
            .collect();

        Ok(Query { sql, params })
    }
}
