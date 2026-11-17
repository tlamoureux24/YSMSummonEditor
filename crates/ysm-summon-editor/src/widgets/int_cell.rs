//! An integer table cell that is edited in place.
//!
//! The cell looks like plain centred text until it is focused. Typing edits
//! a private buffer; the value is committed when focus leaves the cell
//! (Tab, Enter or a click elsewhere) and discarded on Escape.

use eframe::egui::{self, Align, Color32, Key, Response, Stroke, StrokeKind, TextEdit, Ui};

use crate::theme::color;

pub struct IntCellOutput {
    pub response: Response,
    /// The new value, when the user committed a different valid number.
    pub committed: Option<i32>,
}

pub fn int_cell(ui: &mut Ui, id: egui::Id, value: i32, text_color: Color32) -> IntCellOutput {
    let stored: Option<String> = ui.data(|data| data.get_temp(id));
    let mut buffer = stored.unwrap_or_else(|| value.to_string());

    let response = ui.add(
        TextEdit::singleline(&mut buffer)
            .id(id)
            .frame(egui::Frame::NONE)
            .horizontal_align(Align::Center)
            .vertical_align(Align::Center)
            .text_color(text_color)
            .desired_width(f32::INFINITY),
    );

    let mut committed = None;
    if response.has_focus() {
        ui.data_mut(|data| data.insert_temp(id, buffer));
        ui.painter().rect_stroke(
            response.rect.expand(1.0),
            3.0,
            Stroke::new(1.0, color::accent_bright()),
            StrokeKind::Outside,
        );
    } else {
        if response.lost_focus() && !ui.input(|input| input.key_pressed(Key::Escape)) {
            committed = parse(&buffer).filter(|parsed| *parsed != value);
        }
        ui.data_mut(|data| data.remove_temp::<String>(id));
    }

    IntCellOutput { response, committed }
}

/// Blank input means zero, like an empty cell in the original grid.
fn parse(text: &str) -> Option<i32> {
    let text = text.trim();
    if text.is_empty() { Some(0) } else { text.parse().ok() }
}

#[cfg(test)]
mod tests {
    use eframe::egui::accesskit::Role;
    use eframe::egui::{self, Color32, Key, Modifiers};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::{int_cell, parse};

    #[test]
    fn parses_integers_and_treats_blank_as_zero() {
        assert_eq!(parse(" 42 "), Some(42));
        assert_eq!(parse("-7"), Some(-7));
        assert_eq!(parse(""), Some(0));
        assert_eq!(parse("4x"), None);
        assert_eq!(parse("99999999999"), None);
    }

    struct Cell {
        value: i32,
        commits: Vec<i32>,
    }

    /// A cell showing `5`, followed by a button that can take the focus.
    fn harness<'a>() -> Harness<'a, Cell> {
        Harness::new_ui_state(
            |ui, cell: &mut Cell| {
                let output = int_cell(ui, egui::Id::new("cell"), cell.value, Color32::WHITE);
                if let Some(value) = output.committed {
                    cell.value = value;
                    cell.commits.push(value);
                }
                let _ = ui.button("elsewhere");
            },
            Cell { value: 5, commits: Vec::new() },
        )
    }

    /// Focuses the cell, replaces its text with `text` and leaves the cell with `leave`.
    fn edit(harness: &mut Harness<'_, Cell>, text: &str, leave: Key) {
        harness.get_by_role(Role::TextInput).focus();
        harness.run();
        harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
        harness.run();
        harness.get_by_role(Role::TextInput).type_text(text);
        harness.run();
        assert!(harness.state().commits.is_empty(), "nothing is committed while typing");
        harness.key_press(leave);
        harness.run();
    }

    #[test]
    fn leaving_the_cell_commits_the_typed_value() {
        let mut harness = harness();
        edit(&mut harness, "42", Key::Tab);
        assert_eq!(harness.state().commits, [42]);
        assert_eq!(harness.state().value, 42);
    }

    #[test]
    fn enter_commits_too() {
        let mut harness = harness();
        edit(&mut harness, "7", Key::Enter);
        assert_eq!(harness.state().commits, [7]);
    }

    #[test]
    fn escape_discards_the_edit() {
        let mut harness = harness();
        edit(&mut harness, "99", Key::Escape);
        assert!(harness.state().commits.is_empty());
        assert_eq!(harness.state().value, 5);
    }

    #[test]
    fn invalid_or_unchanged_input_commits_nothing() {
        let mut harness = harness();
        edit(&mut harness, "4x", Key::Tab);
        assert!(harness.state().commits.is_empty());

        let mut harness = self::harness();
        edit(&mut harness, "5", Key::Tab);
        assert!(harness.state().commits.is_empty());
    }
}
