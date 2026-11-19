//! Syntax colors for the statements emitted by the SQL Server and MariaDB writers.

use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{Color32, FontId};

use crate::theme::{self, color};

pub(super) fn layout(sql: &str) -> LayoutJob {
    let font = FontId::monospace(theme::BODY_SIZE);
    let identifier_color = color::TEXT.lerp_to_gamma(color::ACCENT_BRIGHT, 0.35);
    let mut job =
        LayoutJob { text: String::with_capacity(sql.len()), keep_trailing_whitespace: true, ..Default::default() };
    let mut remaining = sql;
    while let Some(first) = remaining.chars().next() {
        let (size, tint) = token(remaining, first, identifier_color);
        let (text, rest) = remaining.split_at(size);
        job.append(text, 0.0, TextFormat::simple(font.clone(), tint));
        remaining = rest;
    }
    job
}

fn token(sql: &str, first: char, identifier_color: Color32) -> (usize, Color32) {
    let rest = &sql[first.len_utf8()..];
    match first {
        '-' if rest.starts_with('-') => (sql.find('\n').unwrap_or(sql.len()), color::TEXT_MUTED),
        '/' if rest.starts_with('*') => (sql[2..].find("*/").map_or(sql.len(), |end| end + 4), color::TEXT_MUTED),
        '\'' => (quoted_len(sql, 1, '\''), color::SUCCESS),
        'N' | 'n' | 'X' | 'x' if rest.starts_with('\'') => (quoted_len(sql, 2, '\''), color::SUCCESS),
        '[' => (quoted_len(sql, 1, ']'), identifier_color),
        '`' => (quoted_len(sql, 1, '`'), identifier_color),
        '@' => (1 + word_len(rest), identifier_color),
        c if c.is_ascii_digit() || (matches!(c, '-' | '.') && rest.starts_with(|c: char| c.is_ascii_digit())) => {
            (number_len(sql), color::WARNING)
        }
        c if c.is_alphanumeric() || c == '_' => {
            let size = word_len(sql);
            let tint = if KEYWORDS.iter().any(|word| word.eq_ignore_ascii_case(&sql[..size])) {
                color::ACCENT_BRIGHT
            } else {
                color::TEXT
            };
            (size, tint)
        }
        _ => (first.len_utf8(), color::TEXT),
    }
}

fn word_len(sql: &str) -> usize {
    sql.chars().take_while(|c| c.is_alphanumeric() || *c == '_').map(char::len_utf8).sum()
}

fn quoted_len(sql: &str, mut offset: usize, closing: char) -> usize {
    while let Some(next) = sql[offset..].find(closing) {
        offset += next + closing.len_utf8();
        if !sql[offset..].starts_with(closing) {
            return offset;
        }
        offset += closing.len_utf8();
    }
    sql.len()
}

fn number_len(sql: &str) -> usize {
    let digits = |text: &str| text.bytes().take_while(u8::is_ascii_digit).count();
    let unsigned = sql.strip_prefix('-').unwrap_or(sql);
    let mut size = sql.len() - unsigned.len() + digits(unsigned);
    if let Some(fraction) = sql[size..].strip_prefix('.') {
        size += 1 + digits(fraction);
    }
    if let Some(exponent) = sql[size..].strip_prefix(['e', 'E']) {
        let unsigned = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
        let count = digits(unsigned);
        if count > 0 {
            size = sql.len() - unsigned.len() + count;
        }
    }
    size
}

const KEYWORDS: &[&str] = &[
    "AND",
    "ATOMIC",
    "BEGIN",
    "COALESCE",
    "COMMIT",
    "CONVERT",
    "DECLARE",
    "DELETE",
    "DELIMITER",
    "DUPLICATE",
    "ELSE",
    "END",
    "EXISTS",
    "EXIT",
    "FOR",
    "FROM",
    "GREATEST",
    "HANDLER",
    "HOLDLOCK",
    "IF",
    "INSERT",
    "INT",
    "INTO",
    "ISNULL",
    "ISOLATION",
    "KEY",
    "LEVEL",
    "MAX",
    "MESSAGE_TEXT",
    "NOT",
    "NULL",
    "ON",
    "RESIGNAL",
    "ROLLBACK",
    "ROW_COUNT",
    "SELECT",
    "SERIALIZABLE",
    "SET",
    "SIGNAL",
    "SQLEXCEPTION",
    "SQLSTATE",
    "START",
    "THEN",
    "TRANSACTION",
    "UPDATE",
    "UPDLOCK",
    "USING",
    "VALUES",
    "WHERE",
    "WITH",
    "XACT_ABORT",
];

#[cfg(test)]
mod tests {
    use ysm_core::commit::{CommitRequest, FieldChange, SkillUpdate, TreeCommit};
    use ysm_core::sql_script::{SqlDialect, render_mariadb_script, render_script};
    use ysm_core::value::{FieldKind, FieldValue};

    use super::*;

    fn sample_changes() -> CommitRequest {
        CommitRequest {
            trees: vec![TreeCommit {
                summon_id: 3,
                slot: 2,
                original_tree_id: 30,
                entries: Vec::new(),
                clone_if_shared: true,
            }],
            skills: vec![SkillUpdate {
                skill_id: 55,
                changes: vec![
                    FieldChange {
                        name: "note".into(),
                        kind: FieldKind::Text,
                        old: FieldValue::Null,
                        new: FieldValue::Text("O'Neil – żółć\\".into()),
                    },
                    FieldChange {
                        name: "var1".into(),
                        kind: FieldKind::F64,
                        old: FieldValue::Null,
                        new: FieldValue::F64(-1.25),
                    },
                ],
            }],
            ..Default::default()
        }
    }

    fn check_layout(sql: &str, expected: &[(&str, Color32)]) {
        let job = layout(sql);
        let sections: Vec<_> = job
            .sections
            .iter()
            .map(|section| {
                let range = &section.byte_range;
                (&job.text[range.start.0..range.end.0], section.format.color)
            })
            .collect();
        assert_eq!(job.text, sql);
        assert_eq!(sections.iter().map(|(text, _)| *text).collect::<String>(), sql);
        for &(text, tint) in expected {
            assert!(sections.contains(&(text, tint)), "missing highlighted SQL token {text:?}");
        }
    }

    #[test]
    fn highlights_tsql_and_preserves_the_script() {
        let sql = render_script(&sample_changes(), "2026-11-18", SqlDialect::TSql);
        check_layout(
            &sql,
            &[
                ("-- Generated: 2026-11-18", color::TEXT_MUTED),
                ("XACT_ABORT", color::ACCENT_BRIGHT),
                ("int", color::ACCENT_BRIGHT),
                ("@NewTreeId1", color::TEXT.lerp_to_gamma(color::ACCENT_BRIGHT, 0.35)),
                ("[note]", color::TEXT.lerp_to_gamma(color::ACCENT_BRIGHT, 0.35)),
                ("N'O''Neil – żółć\\'", color::SUCCESS),
                ("-1.25", color::WARNING),
            ],
        );
    }

    #[test]
    fn highlights_mariadb_and_preserves_the_script() {
        let sql = render_mariadb_script(&sample_changes(), "2026-11-18");
        check_layout(
            &sql,
            &[
                ("DELIMITER", color::ACCENT_BRIGHT),
                ("ATOMIC", color::ACCENT_BRIGHT),
                ("ROW_COUNT", color::ACCENT_BRIGHT),
                ("`note`", color::TEXT.lerp_to_gamma(color::ACCENT_BRIGHT, 0.35)),
                ("X'4f274e65696c20e2809320c5bcc3b3c582c4875c'", color::SUCCESS),
                ("'45000'", color::SUCCESS),
                ("-1.25", color::WARNING),
            ],
        );
    }
}
