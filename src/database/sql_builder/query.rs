//! The finished statement: text plus the values that bind to it.

use std::fmt;

use rusqlite::types::Value;

/// A rendered statement and its positional parameters, in bind order.
///
/// Hand it to rusqlite with [`rusqlite::params_from_iter`]:
///
/// ```ignore
/// let query = SqlBuilder::select_all().from_table("projects").build()?;
/// let mut stmt = conn.prepare(&query.sql)?;
/// let rows = stmt.query(rusqlite::params_from_iter(&query.params))?;
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub sql: String,
    pub params: Vec<Value>,
}

impl fmt::Display for Query {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.sql)
    }
}
