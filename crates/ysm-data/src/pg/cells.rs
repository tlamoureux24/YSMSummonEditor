//! Lenient decoding of PostgreSQL result cells.
//!
//! PostgreSQL types are strict (`smallint` is not `integer`), so every cell
//! is first decoded by its column type, then converted the way the MSSQL
//! readers convert theirs.

use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
use tokio_postgres::Row;
use tokio_postgres::types::Type;
use ysm_core::value::{FieldKind, FieldValue};

/// One decoded cell.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Cell {
    Null,
    Bool(bool),
    I16(i16),
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    Decimal(Decimal),
    Text(String),
    /// A type the editor does not decode, by name.
    Other(String),
}

/// Storage class of a column type.
pub(super) fn kind_of(ty: &Type) -> FieldKind {
    match *ty {
        Type::BOOL => FieldKind::Bool,
        Type::INT2 => FieldKind::I16,
        Type::INT4 => FieldKind::I32,
        Type::INT8 => FieldKind::I64,
        Type::FLOAT4 => FieldKind::F32,
        Type::FLOAT8 => FieldKind::F64,
        Type::NUMERIC => FieldKind::Decimal,
        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME => FieldKind::Text,
        _ => FieldKind::Unsupported,
    }
}

fn get<'a, T: tokio_postgres::types::FromSql<'a>>(row: &'a Row, index: usize, wrap: impl FnOnce(T) -> Cell) -> Cell {
    match row.try_get::<_, Option<T>>(index) {
        Ok(Some(value)) => wrap(value),
        Ok(None) => Cell::Null,
        Err(error) => Cell::Other(error.to_string()),
    }
}

pub(super) fn cell(row: &Row, index: usize) -> Cell {
    let ty = row.columns()[index].type_();
    match kind_of(ty) {
        FieldKind::Bool => get(row, index, Cell::Bool),
        FieldKind::I16 => get(row, index, Cell::I16),
        FieldKind::I32 => get(row, index, Cell::I32),
        FieldKind::I64 => get(row, index, Cell::I64),
        FieldKind::F32 => get(row, index, Cell::F32),
        FieldKind::F64 => get(row, index, Cell::F64),
        FieldKind::Decimal => get(row, index, Cell::Decimal),
        FieldKind::Text => get(row, index, Cell::Text),
        FieldKind::U8 | FieldKind::Unsupported => Cell::Other(ty.name().to_owned()),
    }
}

impl Cell {
    pub(super) fn decimal(&self) -> Option<Decimal> {
        match self {
            Cell::Bool(v) => Some(if *v { Decimal::ONE } else { Decimal::ZERO }),
            Cell::I16(v) => Some(Decimal::from(*v)),
            Cell::I32(v) => Some(Decimal::from(*v)),
            Cell::I64(v) => Some(Decimal::from(*v)),
            Cell::F32(v) => Decimal::from_f32(*v),
            Cell::F64(v) => Decimal::from_f64(*v),
            Cell::Decimal(v) => Some(*v),
            Cell::Text(v) => v.trim().parse().ok(),
            Cell::Null | Cell::Other(_) => None,
        }
    }

    pub(super) fn i64(&self) -> Option<i64> {
        match self {
            Cell::I16(v) => Some(i64::from(*v)),
            Cell::I32(v) => Some(i64::from(*v)),
            Cell::I64(v) => Some(*v),
            other => other.decimal().and_then(|d| d.trunc().to_i64()),
        }
    }

    pub(super) fn i32(&self) -> Option<i32> {
        self.i64().map(|v| v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32)
    }

    pub(super) fn bool(&self) -> Option<bool> {
        match self {
            Cell::Bool(v) => Some(*v),
            other => other.i64().map(|v| v != 0),
        }
    }

    pub(super) fn string(&self) -> Option<String> {
        match self {
            Cell::Text(v) => Some(v.clone()),
            other => other.decimal().map(|d| d.to_string()),
        }
    }

    /// The editor's dynamic value, for the skill editor.
    pub(super) fn field(&self, kind: FieldKind) -> FieldValue {
        match self {
            Cell::Null => FieldValue::Null,
            Cell::Bool(v) => FieldValue::Bool(*v),
            Cell::I16(v) => FieldValue::I16(*v),
            Cell::I32(v) => FieldValue::I32(*v),
            Cell::I64(v) => FieldValue::I64(*v),
            Cell::F32(v) => FieldValue::F32(*v),
            Cell::F64(v) => FieldValue::F64(*v),
            Cell::Decimal(v) => FieldValue::Decimal(*v),
            Cell::Text(v) if kind == FieldKind::Text => FieldValue::Text(v.clone()),
            Cell::Text(v) | Cell::Other(v) => FieldValue::Unsupported(v.clone()),
        }
    }
}

/// Reads the cells of a row in column order.
pub(super) struct Cells<'a> {
    row: &'a Row,
    next: usize,
}

impl<'a> Cells<'a> {
    pub(super) fn new(row: &'a Row) -> Self {
        Self { row, next: 0 }
    }

    fn take(&mut self) -> Cell {
        let index = self.next;
        self.next += 1;
        if index < self.row.len() { cell(self.row, index) } else { Cell::Null }
    }

    /// Integer value; `NULL` reads as `0`.
    pub(super) fn i32(&mut self) -> i32 {
        self.take().i32().unwrap_or(0)
    }

    pub(super) fn bool(&mut self) -> bool {
        self.take().bool().unwrap_or(false)
    }

    pub(super) fn decimal(&mut self) -> Decimal {
        self.take().decimal().unwrap_or(Decimal::ZERO)
    }

    pub(super) fn string(&mut self) -> String {
        self.opt_string().unwrap_or_default()
    }

    pub(super) fn opt_string(&mut self) -> Option<String> {
        self.take().string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_cells_convert_across_storage_types() {
        assert_eq!(Cell::I16(7).i32(), Some(7));
        assert_eq!(Cell::I64(i64::MAX).i32(), Some(i32::MAX));
        assert_eq!(Cell::Decimal(Decimal::new(125, 2)).i32(), Some(1));
        assert_eq!(Cell::Text(" 42 ".into()).i32(), Some(42));
        assert_eq!(Cell::I16(1).bool(), Some(true));
        assert_eq!(Cell::I32(0).bool(), Some(false));
        assert_eq!(Cell::Null.i32(), None);
        assert_eq!(Cell::I32(5).string().as_deref(), Some("5"));
    }

    #[test]
    fn column_types_map_to_editable_kinds() {
        assert_eq!(kind_of(&Type::INT2), FieldKind::I16);
        assert_eq!(kind_of(&Type::NUMERIC), FieldKind::Decimal);
        assert_eq!(kind_of(&Type::BPCHAR), FieldKind::Text);
        assert_eq!(kind_of(&Type::BYTEA), FieldKind::Unsupported);
        assert_eq!(Cell::Text("x ".into()).field(FieldKind::Text), FieldValue::Text("x ".into()));
        assert_eq!(Cell::Null.field(FieldKind::I32), FieldValue::Null);
    }
}
