//! Editor for the columns of one `SkillResource` row.

use eframe::egui::{self, Align, Layout, RichText, Ui};
use egui_extras::{Column, TableBuilder};
use ysm_core::commit::{FieldChange, SkillUpdate};
use ysm_core::value::{BUSINESS_FIELDS, FieldKind, FieldValue, SkillRecord, is_business_field};

use super::DialogContext;
use crate::action::Action;
use crate::icons;
use crate::theme::{self, color, space};
use crate::widgets::{self, ColumnAlign, RowBand, Segment, header_cell, modal};

const GRID_HEIGHT: f32 = 440.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Business,
    All,
}

struct FieldRow {
    name: String,
    kind: FieldKind,
    db_value: FieldValue,
    /// The database value as text; a row is dirty when `text` differs from it.
    db_text: String,
    text: String,
    read_only: bool,
    business: bool,
    error: Option<String>,
}

impl FieldRow {
    fn is_dirty(&self) -> bool {
        self.text != self.db_text
    }

    fn validate(&mut self) {
        self.error =
            if self.is_dirty() { FieldValue::parse(self.kind, &self.text).err().map(|e| e.to_string()) } else { None };
    }
}

pub struct SkillEditDialog {
    skill_id: i32,
    skill_name: String,
    rows: Vec<FieldRow>,
    tab: Tab,
    filter: String,
    only_changed: bool,
}

impl SkillEditDialog {
    /// `pending` is the update already staged for this skill, if any; its
    /// values are shown as the current ones so reopening never loses edits.
    pub fn new(record: SkillRecord, pending: Option<&SkillUpdate>, skill_name: String) -> Self {
        let mut rows: Vec<FieldRow> = record
            .fields
            .into_iter()
            .map(|field| {
                let db_text = field.value.to_string();
                let staged =
                    pending.and_then(|update| update.changes.iter().find(|c| c.name.eq_ignore_ascii_case(&field.name)));
                FieldRow {
                    read_only: field.is_read_only(),
                    business: is_business_field(&field.name),
                    text: staged.map_or_else(|| db_text.clone(), |change| change.new.to_string()),
                    db_text,
                    name: field.name,
                    kind: field.kind,
                    db_value: field.value,
                    error: None,
                }
            })
            .collect();
        rows.iter_mut().for_each(FieldRow::validate);

        Self {
            skill_id: record.skill_id,
            skill_name,
            rows,
            tab: Tab::Business,
            filter: String::new(),
            only_changed: false,
        }
    }

    fn changed_count(&self) -> usize {
        self.rows.iter().filter(|row| row.is_dirty()).count()
    }

    fn has_errors(&self) -> bool {
        self.rows.iter().any(|row| row.error.is_some())
    }

    fn build_update(&self) -> SkillUpdate {
        let changes = self
            .rows
            .iter()
            .filter(|row| row.is_dirty() && !row.read_only)
            .filter_map(|row| {
                let new = FieldValue::parse(row.kind, &row.text).ok()?;
                Some(FieldChange { name: row.name.clone(), kind: row.kind, old: row.db_value.clone(), new })
            })
            .collect();
        SkillUpdate { skill_id: self.skill_id, changes }
    }

    /// Indices of the rows shown on the current tab, in display order.
    fn visible_rows(&self) -> Vec<usize> {
        let filter = self.filter.trim().to_lowercase();
        let passes = |row: &FieldRow| {
            (filter.is_empty() || row.name.to_lowercase().contains(&filter)) && (!self.only_changed || row.is_dirty())
        };
        match self.tab {
            Tab::All => (0..self.rows.len()).filter(|i| passes(&self.rows[*i])).collect(),
            // Business fields keep their curated order rather than the table's column order.
            Tab::Business => BUSINESS_FIELDS
                .iter()
                .filter_map(|name| self.rows.iter().position(|row| row.name.eq_ignore_ascii_case(name)))
                .filter(|i| passes(&self.rows[*i]))
                .collect(),
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, id: egui::Id, env: &mut DialogContext<'_>) -> bool {
        let title = format!("Skill {} · {}", self.skill_id, self.skill_name);
        let mut apply = false;
        let mut cancelled = false;

        let outcome = modal::show(ctx, id, icons::EDIT, color::accent_bright(), &title, 784.0, |ui| {
            let business = self.rows.iter().filter(|row| row.business).count();
            let business_label = format!("Business fields · {business}");
            let all_label = format!("All columns · {}", self.rows.len());
            let segments = [
                Segment { icon: "", label: &business_label, tooltip: "The columns that are changed most often" },
                Segment { icon: "", label: &all_label, tooltip: "Every column of SkillResource" },
            ];
            let tab = &mut self.tab;
            let filter = &mut self.filter;
            let only_changed = &mut self.only_changed;
            widgets::control_row(
                ui,
                |ui| {
                    if let Some(index) = widgets::segmented(ui, &segments, usize::from(*tab == Tab::All), 168.0) {
                        *tab = if index == 0 { Tab::Business } else { Tab::All };
                    }
                },
                |ui| {
                    let field = widgets::sized_text_field(filter, 200.0);
                    ui.add(field.hint_text(format!("{}  Filter columns", icons::SEARCH)));
                    ui.checkbox(only_changed, "Changed only");
                },
            );

            let visible = self.visible_rows();
            if visible.is_empty() {
                ui.allocate_ui([ui.available_width(), GRID_HEIGHT + theme::HEADER_HEIGHT].into(), |ui| {
                    widgets::empty_state(
                        ui,
                        icons::SEARCH,
                        "No columns match",
                        "Clear the filter to see every column.",
                    );
                });
            } else {
                self.grid(ui, &visible);
            }

            let changed = self.changed_count();
            let has_errors = self.has_errors();
            modal::footer(ui, |ui| {
                if widgets::button(ui, icons::CLOSE, "Cancel").clicked() {
                    cancelled = true;
                }
                if widgets::primary_button_enabled(ui, !has_errors, icons::CHECK, "Apply").clicked() {
                    apply = true;
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    if has_errors {
                        modal::note(ui, icons::WARNING, "Fix the highlighted values before applying.", color::danger());
                    } else if changed == 0 {
                        ui.label(widgets::caption("No changes"));
                    } else {
                        ui.label(widgets::caption(format!("{changed} column(s) changed")));
                    }
                });
            });
        });

        if apply {
            env.actions.push(Action::StageSkillUpdate(self.build_update()));
            return false;
        }
        let escaped = outcome.dismissed && ctx.input(|input| input.key_pressed(egui::Key::Escape));
        !(cancelled || escaped)
    }

    fn grid(&mut self, ui: &mut Ui, visible: &[usize]) {
        let rows = &mut self.rows;
        ui.spacing_mut().item_spacing = egui::Vec2::new(space::MD, space::XS);
        TableBuilder::new(ui)
            .id_salt(("skill-fields", self.tab == Tab::All))
            .striped(true)
            .cell_layout(Layout::left_to_right(Align::Center))
            .auto_shrink(false)
            .min_scrolled_height(GRID_HEIGHT)
            .max_scroll_height(GRID_HEIGHT)
            .column(Column::exact(216.0).clip(true))
            .column(Column::exact(64.0))
            .column(Column::remainder().at_least(160.0))
            .column(Column::exact(144.0).clip(true))
            .column(Column::exact(theme::CONTROL_HEIGHT))
            .header(theme::HEADER_HEIGHT, |mut header| {
                for (index, title) in ["Column", "Type", "Value", "In database", ""].into_iter().enumerate() {
                    header.col(|ui| {
                        if index == 0 {
                            ui.add_space(space::SM);
                        }
                        header_cell(ui, title, "", ColumnAlign::Left);
                    });
                }
            })
            .body(|mut body| {
                let band = RowBand { x: body.max_rect().x_range(), clip: body.ui_mut().clip_rect() };
                body.rows(theme::ROW_HEIGHT, visible.len(), |mut table_row| {
                    let row = &mut rows[visible[table_row.index()]];
                    let dirty = row.is_dirty();

                    table_row.col(|ui| {
                        if dirty {
                            band.paint(ui, color::row_dirty());
                        }
                        ui.add_space(space::SM);
                        let name = RichText::new(&row.name);
                        let name = if row.business { name.font(theme::bold(theme::BODY_SIZE)) } else { name };
                        ui.add(egui::Label::new(name).truncate());
                    });
                    table_row.col(|ui| {
                        ui.label(widgets::caption(row.kind.label()));
                    });
                    table_row.col(|ui| value_editor(ui, row));
                    table_row.col(|ui| {
                        let original = if row.db_value.is_null() { "NULL".to_owned() } else { row.db_text.clone() };
                        ui.add(egui::Label::new(widgets::muted(original)).truncate()).on_hover_text(&row.db_text);
                    });
                    table_row.col(|ui| {
                        if dirty && widgets::icon_button(ui, icons::DISCARD, "Restore the database value").clicked() {
                            row.text = row.db_text.clone();
                            row.error = None;
                        }
                    });
                });
            });
    }
}

fn value_editor(ui: &mut Ui, row: &mut FieldRow) {
    if row.read_only {
        ui.label(widgets::muted(row.text.as_str())).on_hover_text("This column cannot be edited.");
        return;
    }

    if row.kind == FieldKind::Bool {
        let mut on = matches!(row.text.trim().to_ascii_lowercase().as_str(), "1" | "true");
        if widgets::toggle(ui, &mut on, &row.name).changed() {
            row.text = if on { "1" } else { "0" }.to_owned();
            row.validate();
        }
        return;
    }

    let mut field = widgets::text_field(&mut row.text).desired_width(f32::INFINITY);
    if row.error.is_some() {
        field = field.text_color(color::danger());
    }
    let response = ui.add(field);
    if response.changed() {
        row.validate();
    }
    if let Some(error) = &row.error {
        response.on_hover_text(RichText::new(error).color(color::danger()));
    }
}
