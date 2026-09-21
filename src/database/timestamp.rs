use chrono::{DateTime, Utc};
use rusqlite::ToSql;
use rusqlite::types::{ToSqlOutput, Value};

// Delegating to rusqlite's own ToSql keeps what is written identical to what
// `row.get::<_, DateTime<Utc>>()` parses back.
pub fn to_value(at: DateTime<Utc>) -> Value {
    match at
        .to_sql()
        .expect("chrono's ToSql for DateTime<Utc> cannot fail")
    {
        ToSqlOutput::Owned(value) => value,
        ToSqlOutput::Borrowed(value) => {
            Value::try_from(value).expect("a timestamp is SQLite-representable")
        }
        other => panic!("rusqlite encoded a timestamp as {other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone as _, Utc};

    use super::to_value;
    use crate::database::sql_builder::Query;
    use crate::database::sql_lite::SqlLite;

    #[test]
    fn a_timestamp_round_trips_through_sqlite() {
        let db = SqlLite::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE t (at TEXT NOT NULL)")
            .unwrap();

        let at = Utc.with_ymd_and_hms(2026, 9, 21, 10, 30, 0).unwrap();
        db.execute(&Query {
            sql: "INSERT INTO t (at) VALUES (?);".to_string(),
            params: vec![to_value(at)],
        })
        .unwrap();

        let stored: Vec<chrono::DateTime<Utc>> = db
            .query(
                &Query {
                    sql: "SELECT at FROM t;".to_string(),
                    params: Vec::new(),
                },
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(stored, vec![at]);
    }
}
