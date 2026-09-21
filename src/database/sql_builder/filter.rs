//! The WHERE/HAVING state shared by SELECT, UPDATE and DELETE.

use rusqlite::types::Value;

use super::error::SqlBuilderError;
use super::ident::{ident, placeholders};

/// One condition, kept structured until render time so that identifiers are
/// quoted exactly once and raw caller SQL is never re-parsed.
enum Condition {
    /// SQL written by the caller, passed through verbatim.
    Raw(String),
    /// `column IN (?, ?, ...)`.
    In { column: String, values: usize },
    /// What an `IN` over an empty set means.
    AlwaysFalse,
}

/// Conditions that are ANDed together, plus their bound values in the order
/// they appear in the rendered text.
pub(super) struct Filter {
    conditions: Vec<Condition>,
    params: Vec<Value>,
}

impl Filter {
    pub(super) fn new() -> Self {
        Self {
            conditions: Vec::new(),
            params: Vec::new(),
        }
    }

    /// Add a raw condition. `condition` is SQL written by the caller and is
    /// never escaped; only `params` crosses the trust boundary.
    pub(super) fn and(mut self, condition: &str, params: Vec<Value>) -> Self {
        self.conditions.push(Condition::Raw(condition.to_string()));
        self.params.extend(params);
        self
    }

    /// Add `column IN (...)`. The identifier is checked at render time so
    /// that the chaining methods stay infallible.
    pub(super) fn and_in(mut self, column: &str, values: Vec<Value>) -> Self {
        if values.is_empty() {
            // An IN over no values matches nothing; saying so in SQL keeps
            // the condition visible instead of quietly dropping it.
            self.conditions.push(Condition::AlwaysFalse);
            return self;
        }

        self.conditions.push(Condition::In {
            column: column.to_string(),
            values: values.len(),
        });
        self.params.extend(values);
        self
    }

    pub(super) fn is_empty(&self) -> bool {
        self.conditions.is_empty()
    }

    pub(super) fn params(&self) -> &[Value] {
        &self.params
    }

    /// Join the conditions with AND, parenthesising each one.
    ///
    /// The parentheses are not cosmetic: without them a caller's
    /// `"a = 1 OR b = 2"` would bind looser than the AND this adds.
    pub(super) fn render(&self) -> Result<String, SqlBuilderError> {
        let rendered: Vec<String> = self
            .conditions
            .iter()
            .map(render_condition)
            .collect::<Result<_, _>>()?;

        Ok(rendered.join(" AND "))
    }
}

fn render_condition(condition: &Condition) -> Result<String, SqlBuilderError> {
    match condition {
        Condition::Raw(sql) => Ok(format!("({sql})")),
        Condition::In { column, values } => Ok(format!(
            "({} IN ({}))",
            ident(column)?,
            placeholders(*values)
        )),
        Condition::AlwaysFalse => Ok("(1 = 0)".to_string()),
    }
}
