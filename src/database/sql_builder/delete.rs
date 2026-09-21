//! DELETE.

use rusqlite::types::Value;

use super::error::SqlBuilderError;
use super::filter::Filter;
use super::ident::ident;
use super::query::Query;

pub struct Delete {
    table: String,
    filter: Filter,
}

impl Delete {
    pub(super) fn new(table: &str) -> Self {
        Self {
            table: table.to_string(),
            filter: Filter::new(),
        }
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
        if self.filter.is_empty() {
            return Err(SqlBuilderError::MissingWhere {
                statement: "delete()",
            });
        }

        let sql = format!(
            "DELETE FROM {} WHERE {};",
            ident(&self.table)?,
            self.filter.render()?
        );

        Ok(Query {
            sql,
            params: self.filter.params().to_vec(),
        })
    }
}
