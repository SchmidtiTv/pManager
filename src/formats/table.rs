//! Renders console data as space-padded, aligned columns.

use crate::formats::console_data::{Cell, ConsoleData};

// Headers alone carry no information, so an empty table renders to nothing at
// all rather than to a lone header line.
pub fn render(data: &ConsoleData) -> String {
    if data.rows.is_empty() {
        return String::new();
    }

    let rows: Vec<Vec<String>> = data
        .rows
        .iter()
        .map(|row| row.iter().map(Cell::to_string).collect())
        .collect();
    let widths = column_widths(&data.headers, &rows);

    std::iter::once(data.headers.clone())
        .chain(rows)
        .map(|row| format_row(&row, &widths))
        .collect::<Vec<String>>()
        .join("\n")
}

fn column_widths(headers: &[String], rows: &[Vec<String>]) -> Vec<usize> {
    let mut widths: Vec<usize> = headers
        .iter()
        .map(|header| header.chars().count())
        .collect();

    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }

    widths
}

// The last column stays unpadded: it is the one most likely to be long enough
// to wrap, and wrapping the final column damages no alignment.
fn format_row(row: &[String], widths: &[usize]) -> String {
    let mut line = String::new();

    for (index, (cell, width)) in row.iter().zip(widths).enumerate() {
        if index + 1 == row.len() {
            line.push_str(cell);
        } else {
            line.push_str(&format!("{cell:<width$}  "));
        }
    }

    line
}

#[cfg(test)]
mod tests {
    use super::render;
    use crate::formats::console_data::{Cell, ConsoleData};

    fn data(rows: Vec<Vec<&str>>) -> ConsoleData {
        ConsoleData {
            headers: vec!["NAME".to_string(), "KIND".to_string(), "PATH".to_string()],
            rows: rows
                .into_iter()
                .map(|row| row.into_iter().map(|c| Cell::Text(c.to_string())).collect())
                .collect(),
        }
    }

    #[test]
    fn columns_line_up_across_rows_of_different_widths() {
        let output = render(&data(vec![
            vec!["a", "rust", "C:/a"],
            vec!["much-longer-name", "rust", "C:/b"],
        ]));
        let lines: Vec<&str> = output.lines().collect();

        let kind_column = lines[0].find("KIND").expect("header is present");
        assert_eq!(lines[1].find("rust"), Some(kind_column));
        assert_eq!(lines[2].find("rust"), Some(kind_column));
    }

    #[test]
    fn the_last_column_is_unpadded_so_a_long_value_cannot_misalign_the_rest() {
        let long = "C:/a/very/long/path/that/keeps/going/and/going/and/going";
        let output = render(&data(vec![
            vec!["a", "rust", long],
            vec!["b", "rust", "C:/b"],
        ]));
        let lines: Vec<&str> = output.lines().collect();

        assert!(lines[1].ends_with(long));
        assert!(lines[2].ends_with("C:/b"));
    }

    #[test]
    fn no_rows_renders_nothing_rather_than_a_lone_header_line() {
        let output = render(&ConsoleData {
            headers: vec!["NAME".to_string()],
            rows: Vec::new(),
        });

        assert!(output.is_empty(), "got: {output:?}");
    }

    #[test]
    fn a_day_count_carries_its_unit_and_flags_render_as_words() {
        let output = render(&ConsoleData {
            headers: vec!["IDLE".to_string(), "GIT".to_string()],
            rows: vec![vec![Cell::Days(10), Cell::Bool(true)]],
        });

        assert!(output.lines().nth(1).unwrap().contains("10d   true"));
    }
}
