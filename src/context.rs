//! `Cx` — the single value every command receives.

use anyhow::Context as _;
use rusqlite::Connection;

use crate::error::Result;
use crate::paths::Paths;

pub struct Cx {
    pub paths: Paths,
    db: Option<Connection>,
}

impl Cx {
    pub fn new(paths: Paths) -> Self {
        Self { paths, db: None }
    }

    /// The SQLite ledger, opened on first use.
    ///
    /// Opening is deferred so that `pman --help` and the unimplemented stubs
    /// never create a database as a side effect.
    pub fn db(&mut self) -> Result<&Connection> {
        if self.db.is_none() {
            if let Some(parent) = self.paths.db.parent() {
                std::fs::create_dir_all(parent).with_context(|| {
                    format!("could not create database directory {}", parent.display())
                })?;
            }

            let conn = Connection::open(&self.paths.db)
                .with_context(|| format!("could not open database {}", self.paths.db.display()))?;

            self.db = Some(conn);
        }

        Ok(self.db.as_ref().expect("connection was just stored"))
    }
}

#[cfg(test)]
mod tests {
    use super::Cx;
    use crate::paths::Paths;
    use std::path::Path;

    fn cx_in(dir: &Path) -> Cx {
        Cx::new(Paths::resolve_with(
            Some(dir.join("nested").join("pmanager.db")),
            Some(dir.join("strip")),
            Path::new("/unused"),
            Path::new("/unused"),
        ))
    }

    #[test]
    fn constructing_a_context_does_not_touch_the_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let cx = cx_in(tmp.path());
        assert!(
            !cx.paths.db.exists(),
            "database must not be created eagerly"
        );
    }

    #[test]
    fn asking_for_the_database_creates_it_and_its_parent() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cx = cx_in(tmp.path());

        cx.db().expect("opening the database should succeed");

        assert!(cx.paths.db.exists(), "database file should now exist");
    }

    #[test]
    fn the_connection_is_reused_across_calls() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cx = cx_in(tmp.path());

        // A TEMP table lives on the connection, not in the file. If `db()`
        // reopened the database, the second call would not see it -- which is
        // exactly what makes this a test of reuse rather than of persistence.
        cx.db()
            .unwrap()
            .execute_batch("CREATE TEMP TABLE marker (id INTEGER)")
            .unwrap();

        let count: i64 = cx
            .db()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM sqlite_temp_master WHERE name = 'marker'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "db() must hand back the same live connection");
    }
}
