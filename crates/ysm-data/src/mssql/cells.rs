//! Lenient decoding of result cells.
//!
//! The schema's integer columns vary between `tinyint`, `smallint`, `int`
//! and even `decimal` across server versions. The original editor relied on
//! implicit conversion, so these readers accept any numeric storage type.

use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
use tiberius::ColumnData;
use ysm_core::value::{FieldKind, FieldValue};

pub(super) fn cell_decimal(cell: &ColumnData<'_>) -> Option<Decimal> {
    match cell {
        ColumnData::U8(v) => v.map(Decimal::from),
        ColumnData::I16(v) => v.map(Decimal::from),
        ColumnData::I32(v) => v.map(Decimal::from),
        ColumnData::I64(v) => v.map(Decimal::from),
        ColumnData::F32(v) => v.and_then(Decimal::from_f32),
        ColumnData::F64(v) => v.and_then(Decimal::from_f64),
        ColumnData::Bit(v) => v.map(|bit| if bit { Decimal::ONE } else { Decimal::ZERO }),
        ColumnData::Numeric(v) => {
            v.and_then(|n| Decimal::try_from_i128_with_scale(n.value(), u32::from(n.scale())).ok())
        }
        ColumnData::String(v) => v.as_deref().and_then(|text| text.trim().parse().ok()),
        _ => None,
    }
}

pub(super) fn cell_i64(cell: &ColumnData<'_>) -> Option<i64> {
    match cell {
        ColumnData::U8(v) => v.map(i64::from),
        ColumnData::I16(v) => v.map(i64::from),
        ColumnData::I32(v) => v.map(i64::from),
        ColumnData::I64(v) => *v,
        other => cell_decimal(other).and_then(|d| d.trunc().to_i64()),
    }
}

pub(super) fn cell_i32(cell: &ColumnData<'_>) -> Option<i32> {
    cell_i64(cell).map(|v| v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32)
}

pub(super) fn cell_bool(cell: &ColumnData<'_>) -> Option<bool> {
    match cell {
        ColumnData::Bit(v) => *v,
        other => cell_i64(other).map(|v| v != 0),
    }
}

pub(super) fn cell_string(cell: &ColumnData<'_>) -> Option<String> {
    match cell {
        ColumnData::String(v) => v.as_deref().map(str::to_owned),
        other => cell_decimal(other).map(|d| d.to_string()),
    }
}

/// Decodes a cell of an arbitrary column into the editor's dynamic value.
pub(super) fn cell_field(cell: &ColumnData<'_>) -> (FieldKind, FieldValue) {
    fn value<T>(v: &Option<T>, wrap: impl FnOnce(T) -> FieldValue) -> FieldValue
    where
        T: Clone,
    {
        v.clone().map_or(FieldValue::Null, wrap)
    }

    match cell {
        ColumnData::Bit(v) => (FieldKind::Bool, value(v, FieldValue::Bool)),
        ColumnData::U8(v) => (FieldKind::U8, value(v, FieldValue::U8)),
        ColumnData::I16(v) => (FieldKind::I16, value(v, FieldValue::I16)),
        ColumnData::I32(v) => (FieldKind::I32, value(v, FieldValue::I32)),
        ColumnData::I64(v) => (FieldKind::I64, value(v, FieldValue::I64)),
        ColumnData::F32(v) => (FieldKind::F32, value(v, FieldValue::F32)),
        ColumnData::F64(v) => (FieldKind::F64, value(v, FieldValue::F64)),
        ColumnData::Numeric(v) => {
            let decimal = v.and_then(|n| Decimal::try_from_i128_with_scale(n.value(), u32::from(n.scale())).ok());
            (FieldKind::Decimal, decimal.map_or(FieldValue::Null, FieldValue::Decimal))
        }
        ColumnData::String(v) => {
            (FieldKind::Text, v.as_deref().map_or(FieldValue::Null, |text| FieldValue::Text(text.to_owned())))
        }
        other => (FieldKind::Unsupported, FieldValue::Unsupported(format!("{other:?}"))),
    }
}

/// Reads the cells of a row in column order.
pub(super) struct Cells(std::vec::IntoIter<ColumnData<'static>>);

impl Cells {
    pub(super) fn new(values: std::vec::IntoIter<ColumnData<'static>>) -> Self {
        Self(values)
    }

    fn next(&mut self) -> ColumnData<'static> {
        self.0.next().unwrap_or(ColumnData::I32(None))
    }

    /// Integer value; `NULL` reads as `0`.
    pub(super) fn i32(&mut self) -> i32 {
        cell_i32(&self.next()).unwrap_or(0)
    }

    pub(super) fn bool(&mut self) -> bool {
        cell_bool(&self.next()).unwrap_or(false)
    }

    pub(super) fn decimal(&mut self) -> Decimal {
        cell_decimal(&self.next()).unwrap_or(Decimal::ZERO)
    }

    pub(super) fn string(&mut self) -> String {
        self.opt_string().unwrap_or_default()
    }

    pub(super) fn opt_string(&mut self) -> Option<String> {
        cell_string(&self.next())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;
    use tiberius::numeric::Numeric;

    #[test]
    fn integers_are_read_from_any_numeric_storage() {
        assert_eq!(cell_i32(&ColumnData::U8(Some(7))), Some(7));
        assert_eq!(cell_i32(&ColumnData::I16(Some(-3))), Some(-3));
        assert_eq!(cell_i32(&ColumnData::I64(Some(i64::MAX))), Some(i32::MAX));
        assert_eq!(cell_i32(&ColumnData::F64(Some(12.9))), Some(12));
        assert_eq!(cell_i32(&ColumnData::Numeric(Some(Numeric::new_with_scale(1250, 2)))), Some(12));
        assert_eq!(cell_i32(&ColumnData::Bit(Some(true))), Some(1));
        assert_eq!(cell_i32(&ColumnData::I32(None)), None);
        assert_eq!(cell_i32(&ColumnData::String(Some(Cow::Borrowed(" 42 ")))), Some(42));
    }

    #[test]
    fn decimals_keep_their_scale() {
        let cell = ColumnData::Numeric(Some(Numeric::new_with_scale(300_000, 6)));
        assert_eq!(cell_decimal(&cell), Some(Decimal::new(300_000, 6)));
        assert_eq!(cell_decimal(&ColumnData::I32(Some(4))), Some(Decimal::from(4)));
    }

    #[test]
    fn booleans_accept_bits_and_integers() {
        assert_eq!(cell_bool(&ColumnData::Bit(Some(false))), Some(false));
        assert_eq!(cell_bool(&ColumnData::U8(Some(1))), Some(true));
        assert_eq!(cell_bool(&ColumnData::I32(Some(0))), Some(false));
    }

    #[test]
    fn dynamic_fields_carry_kind_even_when_null() {
        assert_eq!(cell_field(&ColumnData::I16(None)), (FieldKind::I16, FieldValue::Null));
        assert_eq!(cell_field(&ColumnData::F32(Some(1.5))), (FieldKind::F32, FieldValue::F32(1.5)));
        assert_eq!(
            cell_field(&ColumnData::String(Some(Cow::Borrowed("x")))),
            (FieldKind::Text, FieldValue::Text("x".into()))
        );
        assert_eq!(cell_field(&ColumnData::Binary(None)).0, FieldKind::Unsupported);
    }
}
