//! `Cx` — the single value every command receives.

use std::cell::OnceCell;

use crate::database::schema;
use crate::database::sql_lite::SqlLite;
use crate::error::Result;
use crate::formats::console_data::ViewType;
use crate::paths::Paths;

pub struct Cx {
    pub paths: Paths,
    pub view: ViewType,
    db: OnceCell<SqlLite>,
}

impl Cx {
    pub fn new(paths: Paths, view: ViewType) -> Self {
        Self {
            paths,
            view,
            db: OnceCell::new(),
        }
    }

    pub fn db(&self) -> Result<&SqlLite> {
        if let Some(db) = self.db.get() {
            return Ok(db);
        }

        let db = SqlLite::open(&self.paths.db)?;
        schema::apply(&db)?;
        Ok(self.db.get_or_init(|| db))
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::types::Value;

    use super::Cx;
    use crate::database::sql_builder::SqlBuilder;
    use crate::formats::console_data::ViewType;
    use crate::paths::Paths;
    use std::path::Path;

    fn cx_in(dir: &Path) -> Cx {
        Cx::new(
            Paths::resolve_with(
                Some(dir.join("nested").join("pmanager.db")),
                Some(dir.join("strip")),
                Path::new("/unused"),
                Path::new("/unused"),
            ),
            ViewType::Table,
        )
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
        let cx = cx_in(tmp.path());

        cx.db().expect("opening the database should succeed");

        assert!(cx.paths.db.exists(), "database file should now exist");
    }

    #[test]
    fn the_connection_is_reused_across_calls() {
        let tmp = tempfile::tempdir().unwrap();
        let cx = cx_in(tmp.path());

        cx.db()
            .unwrap()
            .execute_batch("CREATE TEMP TABLE marker (id INTEGER)")
            .unwrap();

        let query = SqlBuilder::select(&["name"])
            .from_table("sqlite_temp_master")
            .and_where("name = ?", vec![Value::Text("marker".to_string())])
            .build()
            .unwrap();
        let found: Vec<String> = cx.db().unwrap().query(&query, |row| row.get(0)).unwrap();

        assert_eq!(
            found,
            vec!["marker".to_string()],
            "db() must hand back the same live connection"
        );
    }

    #[test]
    fn the_handle_and_the_paths_can_be_used_together() {
        let tmp = tempfile::tempdir().unwrap();
        let cx = cx_in(tmp.path());

        let db = cx.db().unwrap();
        let query = SqlBuilder::select(&["name"])
            .from_table("sqlite_master")
            .build()
            .unwrap();

        let tables: Vec<String> = db.query(&query, |row| row.get(0)).unwrap();
        assert!(tables.contains(&"projects".to_string()));
        assert!(cx.paths.db.exists());
    }
}
