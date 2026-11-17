//! Typed values of `SkillResource` columns.
//!
//! The skill editor works on an arbitrary set of columns, so values are kept
//! in a small dynamic representation that knows how to parse user input and
//! how to render itself as a T-SQL literal.

use std::fmt;
use std::str::FromStr;

use rust_decimal::Decimal;

/// Columns the "Business" tab of the skill editor exposes, in display order.
pub const BUSINESS_FIELDS: [&str; 31] = [
    "delay_cast",
    "delay_cast_per_skl",
    "delay_common",
    "delay_cooltime",
    "delay_cooltime_per_skl",
    "effect_type",
    "state_id",
    "state_level_base",
    "state_level_per_skl",
    "state_second",
    "state_second_per_level",
    "var1",
    "var2",
    "var3",
    "var4",
    "var5",
    "var6",
    "var7",
    "var8",
    "var9",
    "var10",
    "var11",
    "var12",
    "var13",
    "var14",
    "var15",
    "var16",
    "var17",
    "var18",
    "var19",
    "var20",
];

pub fn is_business_field(name: &str) -> bool {
    BUSINESS_FIELDS.iter().any(|f| f.eq_ignore_ascii_case(name))
}

/// SQL storage class of a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldKind {
    Bool,
    U8,
    I16,
    I32,
    I64,
    F32,
    F64,
    Decimal,
    Text,
    /// A column type the editor shows but cannot edit (binary, date, guid…).
    Unsupported,
}

impl FieldKind {
    pub fn is_editable(self) -> bool {
        self != FieldKind::Unsupported
    }

    pub fn label(self) -> &'static str {
        match self {
            FieldKind::Bool => "bit",
            FieldKind::U8 => "tinyint",
            FieldKind::I16 => "smallint",
            FieldKind::I32 => "int",
            FieldKind::I64 => "bigint",
            FieldKind::F32 => "real",
            FieldKind::F64 => "float",
            FieldKind::Decimal => "decimal",
            FieldKind::Text => "text",
            FieldKind::Unsupported => "other",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FieldValue {
    Null,
    Bool(bool),
    U8(u8),
    I16(i16),
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    Decimal(Decimal),
    Text(String),
    Unsupported(String),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValueError {
    #[error("{}", crate::i18n::trf("'{}' is not a valid {} value", &[input, expected]))]
    Invalid { input: String, expected: &'static str },
    #[error("{}", crate::i18n::tr("this column type cannot be edited"))]
    NotEditable,
}

impl FieldValue {
    /// Parses user input for a column of the given kind.
    ///
    /// Blank input becomes the zero value for numeric and bit columns, the
    /// same way the original editor converted it. Text is taken verbatim.
    pub fn parse(kind: FieldKind, input: &str) -> Result<FieldValue, ValueError> {
        if kind == FieldKind::Text {
            return Ok(FieldValue::Text(input.to_owned()));
        }
        if kind == FieldKind::Unsupported {
            return Err(ValueError::NotEditable);
        }

        let text = input.trim();
        let invalid = || ValueError::Invalid { input: input.to_owned(), expected: kind.label() };

        Ok(match kind {
            FieldKind::Bool => match text.to_ascii_lowercase().as_str() {
                "" | "0" | "false" => FieldValue::Bool(false),
                "1" | "true" => FieldValue::Bool(true),
                _ => return Err(invalid()),
            },
            FieldKind::U8 => FieldValue::U8(parse_or_zero(text).ok_or_else(invalid)?),
            FieldKind::I16 => FieldValue::I16(parse_or_zero(text).ok_or_else(invalid)?),
            FieldKind::I32 => FieldValue::I32(parse_or_zero(text).ok_or_else(invalid)?),
            FieldKind::I64 => FieldValue::I64(parse_or_zero(text).ok_or_else(invalid)?),
            FieldKind::F32 => {
                let value: f32 = parse_or_zero(text).ok_or_else(invalid)?;
                if !value.is_finite() {
                    return Err(invalid());
                }
                FieldValue::F32(value)
            }
            FieldKind::F64 => {
                let value: f64 = parse_or_zero(text).ok_or_else(invalid)?;
                if !value.is_finite() {
                    return Err(invalid());
                }
                FieldValue::F64(value)
            }
            FieldKind::Decimal => {
                if text.is_empty() {
                    FieldValue::Decimal(Decimal::ZERO)
                } else {
                    let parsed =
                        Decimal::from_str(text).or_else(|_| Decimal::from_scientific(text)).map_err(|_| invalid())?;
                    FieldValue::Decimal(parsed)
                }
            }
            FieldKind::Text | FieldKind::Unsupported => unreachable!("handled above"),
        })
    }

    pub fn is_null(&self) -> bool {
        matches!(self, FieldValue::Null)
    }

    /// Renders the value as a PostgreSQL literal.
    pub fn pg_literal(&self) -> String {
        match self {
            FieldValue::Null => "NULL".to_owned(),
            FieldValue::Bool(v) => if *v { "true" } else { "false" }.to_owned(),
            FieldValue::Text(v) | FieldValue::Unsupported(v) => format!("'{}'", v.replace('\'', "''")),
            other => other.to_string(),
        }
    }

    /// Renders the value as a T-SQL literal.
    pub fn sql_literal(&self) -> String {
        match self {
            FieldValue::Null => "NULL".to_owned(),
            FieldValue::Bool(v) => if *v { "1" } else { "0" }.to_owned(),
            FieldValue::Text(v) | FieldValue::Unsupported(v) => {
                format!("N'{}'", v.replace('\'', "''"))
            }
            other => other.to_string(),
        }
    }
}

fn parse_or_zero<T: FromStr + Default>(text: &str) -> Option<T> {
    if text.is_empty() { Some(T::default()) } else { text.parse().ok() }
}

impl fmt::Display for FieldValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FieldValue::Null => Ok(()),
            FieldValue::Bool(v) => f.write_str(if *v { "1" } else { "0" }),
            FieldValue::U8(v) => write!(f, "{v}"),
            FieldValue::I16(v) => write!(f, "{v}"),
            FieldValue::I32(v) => write!(f, "{v}"),
            FieldValue::I64(v) => write!(f, "{v}"),
            FieldValue::F32(v) => write!(f, "{v}"),
            FieldValue::F64(v) => write!(f, "{v}"),
            FieldValue::Decimal(v) => write!(f, "{v}"),
            FieldValue::Text(v) | FieldValue::Unsupported(v) => f.write_str(v),
        }
    }
}

/// One column of a loaded `SkillResource` row.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillField {
    pub name: String,
    pub kind: FieldKind,
    pub value: FieldValue,
}

impl SkillField {
    /// The primary key is shown but never editable.
    pub fn is_read_only(&self) -> bool {
        self.name.eq_ignore_ascii_case("id") || !self.kind.is_editable()
    }
}

/// A full `SkillResource` row with column metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillRecord {
    pub skill_id: i32,
    pub fields: Vec<SkillField>,
}

impl SkillRecord {
    pub fn field(&self, name: &str) -> Option<&SkillField> {
        self.fields.iter().find(|f| f.name.eq_ignore_ascii_case(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_decimal_with_invariant_separator() {
        let value = FieldValue::parse(FieldKind::Decimal, "1.25").unwrap();
        assert_eq!(value, FieldValue::Decimal(Decimal::new(125, 2)));
    }

    #[test]
    fn parses_bool_from_numeric_text() {
        assert_eq!(FieldValue::parse(FieldKind::Bool, "1").unwrap(), FieldValue::Bool(true));
        assert_eq!(FieldValue::parse(FieldKind::Bool, "0").unwrap(), FieldValue::Bool(false));
        assert_eq!(FieldValue::parse(FieldKind::Bool, " True ").unwrap(), FieldValue::Bool(true));
    }

    #[test]
    fn blank_input_becomes_zero_for_value_types() {
        assert_eq!(FieldValue::parse(FieldKind::I32, "  ").unwrap(), FieldValue::I32(0));
        assert_eq!(FieldValue::parse(FieldKind::F64, "").unwrap(), FieldValue::F64(0.0));
        assert_eq!(FieldValue::parse(FieldKind::Bool, "").unwrap(), FieldValue::Bool(false));
        assert_eq!(FieldValue::parse(FieldKind::Decimal, "").unwrap(), FieldValue::Decimal(Decimal::ZERO));
    }

    #[test]
    fn text_is_kept_verbatim() {
        assert_eq!(FieldValue::parse(FieldKind::Text, " a b ").unwrap(), FieldValue::Text(" a b ".into()));
        assert_eq!(FieldValue::parse(FieldKind::Text, "").unwrap(), FieldValue::Text(String::new()));
    }

    #[test]
    fn rejects_garbage_and_overflow() {
        assert!(FieldValue::parse(FieldKind::I32, "abc").is_err());
        assert!(FieldValue::parse(FieldKind::U8, "256").is_err());
        assert!(FieldValue::parse(FieldKind::I16, "40000").is_err());
        assert!(FieldValue::parse(FieldKind::F64, "NaN").is_err());
        assert!(FieldValue::parse(FieldKind::Bool, "2").is_err());
        assert_eq!(FieldValue::parse(FieldKind::Unsupported, "x"), Err(ValueError::NotEditable));
    }

    #[test]
    fn sql_literals_are_escaped_and_typed() {
        assert_eq!(FieldValue::Null.sql_literal(), "NULL");
        assert_eq!(FieldValue::Bool(true).sql_literal(), "1");
        assert_eq!(FieldValue::I32(-5).sql_literal(), "-5");
        assert_eq!(FieldValue::F64(0.5).sql_literal(), "0.5");
        assert_eq!(FieldValue::Text("O'Neil".into()).sql_literal(), "N'O''Neil'");
        assert_eq!(FieldValue::Text("O'Neil".into()).pg_literal(), "'O''Neil'");
        assert_eq!(FieldValue::Bool(true).pg_literal(), "true");
        assert_eq!(FieldValue::Null.pg_literal(), "NULL");
    }

    #[test]
    fn business_fields_match_case_insensitively() {
        assert!(is_business_field("VAR12"));
        assert!(is_business_field("delay_cast"));
        assert!(!is_business_field("id"));
    }
}
