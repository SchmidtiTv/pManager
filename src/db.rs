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
    // v1 — record of every archive written.
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
    // v2 — the registry of folders `vmanager run` sweeps, and the link from an
    // archive back to the folder whose version it holds.
    r#"
    CREATE TABLE folders (
        id       INTEGER PRIMARY KEY,
        path     TEXT    NOT NULL UNIQUE,
        keep     INTEGER          CHECK (keep IS NULL OR keep >= 1),
        added_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
    );

    ALTER TABLE archives ADD COLUMN folder_id INTEGER
        REFERENCES folders (id) ON DELETE SET NULL;

    CREATE INDEX archives_folder_id ON archives (folder_id);
    "#,
];

/// One row of the `folders` table: a folder `vmanager run` watches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderRecord {
    pub id: i64,
    /// The folder holding one tool's version subfolders.
    pub path: PathBuf,
    /// How many versions to keep unpacked, overriding the configured default.
    pub keep: Option<u32>,
    /// UTC, ISO-8601, written by SQLite.
    pub added_at: String,
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

    /// Record a finished archive, attributing it to a registered folder.
    pub fn record_archive(&self, summary: &Summary, folder_id: Option<i64>) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO archives (source_path, archive_path, file_count, source_bytes, archive_bytes, folder_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (archive_path) DO UPDATE SET
                 source_path   = excluded.source_path,
                 file_count    = excluded.file_count,
                 source_bytes  = excluded.source_bytes,
                 archive_bytes = excluded.archive_bytes,
                 folder_id     = excluded.folder_id,
                 created_at    = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')",
            params![
                summary.source.to_string_lossy(),
                summary.archive.to_string_lossy(),
                // SQLite integers are signed; the CHECK constraints keep
                // these non-negative on the way back out.
                summary.file_count as i64,
                summary.source_bytes as i64,
                summary.archive_bytes as i64,
                folder_id,
            ],
        )?;

        Ok(self.conn.last_insert_rowid())
    }

    /// Register a folder for `vmanager run` to sweep.
    ///
    /// Registering a folder that is already known updates its `keep` override
    /// instead of failing; the returned flag says which happened, so the
    /// command can report "registered" or "updated" accurately.
    pub fn add_folder(&self, path: &Path, keep: Option<u32>) -> Result<(i64, bool)> {
        let existing = self.folder_by_path(path)?;

        self.conn.execute(
            "INSERT INTO folders (path, keep) VALUES (?1, ?2)
             ON CONFLICT (path) DO UPDATE SET keep = excluded.keep",
            params![path.to_string_lossy(), keep],
        )?;

        match existing {
            Some(record) => Ok((record.id, false)),
            None => Ok((self.conn.last_insert_rowid(), true)),
        }
    }

    /// Every registered folder, in registration order.
    pub fn folders(&self) -> Result<Vec<FolderRecord>> {
        let mut statement = self
            .conn
            .prepare("SELECT id, path, keep, added_at FROM folders ORDER BY id")?;

        let rows = statement.query_map([], row_to_folder)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Look up a registered folder by path.
    pub fn folder_by_path(&self, path: &Path) -> Result<Option<FolderRecord>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, path, keep, added_at FROM folders WHERE path = ?1",
                [path.to_string_lossy()],
                row_to_folder,
            )
            .optional()?)
    }

    /// Unregister a folder. Nothing on disk is touched, and the archives it
    /// produced keep their records — they simply lose the back-reference.
    pub fn remove_folder(&self, path: &Path) -> Result<FolderRecord> {
        let record = self.folder_by_path(path)?.ok_or_else(|| Error::NotFound {
            kind: "registered folder",
            name: path.display().to_string(),
        })?;

        self.conn
            .execute("DELETE FROM folders WHERE id = ?1", [record.id])?;
        Ok(record)
    }

    /// How many archives were created from a registered folder.
    pub fn archive_count_for_folder(&self, folder_id: i64) -> Result<u64> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM archives WHERE folder_id = ?1",
            [folder_id],
            |row| row.get::<_, i64>(0),
        )? as u64)
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

fn row_to_folder(row: &rusqlite::Row<'_>) -> rusqlite::Result<FolderRecord> {
    Ok(FolderRecord {
        id: row.get(0)?,
        path: PathBuf::from(row.get::<_, String>(1)?),
        keep: row.get::<_, Option<i64>>(2)?.map(|keep| keep as u32),
        added_at: row.get(3)?,
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

    /// Register a folder and record one archive against it.
    fn folder_with_archive(db: &Database, path: &str, archive: &str) -> i64 {
        let (id, _) = db.add_folder(Path::new(path), None).unwrap();
        db.record_archive(&summary(archive), Some(id)).unwrap();
        id
    }

    #[test]
    fn fresh_database_is_fully_migrated() {
        let db = Database::open_in_memory().unwrap();
        assert_eq!(db.schema_version().unwrap(), Database::target_version());
        assert!(db.folders().unwrap().is_empty());
    }

    #[test]
    fn opening_twice_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("vmanager.db");

        let first = Database::open(&path).unwrap();
        let id = folder_with_archive(&first, "/tools/blender", "/tools/blender/1.0.7z");
        drop(first);

        let second = Database::open(&path).unwrap();
        assert_eq!(second.schema_version().unwrap(), Database::target_version());
        assert_eq!(second.folders().unwrap().len(), 1);
        assert_eq!(second.archive_count_for_folder(id).unwrap(), 1);
    }

    #[test]
    fn re_archiving_the_same_path_updates_the_row() {
        let db = Database::open_in_memory().unwrap();
        let id = folder_with_archive(&db, "/tools/blender", "/tools/blender/1.0.7z");

        // A forced re-run writes the same archive path again; that must update
        // the existing row rather than accumulate duplicates.
        let mut second = summary("/tools/blender/1.0.7z");
        second.file_count = 99;
        db.record_archive(&second, Some(id)).unwrap();

        assert_eq!(db.archive_count_for_folder(id).unwrap(), 1);
    }

    #[test]
    fn archives_are_counted_per_folder() {
        let db = Database::open_in_memory().unwrap();
        let blender = folder_with_archive(&db, "/tools/blender", "/tools/blender/1.0.7z");
        let godot = folder_with_archive(&db, "/tools/godot", "/tools/godot/3.5.7z");
        db.record_archive(&summary("/tools/blender/2.0.7z"), Some(blender))
            .unwrap();

        assert_eq!(db.archive_count_for_folder(blender).unwrap(), 2);
        assert_eq!(db.archive_count_for_folder(godot).unwrap(), 1);
    }

    #[test]
    fn registers_and_lists_folders() {
        let db = Database::open_in_memory().unwrap();
        let (id, inserted) = db.add_folder(Path::new("/tools/blender"), Some(3)).unwrap();
        assert!(inserted);

        let folders = db.folders().unwrap();
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].id, id);
        assert_eq!(folders[0].path, PathBuf::from("/tools/blender"));
        assert_eq!(folders[0].keep, Some(3));
    }

    #[test]
    fn registering_a_known_folder_updates_its_keep() {
        let db = Database::open_in_memory().unwrap();
        let (first, _) = db.add_folder(Path::new("/tools/godot"), Some(2)).unwrap();

        let (second, inserted) = db.add_folder(Path::new("/tools/godot"), Some(5)).unwrap();
        assert_eq!(first, second, "the row should be reused, not duplicated");
        assert!(!inserted);
        assert_eq!(db.folders().unwrap().len(), 1);
        assert_eq!(db.folders().unwrap()[0].keep, Some(5));

        // Clearing the override is an update too, not a no-op.
        db.add_folder(Path::new("/tools/godot"), None).unwrap();
        assert_eq!(db.folders().unwrap()[0].keep, None);
    }

    #[test]
    fn removing_an_unregistered_folder_is_not_found() {
        let db = Database::open_in_memory().unwrap();
        assert!(matches!(
            db.remove_folder(Path::new("/tools/absent")),
            Err(Error::NotFound {
                kind: "registered folder",
                ..
            })
        ));
    }

    #[test]
    fn unregistering_orphans_its_archives_instead_of_dropping_them() {
        let db = Database::open_in_memory().unwrap();
        let id = folder_with_archive(&db, "/tools/blender", "/tools/blender/4.1.7z");
        assert_eq!(db.archive_count_for_folder(id).unwrap(), 1);

        db.remove_folder(Path::new("/tools/blender")).unwrap();

        // ON DELETE SET NULL rather than CASCADE: the .7z is still on disk, so
        // its record should survive the registration that produced it.
        assert_eq!(db.archive_count_for_folder(id).unwrap(), 0);
        let orphans: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM archives WHERE folder_id IS NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(orphans, 1);
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
