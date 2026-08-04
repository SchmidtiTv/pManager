//! SQLite storage.
//!
//! The database is a single file under the data directory (see
//! [`crate::config::Config::database_path`]). SQLite is linked in statically
//! via `rusqlite`'s `bundled` feature, so there is no system dependency and no
//! version skew between machines.
//!
//! One table: `projects`. It records where each project is in its life, when
//! it got there, and where an archived one's `.7z` went — the only part that
//! cannot be re-derived from the disk.
//!
//! The database is a record, not the source of truth. Deleting it loses the
//! registrations, not a single project.
//!
//! Schema changes are additive migrations tracked by SQLite's own
//! `user_version` pragma: append to [`MIGRATIONS`], never edit a shipped
//! entry, and every existing database upgrades itself on the next open.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use rusqlite::{Connection, OptionalExtension, params};

use crate::archive::Summary;
use crate::error::{Error, Result};
use crate::lifecycle::State;
use crate::scan;

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
    // v2 — the registry of folders the version sweep walked, and the link from
    // an archive back to the folder whose version it held.
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
    // v3 — pManager manages projects rather than tool versions. The old tables
    // described versions of a tool, a concept this build no longer has, so
    // there is nothing to carry across. Timestamps are unix seconds here; see
    // `scan::to_epoch`.
    r#"
    DROP TABLE archives;
    DROP TABLE folders;

    CREATE TABLE projects (
        id            INTEGER PRIMARY KEY,
        path          TEXT    NOT NULL UNIQUE,
        name          TEXT    NOT NULL,
        state         TEXT    NOT NULL DEFAULT 'active'
                              CHECK (state IN ('active', 'trimmed', 'archived')),
        added_at      INTEGER NOT NULL,
        trimmed_at    INTEGER,
        archived_at   INTEGER,
        restored_at   INTEGER,
        freed_bytes   INTEGER NOT NULL DEFAULT 0 CHECK (freed_bytes >= 0),
        archive_path  TEXT,
        archive_bytes INTEGER CHECK (archive_bytes  IS NULL OR archive_bytes  >= 0),
        source_bytes  INTEGER CHECK (source_bytes   IS NULL OR source_bytes   >= 0),
        file_count    INTEGER CHECK (file_count     IS NULL OR file_count     >= 0)
    );

    CREATE INDEX projects_state ON projects (state);
    "#,
];

/// Where an archived project went, and what it held. The size of the `.7z` is
/// on disk to be read; what went *into* it is only known here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveRecord {
    pub path: PathBuf,
    /// Size of the archive when it was written.
    pub bytes: u64,
    /// Size of the project before compression.
    pub source_bytes: u64,
    /// Files stored in the archive.
    pub file_count: u64,
}

/// One row of the `projects` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRecord {
    pub id: i64,
    /// The project folder. Still the identity of an archived project, even
    /// though nothing is there any more — it is where `restore` puts it back.
    pub path: PathBuf,
    /// Display name: `package.json`'s `name` when there was one, else the
    /// folder name.
    pub name: String,
    pub state: State,
    /// Unix seconds; see the note on migration v3.
    pub added_at: i64,
    pub trimmed_at: Option<i64>,
    pub archived_at: Option<i64>,
    pub restored_at: Option<i64>,
    /// Bytes reclaimed by trimming, across every run.
    pub freed_bytes: u64,
    /// Present only while the project is archived.
    pub archive: Option<ArchiveRecord>,
}

impl ProjectRecord {
    /// The last time pManager itself saw this project used. Feeds
    /// [`crate::lifecycle::decide`] as a floor under the idle clock.
    pub fn active_since(&self) -> Option<SystemTime> {
        self.restored_at.map(scan::from_epoch)
    }
}

/// An open connection to the pManager database.
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

    /// Track a project. Re-adding a tracked one refreshes its name; the flag
    /// says which happened.
    pub fn add_project(&self, path: &Path, name: &str) -> Result<(ProjectRecord, bool)> {
        let existing = self.project_by_path(path)?;

        self.conn.execute(
            "INSERT INTO projects (path, name, added_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (path) DO UPDATE SET name = excluded.name",
            params![path.to_string_lossy(), name, scan::now_epoch()],
        )?;

        let record = self
            .project_by_path(path)?
            .ok_or_else(|| Error::Database("project vanished after being inserted".into()))?;
        Ok((record, existing.is_none()))
    }

    /// Every tracked project, by name.
    pub fn projects(&self) -> Result<Vec<ProjectRecord>> {
        let mut statement = self
            .conn
            .prepare(&format!("{SELECT} ORDER BY name COLLATE NOCASE, path"))?;

        let rows = statement.query_map([], row_to_project)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Look up a tracked project by path.
    pub fn project_by_path(&self, path: &Path) -> Result<Option<ProjectRecord>> {
        Ok(self
            .conn
            .query_row(
                &format!("{SELECT} WHERE path = ?1"),
                [path.to_string_lossy()],
                row_to_project,
            )
            .optional()?)
    }

    /// Find a project by path or by name. The name is how an archived project
    /// is reached, its folder being gone. Two projects sharing a name is an
    /// error rather than a guess.
    pub fn find_project(&self, query: &str) -> Result<ProjectRecord> {
        let as_path = crate::archive::canonical_dir(Path::new(query))
            .unwrap_or_else(|_| PathBuf::from(query));
        if let Some(record) = self.project_by_path(&as_path)? {
            return Ok(record);
        }

        let mut statement = self
            .conn
            .prepare(&format!("{SELECT} WHERE name = ?1 COLLATE NOCASE"))?;
        let matches = statement
            .query_map([query], row_to_project)?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        match matches.len() {
            1 => Ok(matches.into_iter().next().expect("just counted")),
            0 => Err(Error::NotFound {
                kind: "project",
                name: query.to_string(),
            }),
            _ => {
                let paths: Vec<String> = matches
                    .iter()
                    .map(|record| format!("\n  {}", record.path.display()))
                    .collect();
                Err(Error::Other(anyhow::anyhow!(
                    "`{query}` matches {} projects; name one by path:{}",
                    matches.len(),
                    paths.concat()
                )))
            }
        }
    }

    /// Stop tracking a project. Nothing on disk is touched.
    pub fn remove_project(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM projects WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Record that a project's dependency trees were deleted. `freed`
    /// accumulates across runs.
    pub fn mark_trimmed(&self, id: i64, freed: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE projects
                SET state = 'trimmed', trimmed_at = ?2, freed_bytes = freed_bytes + ?3
              WHERE id = ?1",
            params![id, scan::now_epoch(), freed as i64],
        )?;
        Ok(())
    }

    /// Record that a project was compressed and its folder removed.
    pub fn mark_archived(&self, id: i64, summary: &Summary) -> Result<()> {
        self.conn.execute(
            "UPDATE projects
                SET state = 'archived', archived_at = ?2, archive_path = ?3,
                    archive_bytes = ?4, source_bytes = ?5, file_count = ?6
              WHERE id = ?1",
            params![
                id,
                scan::now_epoch(),
                summary.archive.to_string_lossy(),
                // SQLite integers are signed; the CHECK constraints keep these
                // non-negative on the way back out.
                summary.archive_bytes as i64,
                summary.source_bytes as i64,
                summary.file_count as i64,
            ],
        )?;
        Ok(())
    }

    /// Record that an archived project was extracted and is live again. The
    /// row follows the project when `--into` puts it somewhere new.
    pub fn mark_restored(&self, id: i64, path: &Path) -> Result<()> {
        self.conn.execute(
            "UPDATE projects
                SET state = 'active', path = ?2, restored_at = ?3,
                    archived_at = NULL, archive_path = NULL,
                    archive_bytes = NULL, source_bytes = NULL, file_count = NULL
              WHERE id = ?1",
            params![id, path.to_string_lossy(), scan::now_epoch()],
        )?;
        Ok(())
    }

    /// Record that a trimmed project is being worked on again.
    pub fn mark_active(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE projects SET state = 'active', trimmed_at = NULL WHERE id = ?1",
            [id],
        )?;
        Ok(())
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
                 upgrade pmanager",
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

/// Column list shared by every project query, so the row reader can rely on
/// the order.
const SELECT: &str = "SELECT id, path, name, state, added_at, trimmed_at, archived_at,
                             restored_at, freed_bytes, archive_path, archive_bytes,
                             source_bytes, file_count
                        FROM projects";

fn row_to_project(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectRecord> {
    let archive_path: Option<String> = row.get(9)?;

    Ok(ProjectRecord {
        id: row.get(0)?,
        path: PathBuf::from(row.get::<_, String>(1)?),
        name: row.get(2)?,
        state: State::parse(&row.get::<_, String>(3)?),
        added_at: row.get(4)?,
        trimmed_at: row.get(5)?,
        archived_at: row.get(6)?,
        restored_at: row.get(7)?,
        freed_bytes: row.get::<_, i64>(8)? as u64,
        archive: archive_path.map(|path| ArchiveRecord {
            path: PathBuf::from(path),
            bytes: row.get::<_, Option<i64>>(10).unwrap_or(None).unwrap_or(0) as u64,
            source_bytes: row.get::<_, Option<i64>>(11).unwrap_or(None).unwrap_or(0) as u64,
            file_count: row.get::<_, Option<i64>>(12).unwrap_or(None).unwrap_or(0) as u64,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(archive: &str) -> Summary {
        Summary {
            source: PathBuf::from("/code/dashboard"),
            archive: PathBuf::from(archive),
            file_count: 120,
            source_bytes: 4_096,
            archive_bytes: 512,
        }
    }

    /// Track a project at `path` under `name`.
    fn track(db: &Database, path: &str, name: &str) -> ProjectRecord {
        db.add_project(Path::new(path), name).unwrap().0
    }

    #[test]
    fn fresh_database_is_fully_migrated() {
        let db = Database::open_in_memory().unwrap();
        assert_eq!(db.schema_version().unwrap(), Database::target_version());
        assert!(db.projects().unwrap().is_empty());
    }

    #[test]
    fn opening_twice_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("pmanager.db");

        let first = Database::open(&path).unwrap();
        track(&first, "/code/api", "api");
        drop(first);

        let second = Database::open(&path).unwrap();
        assert_eq!(second.schema_version().unwrap(), Database::target_version());
        assert_eq!(second.projects().unwrap().len(), 1);
    }

    #[test]
    fn a_database_from_the_version_manager_upgrades_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pmanager.db");

        // Stop at v2 — the schema as the tool-version sweep left it — and
        // reopen, which is what an existing installation does after upgrading.
        let conn = Connection::open(&path).unwrap();
        for (index, migration) in MIGRATIONS.iter().take(2).enumerate() {
            conn.execute_batch(migration).unwrap();
            conn.pragma_update(None, "user_version", (index + 1) as i64)
                .unwrap();
        }
        conn.execute("INSERT INTO folders (path) VALUES ('/tools/blender')", [])
            .unwrap();
        drop(conn);

        let db = Database::open(&path).unwrap();
        assert_eq!(db.schema_version().unwrap(), Database::target_version());
        assert!(db.projects().unwrap().is_empty());
    }

    #[test]
    fn tracking_a_known_project_updates_it_instead_of_duplicating() {
        let db = Database::open_in_memory().unwrap();
        let first = track(&db, "/code/api", "api");

        let (second, inserted) = db
            .add_project(Path::new("/code/api"), "api-server")
            .unwrap();
        assert_eq!(first.id, second.id, "the row should be reused");
        assert!(!inserted);
        assert_eq!(second.name, "api-server");
        assert_eq!(db.projects().unwrap().len(), 1);
    }

    #[test]
    fn a_new_project_starts_active_with_nothing_recorded() {
        let db = Database::open_in_memory().unwrap();
        let record = track(&db, "/code/api", "api");

        assert_eq!(record.state, State::Active);
        assert_eq!(record.freed_bytes, 0);
        assert_eq!(record.archive, None);
        assert_eq!(record.active_since(), None);
        assert!(record.added_at > 1_700_000_000, "{}", record.added_at);
    }

    #[test]
    fn trimming_accumulates_what_it_reclaimed() {
        let db = Database::open_in_memory().unwrap();
        let record = track(&db, "/code/api", "api");

        db.mark_trimmed(record.id, 400).unwrap();
        // Worked on again, dependencies reinstalled, abandoned again.
        db.mark_active(record.id).unwrap();
        db.mark_trimmed(record.id, 600).unwrap();

        let record = db.project_by_path(Path::new("/code/api")).unwrap().unwrap();
        assert_eq!(record.state, State::Trimmed);
        assert_eq!(record.freed_bytes, 1_000);
        assert!(record.trimmed_at.is_some());
    }

    #[test]
    fn archiving_records_what_the_archive_cannot_say_about_itself() {
        let db = Database::open_in_memory().unwrap();
        let record = track(&db, "/code/dashboard", "dashboard");
        db.mark_archived(record.id, &summary("/code/dashboard.7z"))
            .unwrap();

        let record = db.find_project("dashboard").unwrap();
        assert_eq!(record.state, State::Archived);
        assert!(record.archived_at.is_some());

        let archive = record.archive.expect("an archived project has an archive");
        assert_eq!(archive.path, PathBuf::from("/code/dashboard.7z"));
        assert_eq!(archive.bytes, 512);
        assert_eq!(archive.source_bytes, 4_096);
        assert_eq!(archive.file_count, 120);
    }

    #[test]
    fn restoring_clears_the_archive_and_starts_the_clock() {
        let db = Database::open_in_memory().unwrap();
        let record = track(&db, "/code/dashboard", "dashboard");
        db.mark_archived(record.id, &summary("/code/dashboard.7z"))
            .unwrap();

        db.mark_restored(record.id, Path::new("/elsewhere/dashboard"))
            .unwrap();

        let record = db.find_project("dashboard").unwrap();
        assert_eq!(record.state, State::Active);
        assert_eq!(record.path, PathBuf::from("/elsewhere/dashboard"));
        assert_eq!(
            record.archive, None,
            "the .7z is gone, so the record is too"
        );
        assert_eq!(
            db.projects().unwrap().len(),
            1,
            "restoring elsewhere moves the row rather than adding one"
        );

        // This is what stops the next run archiving it straight back.
        let since = record.active_since().expect("restored just now");
        assert!(scan::age(since) < Duration::from_secs(60));
    }

    #[test]
    fn a_project_is_found_by_name_or_by_path() {
        let db = Database::open_in_memory().unwrap();
        track(&db, "/code/dashboard", "dashboard");

        assert_eq!(db.find_project("dashboard").unwrap().name, "dashboard");
        assert_eq!(db.find_project("DashBoard").unwrap().name, "dashboard");
        assert_eq!(
            db.find_project("/code/dashboard").unwrap().path,
            PathBuf::from("/code/dashboard")
        );
    }

    #[test]
    fn an_unknown_project_is_not_found() {
        let db = Database::open_in_memory().unwrap();
        assert!(matches!(
            db.find_project("nothing"),
            Err(Error::NotFound {
                kind: "project",
                ..
            })
        ));
    }

    #[test]
    fn an_ambiguous_name_asks_for_a_path_rather_than_guessing() {
        let db = Database::open_in_memory().unwrap();
        track(&db, "/code/one/api", "api");
        track(&db, "/code/two/api", "api");

        let err = db.find_project("api").unwrap_err();
        let message = err.to_string();
        assert!(message.contains("matches 2 projects"), "{message}");
        assert!(message.contains("/code/one/api"), "{message}");
        assert!(message.contains("/code/two/api"), "{message}");
    }

    #[test]
    fn untracking_leaves_the_others_alone() {
        let db = Database::open_in_memory().unwrap();
        let api = track(&db, "/code/api", "api");
        track(&db, "/code/web", "web");

        db.remove_project(api.id).unwrap();

        let remaining = db.projects().unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].name, "web");
    }

    #[test]
    fn projects_are_listed_by_name_regardless_of_case() {
        let db = Database::open_in_memory().unwrap();
        track(&db, "/code/zeta", "zeta");
        track(&db, "/code/alpha", "Alpha");
        track(&db, "/code/mid", "mid");

        let names: Vec<String> = db
            .projects()
            .unwrap()
            .into_iter()
            .map(|record| record.name)
            .collect();
        assert_eq!(names, ["Alpha", "mid", "zeta"]);
    }

    #[test]
    fn a_database_from_the_future_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pmanager.db");

        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", 999_i64).unwrap();
        drop(conn);

        let err = Database::open(&path).unwrap_err();
        assert!(matches!(err, Error::Database(_)), "{err}");
        assert!(err.to_string().contains("upgrade pmanager"), "{err}");
    }
}
