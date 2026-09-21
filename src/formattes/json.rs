//! Renders console data as a JSON array of one object per row.

use anyhow::Context as _;
use serde_json::{Map, Value};

use crate::error::Result;
use crate::formattes::console_data::{Cell, ConsoleData};

pub fn render(data: &ConsoleData) -> Result<String> {
    let rows: Vec<Value> = data
        .rows
        .iter()
        .map(|row| object(&data.headers, row))
        .collect();

    serde_json::to_string_pretty(&Value::Array(rows))
        .context("serialising the console output as JSON failed")
}

fn object(headers: &[String], row: &[Cell]) -> Value {
    let fields: Map<String, Value> = headers
        .iter()
        .zip(row)
        .map(|(header, cell)| (key(header), value(cell)))
        .collect();

    Value::Object(fields)
}

fn key(header: &str) -> String {
    header.to_lowercase().replace(' ', "_")
}

fn value(cell: &Cell) -> Value {
    match cell {
        Cell::Text(text) => Value::String(text.clone()),
        Cell::Int(number) => Value::Number((*number).into()),
        Cell::Bool(flag) => Value::Bool(*flag),
        // The "d" is a table suffix; JSON keeps the day count comparable.
        Cell::Days(days) => Value::Number((*days).into()),
    }
}

#[cfg(test)]
mod tests {
    use super::render;
    use crate::formattes::console_data::{Cell, ConsoleData};

    #[test]
    fn each_row_becomes_an_object_keyed_by_its_header() {
        let data = ConsoleData {
            headers: vec![
                "NAME".to_string(),
                "IDLE".to_string(),
                "SIZE KB".to_string(),
                "GIT".to_string(),
            ],
            rows: vec![vec![
                Cell::Text("pManager".to_string()),
                Cell::Days(10),
                Cell::Int(4096),
                Cell::Bool(true),
            ]],
        };

        let parsed: serde_json::Value = serde_json::from_str(&render(&data).unwrap()).unwrap();

        assert_eq!(parsed[0]["name"], serde_json::json!("pManager"));
        assert_eq!(parsed[0]["idle"], serde_json::json!(10));
        assert_eq!(parsed[0]["size_kb"], serde_json::json!(4096));
        assert_eq!(parsed[0]["git"], serde_json::json!(true));
    }

    #[test]
    fn no_rows_is_an_empty_array_not_an_error() {
        let data = ConsoleData {
            headers: vec!["NAME".to_string()],
            rows: Vec::new(),
        };

        assert_eq!(render(&data).unwrap(), "[]");
    }
}
