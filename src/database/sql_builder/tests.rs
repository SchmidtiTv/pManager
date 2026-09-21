//! Every test runs its query against a real SQLite connection.
//!
//! Asserting on the generated string alone would happily pass on SQL that
//! SQLite rejects — which is the exact failure mode a dialect port produces.

use rusqlite::types::Value;
use rusqlite::{Connection, params_from_iter};

use super::{JoinKind, Query, SqlBuilder, SqlBuilderError};

fn text(value: &str) -> Value {
    Value::Text(value.to_string())
}

fn seeded_connection() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, age INTEGER, country TEXT);
         CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER, total INTEGER);
         INSERT INTO users VALUES (1, 'Finn', 30, 'DE'), (2, 'Ada', 17, 'DE'), (3, 'Bo', 40, 'NL');
         INSERT INTO orders VALUES (1, 1, 100), (2, 1, 50), (3, 3, 70);",
    )
    .unwrap();
    conn
}

fn first_column(conn: &Connection, query: &Query) -> Vec<String> {
    let mut stmt = conn.prepare(&query.sql).unwrap();
    let rows = stmt
        .query_map(params_from_iter(&query.params), |row| row.get(0))
        .unwrap();
    rows.map(|row| row.unwrap()).collect()
}

#[test]
fn select_filters_orders_and_limits() {
    let conn = seeded_connection();
    let query = SqlBuilder::select(&["name"])
        .from_table("users")
        .and_where("age > ?", vec![Value::Integer(18)])
        .and_where("country = ?", vec![text("DE")])
        .order_by_asc(&["name"])
        .limit(10)
        .build()
        .unwrap();

    assert_eq!(first_column(&conn, &query), vec!["Finn".to_string()]);
}

#[test]
fn select_joins_groups_and_filters_on_an_aggregate() {
    let conn = seeded_connection();
    let query = SqlBuilder::select(&["users.name"])
        .from_table("users")
        .join(JoinKind::Inner, "orders", "orders.user_id = users.id")
        .group_by(&["users.name"])
        .having("sum(orders.total) > ?", vec![Value::Integer(100)])
        .order_by_desc(&["users.name"])
        .build()
        .unwrap();

    assert_eq!(first_column(&conn, &query), vec!["Finn".to_string()]);
}

#[test]
fn select_all_projects_every_column() {
    let conn = seeded_connection();
    let query = SqlBuilder::select_all()
        .from_table("users")
        .build()
        .unwrap();

    let mut stmt = conn.prepare(&query.sql).unwrap();
    assert_eq!(stmt.column_count(), 4);
    assert_eq!(
        stmt.query_map(params_from_iter(&query.params), |row| row.get::<_, i64>(0))
            .unwrap()
            .count(),
        3
    );
}

/// WHERE and HAVING both bind values, and a swap between them would return
/// wrong rows rather than an error -- the one failure mode in this module
/// that SQLite cannot catch for us.
#[test]
fn where_params_bind_before_having_params() {
    let conn = seeded_connection();
    let query = SqlBuilder::select(&["users.name"])
        .from_table("users")
        .join(JoinKind::Inner, "orders", "orders.user_id = users.id")
        .and_where("users.age > ?", vec![Value::Integer(18)])
        .group_by(&["users.name"])
        .having("sum(orders.total) > ?", vec![Value::Integer(100)])
        .build()
        .unwrap();

    assert_eq!(
        query.params,
        vec![Value::Integer(18), Value::Integer(100)],
        "the age bound must precede the total bound"
    );
    assert_eq!(first_column(&conn, &query), vec!["Finn".to_string()]);
}

#[test]
fn offset_needs_a_limit_because_sqlite_has_no_standalone_offset() {
    let unsupported = SqlBuilder::select_all()
        .from_table("users")
        .offset(1)
        .build();
    assert_eq!(unsupported, Err(SqlBuilderError::OffsetWithoutLimit));

    let conn = seeded_connection();
    let query = SqlBuilder::select(&["name"])
        .from_table("users")
        .order_by_asc(&["id"])
        .limit(1)
        .offset(1)
        .build()
        .unwrap();

    assert_eq!(first_column(&conn, &query), vec!["Ada".to_string()]);
}

#[test]
fn where_in_binds_every_value_and_an_empty_set_matches_nothing() {
    let conn = seeded_connection();
    let matching = SqlBuilder::select(&["name"])
        .from_table("users")
        .and_where_in("country", vec![text("DE"), text("NL")])
        .order_by_asc(&["id"])
        .build()
        .unwrap();
    assert_eq!(first_column(&conn, &matching).len(), 3);

    let empty = SqlBuilder::select(&["name"])
        .from_table("users")
        .and_where_in("country", vec![])
        .build()
        .unwrap();
    assert!(first_column(&conn, &empty).is_empty());
}

/// Each condition is parenthesised, so an OR inside one does not swallow the
/// AND between them. The Python original omitted the parentheses.
#[test]
fn conditions_are_isolated_from_each_other() {
    let conn = seeded_connection();
    let query = SqlBuilder::select(&["name"])
        .from_table("users")
        .and_where("name = ? OR name = ?", vec![text("Finn"), text("Bo")])
        .and_where("country = ?", vec![text("NL")])
        .build()
        .unwrap();

    assert_eq!(first_column(&conn, &query), vec!["Bo".to_string()]);
}

#[test]
fn insert_writes_a_row() {
    let conn = seeded_connection();
    let query = SqlBuilder::insert("users")
        .value("name", text("Kim"))
        .value("age", Value::Integer(22))
        .build()
        .unwrap();

    let changed = conn
        .execute(&query.sql, params_from_iter(&query.params))
        .unwrap();
    assert_eq!(changed, 1);
}

#[test]
fn update_binds_set_values_before_where_values() {
    let conn = seeded_connection();
    let query = SqlBuilder::update("users")
        .set("country", text("AT"))
        .and_where("id = ?", vec![Value::Integer(1)])
        .build()
        .unwrap();

    conn.execute(&query.sql, params_from_iter(&query.params))
        .unwrap();

    let country: String = conn
        .query_row("SELECT country FROM users WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(country, "AT");
}

#[test]
fn delete_removes_only_the_matching_rows() {
    let conn = seeded_connection();
    let query = SqlBuilder::delete("users")
        .and_where("age < ?", vec![Value::Integer(18)])
        .build()
        .unwrap();

    let removed = conn
        .execute(&query.sql, params_from_iter(&query.params))
        .unwrap();
    assert_eq!(removed, 1);
}

#[test]
fn update_and_delete_refuse_to_touch_every_row() {
    assert_eq!(
        SqlBuilder::update("users")
            .set("age", Value::Integer(1))
            .build(),
        Err(SqlBuilderError::MissingWhere {
            statement: "update()"
        })
    );
    assert_eq!(
        SqlBuilder::delete("users").build(),
        Err(SqlBuilderError::MissingWhere {
            statement: "delete()"
        })
    );
}

#[test]
fn incomplete_statements_are_rejected() {
    assert_eq!(
        SqlBuilder::insert("users").build(),
        Err(SqlBuilderError::NoInsertValues)
    );
    assert_eq!(
        SqlBuilder::select(&[]).from_table("users").build(),
        Err(SqlBuilderError::NoColumns)
    );
    assert_eq!(
        SqlBuilder::insert("users")
            .value("name", text("a"))
            .value("name", text("b"))
            .build(),
        Err(SqlBuilderError::DuplicateColumn {
            column: "name".to_string()
        })
    );
}

/// psycopg escaped identifiers for the Python original; here it is our job,
/// and a column name is the one place a caller's string reaches the statement
/// outside a parameter.
#[test]
fn identifiers_are_quoted_and_escaped() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE odd (\"we\"\"ird\" TEXT); INSERT INTO odd VALUES ('x');")
        .unwrap();

    let query = SqlBuilder::select(&["we\"ird"])
        .from_table("odd")
        .build()
        .unwrap();
    assert_eq!(first_column(&conn, &query), vec!["x".to_string()]);

    assert_eq!(
        SqlBuilder::select(&["a..b"]).from_table("t").build(),
        Err(SqlBuilderError::InvalidIdentifier {
            name: "a..b".to_string()
        })
    );
}

#[test]
fn a_dotted_name_is_quoted_per_segment() {
    let conn = seeded_connection();
    let query = SqlBuilder::select(&["users.name"])
        .from_table("main.users")
        .distinct()
        .order_by_asc(&["users.name"])
        .build()
        .unwrap();

    assert_eq!(first_column(&conn, &query).len(), 3);
}
