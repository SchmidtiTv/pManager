//! SELECT.

use rusqlite::types::Value;

use super::error::SqlBuilderError;
use super::filter::Filter;
use super::ident::{ident, ident_list};
use super::query::Query;

/// How two tables are joined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinKind {
    Inner,
    Left,
    Right,
    Full,
    Cross,
}

impl JoinKind {
    fn keyword(self) -> &'static str {
        match self {
            Self::Inner => "INNER JOIN",
            Self::Left => "LEFT JOIN",
            Self::Right => "RIGHT JOIN",
            Self::Full => "FULL JOIN",
            Self::Cross => "CROSS JOIN",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Asc,
    Desc,
}

struct Join {
    kind: JoinKind,
    table: String,
    on: String,
}

struct Order {
    column: String,
    direction: Direction,
}

/// What a SELECT projects.
pub(super) enum Columns {
    All,
    Named(Vec<String>),
}

impl Columns {
    fn render(&self) -> Result<String, SqlBuilderError> {
        match self {
            Self::All => Ok("*".to_string()),
            Self::Named(names) if names.is_empty() => Err(SqlBuilderError::NoColumns),
            Self::Named(names) => ident_list(names),
        }
    }
}

/// A projection that still needs a table. Exists so that a SELECT without a
/// FROM cannot be expressed at all, rather than failing at `build()`.
pub struct SelectColumns {
    columns: Columns,
}

impl SelectColumns {
    pub(super) fn new(columns: Columns) -> Self {
        Self { columns }
    }

    pub fn from_table(self, table: &str) -> Select {
        Select {
            columns: self.columns,
            table: table.to_string(),
            distinct: false,
            joins: Vec::new(),
            filter: Filter::new(),
            group_by: Vec::new(),
            having: Filter::new(),
            order_by: Vec::new(),
            limit: None,
            offset: None,
        }
    }
}

pub struct Select {
    columns: Columns,
    table: String,
    distinct: bool,
    joins: Vec<Join>,
    filter: Filter,
    group_by: Vec<String>,
    having: Filter,
    order_by: Vec<Order>,
    limit: Option<u32>,
    offset: Option<u32>,
}

impl Select {
    pub fn distinct(mut self) -> Self {
        self.distinct = true;
        self
    }

    /// `on` is raw SQL written by the caller and is not escaped.
    pub fn join(mut self, kind: JoinKind, table: &str, on: &str) -> Self {
        self.joins.push(Join {
            kind,
            table: table.to_string(),
            on: on.to_string(),
        });
        self
    }

    /// `condition` is raw SQL with `?` placeholders; `params` bind to them.
    pub fn and_where(mut self, condition: &str, params: Vec<Value>) -> Self {
        self.filter = self.filter.and(condition, params);
        self
    }

    pub fn and_where_in(mut self, column: &str, values: Vec<Value>) -> Self {
        self.filter = self.filter.and_in(column, values);
        self
    }

    pub fn group_by(mut self, columns: &[&str]) -> Self {
        self.group_by
            .extend(columns.iter().map(|column| column.to_string()));
        self
    }

    pub fn having(mut self, condition: &str, params: Vec<Value>) -> Self {
        self.having = self.having.and(condition, params);
        self
    }

    pub fn order_by_asc(mut self, columns: &[&str]) -> Self {
        self.push_order(columns, Direction::Asc);
        self
    }

    pub fn order_by_desc(mut self, columns: &[&str]) -> Self {
        self.push_order(columns, Direction::Desc);
        self
    }

    fn push_order(&mut self, columns: &[&str], direction: Direction) {
        self.order_by.extend(columns.iter().map(|column| Order {
            column: column.to_string(),
            direction,
        }));
    }

    pub fn limit(mut self, rows: u32) -> Self {
        self.limit = Some(rows);
        self
    }

    pub fn offset(mut self, rows: u32) -> Self {
        self.offset = Some(rows);
        self
    }

    pub fn build(&self) -> Result<Query, SqlBuilderError> {
        if self.offset.is_some() && self.limit.is_none() {
            return Err(SqlBuilderError::OffsetWithoutLimit);
        }

        let mut sql = String::from("SELECT ");
        let mut params: Vec<Value> = Vec::new();

        if self.distinct {
            sql.push_str("DISTINCT ");
        }
        sql.push_str(&self.columns.render()?);
        sql.push_str(" FROM ");
        sql.push_str(&ident(&self.table)?);

        for join in &self.joins {
            sql.push_str(&format!(
                " {} {} ON {}",
                join.kind.keyword(),
                ident(&join.table)?,
                join.on
            ));
        }

        if !self.filter.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&self.filter.render()?);
            params.extend(self.filter.params().iter().cloned());
        }

        if !self.group_by.is_empty() {
            sql.push_str(" GROUP BY ");
            sql.push_str(&ident_list(&self.group_by)?);
        }

        if !self.having.is_empty() {
            sql.push_str(" HAVING ");
            sql.push_str(&self.having.render()?);
            params.extend(self.having.params().iter().cloned());
        }

        if !self.order_by.is_empty() {
            let terms: Vec<String> = self
                .order_by
                .iter()
                .map(|order| match order.direction {
                    Direction::Asc => ident(&order.column),
                    Direction::Desc => ident(&order.column).map(|column| format!("{column} DESC")),
                })
                .collect::<Result<_, _>>()?;

            sql.push_str(" ORDER BY ");
            sql.push_str(&terms.join(", "));
        }

        if let Some(rows) = self.limit {
            sql.push_str(" LIMIT ?");
            params.push(Value::Integer(i64::from(rows)));
        }

        if let Some(rows) = self.offset {
            sql.push_str(" OFFSET ?");
            params.push(Value::Integer(i64::from(rows)));
        }

        sql.push(';');
        Ok(Query { sql, params })
    }
}
