//! Interface language: English (the source strings) or French.
//!
//! Every displayed string goes through [`tr`] or, when it carries values,
//! [`trf`]. The English text is the key; [`fr`] maps it to French. A test
//! scans the sources and fails on any wrapped string without a translation.

mod fr;

use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU8, Ordering};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Language {
    #[default]
    English,
    French,
}

impl Language {
    pub const ALL: [Language; 2] = [Language::English, Language::French];

    /// The language's own name, the same in every interface language.
    pub fn native_name(self) -> &'static str {
        match self {
            Language::English => "English",
            Language::French => "Français",
        }
    }

    /// French for a `fr`, `fr-FR` or `fr_CA.UTF-8` locale, English otherwise.
    pub fn from_locale(locale: &str) -> Language {
        if locale.to_ascii_lowercase().starts_with("fr") { Language::French } else { Language::English }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set_language(language: Language) {
    CURRENT.store(language as u8, Ordering::Relaxed);
}

pub fn language() -> Language {
    if CURRENT.load(Ordering::Relaxed) == Language::French as u8 { Language::French } else { Language::English }
}

static FRENCH: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| fr::TABLE.iter().copied().collect());

/// The current-language version of an English interface string.
pub fn tr(english: &'static str) -> &'static str {
    match language() {
        Language::English => english,
        Language::French => FRENCH.get(english).copied().unwrap_or(english),
    }
}

/// [`tr`] of a template, then each `{}` replaced by the next value.
pub fn trf(english: &'static str, values: &[&dyn std::fmt::Display]) -> String {
    let template = tr(english);
    let mut out = String::with_capacity(template.len() + 16);
    let mut values = values.iter();
    let mut rest = template;
    while let Some(at) = rest.find("{}") {
        out.push_str(&rest[..at]);
        if let Some(value) = values.next() {
            out.push_str(&value.to_string());
        }
        rest = &rest[at + 2..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_take_their_values_in_order() {
        assert_eq!(trf("{} of {}", &[&1, &"two"]), "1 of two");
        assert_eq!(trf("no value", &[]), "no value");
    }

    #[test]
    fn locales_pick_the_language() {
        assert_eq!(Language::from_locale("fr-FR"), Language::French);
        assert_eq!(Language::from_locale("fr_CA.UTF-8"), Language::French);
        assert_eq!(Language::from_locale("en-US"), Language::English);
        assert_eq!(Language::from_locale(""), Language::English);
    }

    #[test]
    fn the_french_table_is_well_formed() {
        let mut seen = std::collections::HashSet::new();
        for (english, french) in fr::TABLE {
            assert!(seen.insert(english), "duplicate key {english:?}");
            assert!(!french.trim().is_empty(), "empty translation for {english:?}");
            assert_eq!(english.matches("{}").count(), french.matches("{}").count(), "placeholders of {english:?}");
        }
    }

    /// Every string wrapped in `tr("…")` / `trf("…"` anywhere in the
    /// workspace has a French translation.
    #[test]
    fn every_wrapped_string_is_translated() {
        let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let mut missing = Vec::new();
        let mut stack = vec![crates.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") && !path.ends_with("i18n/mod.rs") {
                    let source = std::fs::read_to_string(&path).unwrap();
                    for key in wrapped_strings(&source) {
                        if !FRENCH.contains_key(key.as_str()) {
                            missing.push(format!("{}: {key:?}", path.display()));
                        }
                    }
                }
            }
        }
        assert!(missing.is_empty(), "{} untranslated:\n{}", missing.len(), missing.join("\n"));
    }

    fn wrapped_strings(source: &str) -> Vec<String> {
        let mut keys = Vec::new();
        for marker in ["tr(\"", "trf(\""] {
            let mut rest = source;
            while let Some(at) = rest.find(marker) {
                let before = rest[..at].chars().last();
                rest = &rest[at + marker.len()..];
                if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    continue;
                }
                let mut key = String::new();
                let mut chars = rest.chars();
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => match chars.next() {
                            Some('n') => key.push('\n'),
                            Some('\n') => {
                                // A line continuation: skip the next line's indentation.
                                let tail: String = chars.clone().collect();
                                let trimmed = tail.trim_start();
                                for _ in 0..(tail.chars().count() - trimmed.chars().count()) {
                                    chars.next();
                                }
                            }
                            Some(other) => key.push(other),
                            None => break,
                        },
                        other => key.push(other),
                    }
                }
                keys.push(key);
            }
        }
        keys
    }
}
