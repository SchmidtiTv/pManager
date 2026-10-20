use std::path::Path;

use anyhow::Context as _;
use rusqlite::{Connection, OpenFlags, Row, params_from_iter};

use super::sql_builder::Query;
use crate::error::Result;

pub struct SqlLite {
    connection: Connection,
}

impl SqlLite {
    /// Open the ledger, creating the file and its parent directory if needed.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("could not create database directory {}", parent.display())
            })?;
        }

        let connection = Connection::open(path)
            .with_context(|| format!("could not open database {}", path.display()))?;

        Ok(Self { connection })
    }

    /// Open an existing database without the right to change it. A missing
    /// file is an error, not a new empty database.
    pub fn open_read_only(path: &Path) -> Result<Self> {
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("could not open database {} read-only", path.display()))?;
        Ok(Self { connection })
    }

    /// A throwaway database that never touches the disk.
    pub fn open_in_memory() -> Result<Self> {
        let connection =
            Connection::open_in_memory().context("could not open an in-memory database")?;

        Ok(Self { connection })
    }

    /// Run a statement that returns no rows, yielding the number of rows it
    /// changed.
    pub fn execute(&self, query: &Query) -> Result<usize> {
        self.connection
            .execute(&query.sql, params_from_iter(&query.params))
            .with_context(|| format!("could not execute: {}", query.sql))
    }

    /// Run a statement that returns rows, mapping each one.
    pub fn query<T>(
        &self,
        query: &Query,
        map: impl FnMut(&Row<'_>) -> rusqlite::Result<T>,
    ) -> Result<Vec<T>> {
        let failed = || format!("could not query: {}", query.sql);

        let mut statement = self.connection.prepare(&query.sql).with_context(failed)?;
        let rows = statement
            .query_map(params_from_iter(&query.params), map)
            .with_context(failed)?;

        rows.collect::<rusqlite::Result<Vec<T>>>()
            .with_context(failed)
    }

    /// Run schema statements verbatim.
    ///
    /// DDL is outside what [`crate::database::sql_builder`] covers, so it
    /// arrives as text; nothing here is parameterised.
    pub fn execute_batch(&self, sql: &str) -> Result<()> {
        self.connection
            .execute_batch(sql)
            .with_context(|| format!("could not execute schema statements: {sql}"))
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::types::Value;

    use super::SqlLite;
    use crate::database::sql_builder::SqlBuilder;

    fn seeded() -> SqlLite {
        let db = SqlLite::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE projects (name TEXT, path TEXT)")
            .unwrap();
        db
    }

    /// The two halves of `database` meeting: a built statement, executed and
    /// read back through the adapter.
    #[test]
    fn a_built_statement_round_trips_through_the_adapter() {
        let db = seeded();

        let insert = SqlBuilder::insert("projects")
            .value("name", Value::Text("pManager".to_string()))
            .value("path", Value::Text("C:/dev/rust/pManager".to_string()))
            .build()
            .unwrap();
        assert_eq!(db.execute(&insert).unwrap(), 1);

        let select = SqlBuilder::select(&["path"])
            .from_table("projects")
            .and_where("name = ?", vec![Value::Text("pManager".to_string())])
            .build()
            .unwrap();
        let paths: Vec<String> = db.query(&select, |row| row.get(0)).unwrap();

        assert_eq!(paths, vec!["C:/dev/rust/pManager".to_string()]);
    }

    /// The statement is what makes a SQLite error actionable, so it has to
    /// survive into the message -- on both paths, which attach their context
    /// at different call sites.
    #[test]
    fn a_failing_statement_names_itself_in_the_error() {
        let db = seeded();

        let select = SqlBuilder::select(&["name"])
            .from_table("no_such_table")
            .build()
            .unwrap();
        let read = db
            .query(&select, |row| row.get::<_, String>(0))
            .unwrap_err();
        assert!(
            format!("{read:#}").contains("no_such_table"),
            "query should quote the statement, got: {read:#}"
        );

        let insert = SqlBuilder::insert("no_such_table")
            .value("name", Value::Text("x".to_string()))
            .build()
            .unwrap();
        let write = db.execute(&insert).unwrap_err();
        assert!(
            format!("{write:#}").contains("no_such_table"),
            "execute should quote the statement, got: {write:#}"
        );
    }
}
