//! Everything the builder can refuse to do, and why.

use thiserror::Error;

/// A statement that could not be assembled. Every variant names the call the
/// caller still owes, so the message is actionable without a stack trace.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SqlBuilderError {
    #[error("invalid identifier: {name:?}")]
    InvalidIdentifier { name: String },

    #[error("select() needs at least one column; use select_all() for `*`")]
    NoColumns,

    #[error("insert() requires at least one value(); call .value(...)")]
    NoInsertValues,

    #[error("update() requires at least one assignment; call .set(...)")]
    NoAssignments,

    #[error("column {column:?} was given twice in the same statement")]
    DuplicateColumn { column: String },

    #[error(
        "{statement} with no where() would affect every row; \
         call .and_where(\"1 = 1\", vec![]) if that is really intended"
    )]
    MissingWhere { statement: &'static str },

    // SQLite's grammar only accepts OFFSET as a suffix of LIMIT, unlike
    // PostgreSQL. Rejecting is better than silently emitting `LIMIT -1`.
    #[error("offset() requires limit(); SQLite has no standalone OFFSET")]
    OffsetWithoutLimit,
}
