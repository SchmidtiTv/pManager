use super::sql_lite::SqlLite;
use crate::error::Result;

const VERSION: u32 = 1;

const PROJECTS: &str = include_str!("../../docs/sql/schema.sql");

pub fn apply(db: &SqlLite) -> Result<()> {
    db.execute_batch(PROJECTS)?;
    db.execute_batch(&format!("PRAGMA user_version = {VERSION};"))
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, TimeZone as _, Utc};
    use rusqlite::types::Value;

    use super::{VERSION, apply};
    use crate::database::sql_builder::{Query, SqlBuilder};
    use crate::database::sql_lite::SqlLite;
    use crate::database::timestamp;

    fn applied() -> SqlLite {
        let db = SqlLite::open_in_memory().unwrap();
        apply(&db).unwrap();
        db
    }

    fn text(value: &str) -> Value {
        Value::Text(value.to_string())
    }

    fn sample_time() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 21, 10, 0, 0).unwrap()
    }

    fn registration(name: &str, path: &str) -> Query {
        SqlBuilder::insert("projects")
            .value("name", text(name))
            .value("path", text(path))
            .value("kind", text("rust"))
            .value("state", text("registered"))
            .value("has_git_repo", Value::Integer(1))
            .value("last_change_at", timestamp::to_value(sample_time()))
            .value("registered_at", timestamp::to_value(sample_time()))
            .build()
            .unwrap()
    }

    /// `Cx::db()` applies the schema on every open, so a second run over an
    /// existing ledger must be a no-op rather than an error.
    #[test]
    fn applying_the_schema_twice_is_harmless() {
        let db = applied();
        apply(&db).expect("a second apply must not fail");

        db.execute(&registration("pManager", "C:/dev/rust/pManager"))
            .expect("the table should still be usable");
    }

    #[test]
    fn a_project_round_trips() {
        let db = applied();
        db.execute(&registration("pManager", "C:/dev/rust/pManager"))
            .unwrap();

        let query = SqlBuilder::select(&["path", "state"])
            .from_table("projects")
            .and_where("name = ?", vec![text("pManager")])
            .build()
            .unwrap();
        let rows: Vec<(String, String)> = db
            .query(&query, |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap();

        assert_eq!(
            rows,
            vec![("C:/dev/rust/pManager".to_string(), "registered".to_string())]
        );
    }

    /// `restore <name>` and `project remove <path>` both look a project up by
    /// a single value, so neither may identify two rows.
    #[test]
    fn names_and_paths_are_each_unique() {
        let db = applied();
        db.execute(&registration("pManager", "C:/dev/rust/pManager"))
            .unwrap();

        let same_name = db.execute(&registration("pManager", "C:/dev/rust/other"));
        assert!(same_name.is_err(), "a duplicate name must be rejected");

        let same_path = db.execute(&registration("other", "C:/dev/rust/pManager"));
        assert!(same_path.is_err(), "a duplicate path must be rejected");
    }

    #[test]
    fn an_unknown_state_is_rejected() {
        let db = applied();
        let query = SqlBuilder::insert("projects")
            .value("name", text("pManager"))
            .value("path", text("C:/dev/rust/pManager"))
            .value("kind", text("rust"))
            .value("state", text("hibernating"))
            .value("has_git_repo", Value::Integer(0))
            .value("last_change_at", timestamp::to_value(sample_time()))
            .value("registered_at", timestamp::to_value(sample_time()))
            .build()
            .unwrap();

        assert!(db.execute(&query).is_err());
    }

    #[test]
    fn timestamps_read_back_as_typed_values() {
        let db = applied();
        db.execute(&registration("pManager", "C:/dev/rust/pManager"))
            .unwrap();

        let query = SqlBuilder::select(&["last_change_at"])
            .from_table("projects")
            .build()
            .unwrap();
        let stored: Vec<DateTime<Utc>> = db.query(&query, |row| row.get(0)).unwrap();

        assert_eq!(stored, vec![sample_time()]);
    }

    // The 60/120/160-day thresholds are computed from this column, so the
    // stored format has to stay legible to SQLite's own date functions.
    #[test]
    fn sqlite_can_measure_the_age_of_a_stored_timestamp() {
        let db = applied();
        db.execute(&registration("pManager", "C:/dev/rust/pManager"))
            .unwrap();

        let query = Query {
            sql: "SELECT julianday('now') - julianday(last_change_at) FROM projects;".to_string(),
            params: Vec::new(),
        };
        let days: Vec<f64> = db.query(&query, |row| row.get(0)).unwrap();

        assert_eq!(days.len(), 1);
        assert!(days[0] > 0.0, "a past timestamp should have a positive age");
    }

    /// The README's rule, enforced by the table: an archived project is the
    /// only kind that has a `.pproject` to point at.
    #[test]
    fn an_archive_path_must_accompany_the_archived_state() {
        let db = applied();
        db.execute(&registration("pManager", "C:/dev/rust/pManager"))
            .unwrap();

        let without_path = SqlBuilder::update("projects")
            .set("state", text("archived"))
            .and_where("name = ?", vec![text("pManager")])
            .build()
            .unwrap();
        assert!(
            db.execute(&without_path).is_err(),
            "archiving without an archive path must be rejected"
        );

        let with_path = SqlBuilder::update("projects")
            .set("state", text("archived"))
            .set("archive_path", text("C:/archive/pManager.pproject"))
            .and_where("name = ?", vec![text("pManager")])
            .build()
            .unwrap();
        assert_eq!(db.execute(&with_path).unwrap(), 1);
    }

    #[test]
    fn the_schema_version_is_recorded() {
        let db = applied();
        let query = Query {
            sql: "PRAGMA user_version;".to_string(),
            params: Vec::new(),
        };

        let versions: Vec<i64> = db.query(&query, |row| row.get(0)).unwrap();
        assert_eq!(versions, vec![i64::from(VERSION)]);
    }
}
