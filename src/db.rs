//! SQLite storage.
//!
//! The database is a single file under the data directory (see
//! [`crate::config::Config::database_path`]). SQLite is linked in statically
//! via `rusqlite`'s `bundled` feature, so there is no system dependency and no
//! version skew between machines.
//!
//! Schema changes are additive migrations tracked by SQLite's own
//! `user_version` pragma: append to [`MIGRATIONS`], never edit a shipped
//! entry, and every existing database upgrades itself on the next open.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, params};

use crate::archive::Summary;
use crate::error::{Error, Result};

/// How long to wait for another process to release a write lock.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Ordered schema migrations. The index + 1 is the resulting `user_version`.
const MIGRATIONS: &[&str] = &[
    // v1 — record of every archive created by `vmanager archive create`.
    r#"
    CREATE TABLE archives (
        id            INTEGER PRIMARY KEY,
        source_path   TEXT    NOT NULL,
        archive_path  TEXT    NOT NULL UNIQUE,
        file_count    INTEGER NOT NULL CHECK (file_count >= 0),
        source_bytes  INTEGER NOT NULL CHECK (source_bytes >= 0),
        archive_bytes INTEGER NOT NULL CHECK (archive_bytes >= 0),
        created_at    TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
    );

    CREATE INDEX archives_created_at ON archives (created_at DESC);
    "#,
];

/// One row of the `archives` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveRecord {
    pub id: i64,
    pub source_path: PathBuf,
    pub archive_path: PathBuf,
    pub file_count: u64,
    pub source_bytes: u64,
    pub archive_bytes: u64,
    /// UTC, ISO-8601 (`2026-08-04T11:30:00Z`), written by SQLite.
    pub created_at: String,
}

/// An open connection to the vManager database.
#[derive(Debug)]
pub struct Database {
    conn: Connection,
    path: PathBuf,
}

impl Database {
    /// Open (creating if needed) the database at `path` and migrate it.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(path)?;
        let mut db = Self {
            conn,
            path: path.to_path_buf(),
        };
        db.configure()?;
        db.migrate()?;

        tracing::debug!(path = %path.display(), version = db.schema_version()?, "database ready");
        Ok(db)
    }

    /// A private in-memory database. Used by tests.
    pub fn open_in_memory() -> Result<Self> {
        let mut db = Self {
            conn: Connection::open_in_memory()?,
            path: PathBuf::from(":memory:"),
        };
        db.configure()?;
        db.migrate()?;
        Ok(db)
    }

    /// Path this database was opened from.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The schema version currently applied.
    pub fn schema_version(&self) -> Result<u32> {
        Ok(self
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))?)
    }

    /// The newest schema version this build knows how to produce.
    pub fn target_version() -> u32 {
        MIGRATIONS.len() as u32
    }

    /// Record a finished archive, returning its row id.
    pub fn record_archive(&self, summary: &Summary) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO archives (source_path, archive_path, file_count, source_bytes, archive_bytes)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (archive_path) DO UPDATE SET
                 source_path   = excluded.source_path,
                 file_count    = excluded.file_count,
                 source_bytes  = excluded.source_bytes,
                 archive_bytes = excluded.archive_bytes,
                 created_at    = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')",
            params![
                summary.source.to_string_lossy(),
                summary.archive.to_string_lossy(),
                // SQLite integers are signed; the CHECK constraints keep
                // these non-negative on the way back out.
                summary.file_count as i64,
                summary.source_bytes as i64,
                summary.archive_bytes as i64,
            ],
        )?;

        Ok(self.conn.last_insert_rowid())
    }

    /// The most recently created archives, newest first.
    pub fn recent_archives(&self, limit: u32) -> Result<Vec<ArchiveRecord>> {
        let mut statement = self.conn.prepare(
            "SELECT id, source_path, archive_path, file_count, source_bytes, archive_bytes, created_at
               FROM archives
              ORDER BY created_at DESC, id DESC
              LIMIT ?1",
        )?;

        let rows = statement.query_map([limit], row_to_record)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Look up a single archive by row id.
    pub fn archive(&self, id: i64) -> Result<ArchiveRecord> {
        self.conn
            .query_row(
                "SELECT id, source_path, archive_path, file_count, source_bytes, archive_bytes, created_at
                   FROM archives WHERE id = ?1",
                [id],
                row_to_record,
            )
            .optional()?
            .ok_or_else(|| Error::NotFound {
                kind: "archive",
                name: id.to_string(),
            })
    }

    /// Drop a record. The archive file itself is left alone.
    pub fn forget_archive(&self, id: i64) -> Result<()> {
        let removed = self
            .conn
            .execute("DELETE FROM archives WHERE id = ?1", [id])?;
        if removed == 0 {
            return Err(Error::NotFound {
                kind: "archive",
                name: id.to_string(),
            });
        }
        Ok(())
    }

    /// Drop records whose archive file no longer exists, returning them.
    pub fn prune_missing(&self) -> Result<Vec<ArchiveRecord>> {
        let stale: Vec<ArchiveRecord> = self
            .recent_archives(u32::MAX)?
            .into_iter()
            .filter(|record| !record.archive_path.exists())
            .collect();

        for record in &stale {
            self.conn
                .execute("DELETE FROM archives WHERE id = ?1", [record.id])?;
        }

        Ok(stale)
    }

    /// Number of archives on record.
    pub fn archive_count(&self) -> Result<u64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM archives", [], |row| {
                row.get::<_, i64>(0)
            })? as u64)
    }

    /// Connection-level settings applied on every open.
    fn configure(&mut self) -> Result<()> {
        self.conn.busy_timeout(BUSY_TIMEOUT)?;
        self.conn
            .execute_batch("PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL;")?;

        // WAL keeps readers from blocking the writer. In-memory databases
        // report "memory" instead and that is fine.
        let _mode: String = self
            .conn
            .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
        Ok(())
    }

    /// Apply every migration the database has not seen yet.
    fn migrate(&mut self) -> Result<()> {
        let current = self.schema_version()? as usize;

        if current > MIGRATIONS.len() {
            return Err(Error::Database(format!(
                "database `{}` has schema version {current}, but this build only knows up to {}; \
                 upgrade vmanager",
                self.path.display(),
                MIGRATIONS.len()
            )));
        }

        for (index, migration) in MIGRATIONS.iter().enumerate().skip(current) {
            let version = index + 1;
            tracing::debug!(version, "applying migration");

            // Each migration is atomic: either the statements and the version
            // bump both land, or neither does.
            let tx = self.conn.transaction()?;
            tx.execute_batch(migration)?;
            tx.pragma_update(None, "user_version", version as i64)?;
            tx.commit()?;
        }

        Ok(())
    }
}

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArchiveRecord> {
    Ok(ArchiveRecord {
        id: row.get(0)?,
        source_path: PathBuf::from(row.get::<_, String>(1)?),
        archive_path: PathBuf::from(row.get::<_, String>(2)?),
        file_count: row.get::<_, i64>(3)? as u64,
        source_bytes: row.get::<_, i64>(4)? as u64,
        archive_bytes: row.get::<_, i64>(5)? as u64,
        created_at: row.get(6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(archive: &str) -> Summary {
        Summary {
            source: PathBuf::from("/data/photos"),
            archive: PathBuf::from(archive),
            file_count: 3,
            source_bytes: 1024,
            archive_bytes: 256,
        }
    }

    #[test]
    fn fresh_database_is_fully_migrated() {
        let db = Database::open_in_memory().unwrap();
        assert_eq!(db.schema_version().unwrap(), Database::target_version());
        assert_eq!(db.archive_count().unwrap(), 0);
    }

    #[test]
    fn opening_twice_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("vmanager.db");

        let first = Database::open(&path).unwrap();
        first.record_archive(&summary("/tmp/a.7z")).unwrap();
        drop(first);

        let second = Database::open(&path).unwrap();
        assert_eq!(second.schema_version().unwrap(), Database::target_version());
        assert_eq!(second.archive_count().unwrap(), 1);
    }

    #[test]
    fn records_and_reads_back_an_archive() {
        let db = Database::open_in_memory().unwrap();
        let id = db.record_archive(&summary("/tmp/photos.7z")).unwrap();

        let record = db.archive(id).unwrap();
        assert_eq!(record.archive_path, PathBuf::from("/tmp/photos.7z"));
        assert_eq!(record.source_path, PathBuf::from("/data/photos"));
        assert_eq!(record.file_count, 3);
        assert_eq!(record.archive_bytes, 256);
        assert!(
            record.created_at.ends_with('Z') && record.created_at.len() == 20,
            "unexpected timestamp {}",
            record.created_at
        );
    }

    #[test]
    fn re_archiving_the_same_path_updates_the_row() {
        let db = Database::open_in_memory().unwrap();
        db.record_archive(&summary("/tmp/photos.7z")).unwrap();

        let mut second = summary("/tmp/photos.7z");
        second.file_count = 99;
        db.record_archive(&second).unwrap();

        assert_eq!(db.archive_count().unwrap(), 1);
        assert_eq!(db.recent_archives(10).unwrap()[0].file_count, 99);
    }

    #[test]
    fn recent_archives_are_newest_first_and_limited() {
        let db = Database::open_in_memory().unwrap();
        for name in ["/tmp/a.7z", "/tmp/b.7z", "/tmp/c.7z"] {
            db.record_archive(&summary(name)).unwrap();
        }

        let recent = db.recent_archives(2).unwrap();
        assert_eq!(recent.len(), 2);
        // Same-second timestamps tie-break on id, so the newest row wins.
        assert_eq!(recent[0].archive_path, PathBuf::from("/tmp/c.7z"));
    }

    #[test]
    fn forgetting_an_unknown_archive_is_not_found() {
        let db = Database::open_in_memory().unwrap();
        assert!(matches!(
            db.forget_archive(42),
            Err(Error::NotFound {
                kind: "archive",
                ..
            })
        ));
        assert!(matches!(db.archive(42), Err(Error::NotFound { .. })));
    }

    #[test]
    fn forgetting_removes_only_the_named_row() {
        let db = Database::open_in_memory().unwrap();
        let keep = db.record_archive(&summary("/tmp/keep.7z")).unwrap();
        let drop_me = db.record_archive(&summary("/tmp/drop.7z")).unwrap();

        db.forget_archive(drop_me).unwrap();
        assert_eq!(db.archive_count().unwrap(), 1);
        assert!(db.archive(keep).is_ok());
    }

    #[test]
    fn prune_drops_records_whose_file_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let present = dir.path().join("present.7z");
        std::fs::write(&present, b"7z").unwrap();

        let db = Database::open_in_memory().unwrap();
        db.record_archive(&summary(present.to_str().unwrap()))
            .unwrap();
        db.record_archive(&summary("/tmp/definitely/missing.7z"))
            .unwrap();

        let pruned = db.prune_missing().unwrap();
        assert_eq!(pruned.len(), 1);
        assert_eq!(
            pruned[0].archive_path,
            PathBuf::from("/tmp/definitely/missing.7z")
        );
        assert_eq!(db.archive_count().unwrap(), 1);
    }

    #[test]
    fn a_database_from_the_future_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vmanager.db");

        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", 999_i64).unwrap();
        drop(conn);

        let err = Database::open(&path).unwrap_err();
        assert!(matches!(err, Error::Database(_)), "{err}");
        assert!(err.to_string().contains("upgrade vmanager"), "{err}");
    }
}
