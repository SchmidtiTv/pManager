//! The view-agnostic shape every command hands to the formatter.

use std::fmt;

use anyhow::ensure;
use clap::ValueEnum;

use crate::error::Result;
use crate::formats::{json, table};

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
    Bytes(u64),
}

impl fmt::Display for Cell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Cell::Text(text) => write!(f, "{text}"),
            Cell::Int(number) => write!(f, "{number}"),
            Cell::Bool(flag) => write!(f, "{flag}"),
            Cell::Days(days) => write!(f, "{days}d"),
            Cell::Bytes(bytes) => write!(f, "{}", human_size(*bytes)),
        }
    }
}

const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];

/// Powers of 1024, because the number being reported is disk space and that
/// is the unit a file manager shows for the same folder.
///
/// Whole bytes below the first step: `512 B` is exact where `0.5 KiB` is
/// both longer and vaguer.
fn human_size(bytes: u64) -> String {
    let mut size = bytes as f64;
    let mut unit = 0;

    while size >= 1024.0 && unit + 1 < UNITS.len() {
        size /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{bytes} {}", UNITS[0])
    } else {
        format!("{size:.1} {}", UNITS[unit])
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
    fn a_size_carries_a_unit_and_stays_exact_below_the_first_step() {
        assert_eq!(Cell::Bytes(0).to_string(), "0 B");
        assert_eq!(Cell::Bytes(512).to_string(), "512 B");
        assert_eq!(Cell::Bytes(1023).to_string(), "1023 B");
        assert_eq!(Cell::Bytes(1024).to_string(), "1.0 KiB");
        assert_eq!(Cell::Bytes(1536).to_string(), "1.5 KiB");
        assert_eq!(Cell::Bytes(3 * 1024 * 1024).to_string(), "3.0 MiB");
        assert_eq!(Cell::Bytes(u64::MAX).to_string(), "16777216.0 TiB");
    }

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
