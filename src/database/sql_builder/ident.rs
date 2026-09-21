//! Identifier quoting.
//!
//! psycopg's `sql.Identifier` did this for the Python original; against
//! SQLite it is ours to do, and getting it wrong is an injection hole.

use super::error::SqlBuilderError;

/// Quote a possibly dotted identifier: `main.items` becomes `"main"."items"`.
///
/// Embedded double quotes are doubled, which is what stops a caller-supplied
/// name from closing the quoted string and continuing as SQL.
pub(super) fn ident(name: &str) -> Result<String, SqlBuilderError> {
    let invalid = || SqlBuilderError::InvalidIdentifier {
        name: name.to_string(),
    };

    if name.is_empty() {
        return Err(invalid());
    }

    let mut quoted = String::with_capacity(name.len() + 4);
    for (index, part) in name.split('.').enumerate() {
        if part.is_empty() {
            return Err(invalid());
        }
        if index > 0 {
            quoted.push('.');
        }
        quoted.push('"');
        quoted.push_str(&part.replace('"', "\"\""));
        quoted.push('"');
    }

    Ok(quoted)
}

/// Quote each name and join them for a column list.
pub(super) fn ident_list(names: &[String]) -> Result<String, SqlBuilderError> {
    let quoted: Vec<String> = names
        .iter()
        .map(|name| ident(name))
        .collect::<Result<_, _>>()?;

    Ok(quoted.join(", "))
}

/// `?, ?, ?` for `count` bound values.
pub(super) fn placeholders(count: usize) -> String {
    vec!["?"; count].join(", ")
}
