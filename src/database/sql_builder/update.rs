//! UPDATE.

use rusqlite::types::Value;

use super::error::SqlBuilderError;
use super::filter::Filter;
use super::ident::ident;
use super::query::Query;

pub struct Update {
    table: String,
    assignments: Vec<(String, Value)>,
    filter: Filter,
}

impl Update {
    pub(super) fn new(table: &str) -> Self {
        Self {
            table: table.to_string(),
            assignments: Vec::new(),
            filter: Filter::new(),
        }
    }

    pub fn set(mut self, column: &str, value: Value) -> Self {
        self.assignments.push((column.to_string(), value));
        self
    }

    /// `condition` is raw SQL with `?` placeholders; `params` bind to them.
    pub fn and_where(mut self, condition: &str, params: Vec<Value>) -> Self {
        self.filter = self.filter.and(condition, params);
        self
    }

    pub fn and_where_in(mut self, column: &str, values: Vec<Value>) -> Self {
        self.filter = self.filter.and_in(column, values);
        self
    }

    pub fn build(&self) -> Result<Query, SqlBuilderError> {
        if self.assignments.is_empty() {
            return Err(SqlBuilderError::NoAssignments);
        }
        if self.filter.is_empty() {
            return Err(SqlBuilderError::MissingWhere {
                statement: "update()",
            });
        }

        let columns: Vec<String> = self
            .assignments
            .iter()
            .map(|(column, _)| column.clone())
            .collect();
        super::reject_duplicates(&columns)?;

        let assignments: Vec<String> = columns
            .iter()
            .map(|column| ident(column).map(|quoted| format!("{quoted} = ?")))
            .collect::<Result<_, _>>()?;

        let sql = format!(
            "UPDATE {} SET {} WHERE {};",
            ident(&self.table)?,
            assignments.join(", "),
            self.filter.render()?
        );

        // SET values bind before the WHERE values, matching their order in
        // the text above.
        let mut params: Vec<Value> = self
            .assignments
            .iter()
            .map(|(_, value)| value.clone())
            .collect();
        params.extend(self.filter.params().iter().cloned());

        Ok(Query { sql, params })
    }
}
