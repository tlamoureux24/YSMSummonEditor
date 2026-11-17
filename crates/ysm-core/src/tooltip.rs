//! Parser for the game's tooltip markup.
//!
//! Supported tags: `<B>`, `<I>`, `<U>`, `<BR>`, `<SIZE=n>` … `</SIZE>` and
//! colours written as `<#RRGGBB>` … `</#RRGGBB>`. Unknown tags are dropped
//! and their text content is preserved.

use crate::i18n::trf;
#[derive(Debug, Clone, PartialEq)]
pub struct TooltipSegment {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub font_size: Option<f32>,
    /// Upper-case `#RRGGBB` or `#AARRGGBB`.
    pub color_hex: Option<String>,
}

#[derive(Default)]
struct Style {
    bold: u32,
    italic: u32,
    underline: u32,
    size: Option<f32>,
    color: Option<String>,
    size_stack: Vec<Option<f32>>,
    color_stack: Vec<Option<String>>,
}

impl Style {
    fn segment(&self, text: &str) -> TooltipSegment {
        TooltipSegment {
            text: text.to_owned(),
            bold: self.bold > 0,
            italic: self.italic > 0,
            underline: self.underline > 0,
            font_size: self.size,
            color_hex: self.color.clone(),
        }
    }
}

/// Splits markup into styled text runs. Line breaks are separate `"\n"` runs.
pub fn parse(markup: &str) -> Vec<TooltipSegment> {
    if markup.trim().is_empty() {
        return Vec::new();
    }

    let source = markup.replace('\r', "");
    let mut output = Vec::new();
    let mut style = Style::default();
    let mut text_start = 0;

    for (tag_start, tag_end) in tag_ranges(&source) {
        if tag_start > text_start {
            output.push(style.segment(&source[text_start..tag_start]));
        }
        handle_tag(source[tag_start + 1..tag_end - 1].trim(), &mut style, &mut output);
        text_start = tag_end;
    }

    if text_start < source.len() {
        output.push(style.segment(&source[text_start..]));
    }

    output
}

/// Text content with tags removed and `<BR>` turned into newlines.
pub fn to_plain_text(markup: &str) -> String {
    parse(markup).into_iter().map(|s| s.text).collect()
}

/// Plain tooltip text, or `Skill {id} | {name}` when the skill has none.
pub fn plain_or_fallback(markup: Option<&str>, skill_id: i32, skill_name: &str) -> String {
    let plain = markup.map(to_plain_text).unwrap_or_default();
    let plain = plain.trim();
    if plain.is_empty() { trf("Skill {} | {}", &[&skill_id, &skill_name]) } else { plain.to_owned() }
}

/// Converts `#RRGGBB` / `#AARRGGBB` into `[r, g, b, a]`.
pub fn parse_hex_color(hex: &str) -> Option<[u8; 4]> {
    let raw = hex.strip_prefix('#').unwrap_or(hex);
    if !is_valid_hex_color(raw) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&raw[i..i + 2], 16).ok();
    if raw.len() == 6 {
        Some([byte(0)?, byte(2)?, byte(4)?, 255])
    } else {
        Some([byte(2)?, byte(4)?, byte(6)?, byte(0)?])
    }
}

/// Byte ranges of every `<...>` tag: a `<`, at least one non-`>` character, then `>`.
fn tag_ranges(source: &str) -> Vec<(usize, usize)> {
    let bytes = source.as_bytes();
    let mut ranges = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        match bytes[i + 1..].iter().position(|&b| b == b'>') {
            // "<>" is not a tag; keep scanning after the '<'.
            Some(0) => i += 1,
            Some(offset) => {
                let end = i + 1 + offset + 1;
                ranges.push((i, end));
                i = end;
            }
            None => break,
        }
    }
    ranges
}

fn handle_tag(token: &str, style: &mut Style, output: &mut Vec<TooltipSegment>) {
    if token.is_empty() {
        return;
    }

    let upper = token.to_ascii_uppercase();
    match upper.as_str() {
        "BR" | "BR/" | "/BR" => output.push(style.segment("\n")),
        "B" => style.bold += 1,
        "/B" => style.bold = style.bold.saturating_sub(1),
        "I" => style.italic += 1,
        "/I" => style.italic = style.italic.saturating_sub(1),
        "U" => style.underline += 1,
        "/U" => style.underline = style.underline.saturating_sub(1),
        "/SIZE" => style.size = style.size_stack.pop().flatten(),
        _ if upper.starts_with("SIZE=") => {
            if let Ok(size) = token[5..].trim().parse::<f32>()
                && size > 0.0
                && size.is_finite()
            {
                style.size_stack.push(style.size);
                style.size = Some(size);
            }
        }
        _ if token.starts_with("/#") => style.color = style.color_stack.pop().flatten(),
        _ if token.starts_with('#') => {
            let hex = token[1..].trim();
            if is_valid_hex_color(hex) {
                style.color_stack.push(style.color.take());
                style.color = Some(format!("#{}", hex.to_ascii_uppercase()));
            }
        }
        _ => {}
    }
}

fn is_valid_hex_color(raw: &str) -> bool {
    matches!(raw.len(), 6 | 8) && raw.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_game_tags_and_hex_color() {
        let segments = parse("<B>Bold</B><BR><#FFAA00><SIZE=18>Gold</SIZE></#FFAA00>");

        assert!(segments.len() >= 3);
        assert_eq!(segments[0].text, "Bold");
        assert!(segments[0].bold);
        assert!(segments.iter().any(|s| s.text == "\n"));
        assert!(
            segments.iter().any(|s| {
                s.text == "Gold" && s.font_size == Some(18.0) && s.color_hex.as_deref() == Some("#FFAA00")
            })
        );
    }

    #[test]
    fn ignores_unknown_tags_and_preserves_text() {
        let segments = parse("<UNKNOWN>Hello</UNKNOWN> <I>world</I>");
        let combined: String = segments.iter().map(|s| s.text.as_str()).collect();

        assert_eq!(combined, "Hello world");
        assert!(segments.iter().any(|s| s.text.contains("world") && s.italic));
    }

    #[test]
    fn plain_text_strips_tags_and_keeps_breaks() {
        assert_eq!(to_plain_text("<B>One</B><BR><#00FF00>Two</#00FF00>"), "One\nTwo");
    }

    #[test]
    fn nested_colors_restore_the_outer_color() {
        let segments = parse("<#112233>a<#445566>b</#445566>c</#112233>d");
        let colors: Vec<_> = segments.iter().map(|s| s.color_hex.as_deref()).collect();
        assert_eq!(colors, [Some("#112233"), Some("#445566"), Some("#112233"), None]);
    }

    #[test]
    fn stray_angle_brackets_do_not_break_parsing() {
        assert_eq!(to_plain_text("a <> b"), "a <> b");
        assert_eq!(to_plain_text("5 < 6"), "5 < 6");
        assert_eq!(to_plain_text("żółć<BR>ąę"), "żółć\nąę");
    }

    #[test]
    fn blank_markup_has_no_segments() {
        assert!(parse("  \r\n ").is_empty());
    }

    #[test]
    fn fallback_is_used_when_tooltip_is_empty() {
        assert_eq!(plain_or_fallback(None, 7, "Bash"), "Skill 7 | Bash");
        assert_eq!(plain_or_fallback(Some("<B></B>"), 7, "Bash"), "Skill 7 | Bash");
        assert_eq!(plain_or_fallback(Some(" <B>Hit</B> "), 7, "Bash"), "Hit");
    }

    #[test]
    fn hex_colors_convert_to_rgba() {
        assert_eq!(parse_hex_color("#FFAA00"), Some([255, 170, 0, 255]));
        assert_eq!(parse_hex_color("80FFAA00"), Some([255, 170, 0, 128]));
        assert_eq!(parse_hex_color("#FFF"), None);
        assert_eq!(parse_hex_color("#GGGGGG"), None);
    }
}
