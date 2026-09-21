//! The view-agnostic shape every command hands to the formatter.

use std::fmt;

use anyhow::ensure;
use clap::ValueEnum;

use crate::error::Result;
use crate::formattes::{json, table};

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewType {
    Json,
    Table,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cell {
    Text(String),
    Int(i64),
    Bool(bool),
    Days(i64),
}

impl fmt::Display for Cell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Cell::Text(text) => write!(f, "{text}"),
            Cell::Int(number) => write!(f, "{number}"),
            Cell::Bool(flag) => write!(f, "{flag}"),
            Cell::Days(days) => write!(f, "{days}d"),
        }
    }
}

pub struct ConsoleData {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<Cell>>,
}

pub fn build_output(data: &ConsoleData, view: ViewType) -> Result<String> {
    for (index, row) in data.rows.iter().enumerate() {
        ensure!(
            row.len() == data.headers.len(),
            "row {index} has {} cells but there are {} headers ({:?})",
            row.len(),
            data.headers.len(),
            data.headers
        );
    }

    match view {
        ViewType::Json => json::render(data),
        ViewType::Table => Ok(table::render(data)),
    }
}

#[cfg(test)]
mod tests {
    use super::{Cell, ConsoleData, ViewType, build_output};

    #[test]
    fn a_row_that_does_not_match_the_headers_is_rejected() {
        let data = ConsoleData {
            headers: vec!["NAME".to_string(), "KIND".to_string()],
            rows: vec![vec![Cell::Text("a".to_string())]],
        };

        let err = build_output(&data, ViewType::Table).unwrap_err();

        assert!(
            format!("{err:#}").contains("row 0 has 1 cells but there are 2 headers"),
            "got: {err:#}"
        );
    }
}
