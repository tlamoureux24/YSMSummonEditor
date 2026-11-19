//! Review of everything a save will change, with the equivalent SQL script.

mod sql;

use eframe::egui::text::LayoutJob;
use eframe::egui::{self, Align, Color32, Layout, RichText, Ui};
use egui_extras::{Column, TableBuilder};
use ysm_core::review::{ReviewKind, ReviewRow};

use super::DialogContext;
use crate::action::Action;
use crate::icons;
use crate::theme::{self, color, space};
use crate::widgets::toast::ToastKind;
use crate::widgets::{self, ColumnAlign, Segment, header_cell, modal};

const CONTENT_HEIGHT: f32 = 440.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Changes,
    Sql,
}

pub struct SaveReviewDialog {
    rows: Vec<ReviewRow>,
    sql: LayoutJob,
    /// Whether confirming exports a file instead of writing to the database.
    to_sql_file: bool,
    /// Where a database save would go, for the footer.
    target: String,
    tab: Tab,
}

impl SaveReviewDialog {
    pub fn new(rows: Vec<ReviewRow>, sql: String, to_sql_file: bool, target: String) -> Self {
        Self { rows, sql: sql::layout(&sql), to_sql_file, target, tab: Tab::Changes }
    }

    pub fn show(&mut self, ctx: &egui::Context, id: egui::Id, env: &mut DialogContext<'_>) -> bool {
        let mut confirmed = false;
        let mut cancelled = false;

        let outcome = modal::show(ctx, id, icons::CHANGES, color::ACCENT_BRIGHT, "Review Changes", 980.0, |ui| {
            let changes_label = format!("Changes · {}", self.rows.len());
            let segments = [
                Segment { icon: icons::CHANGES, label: &changes_label, tooltip: "Every change, with before and after" },
                Segment { icon: icons::CODE, label: "SQL script", tooltip: "The same changes as an executable script" },
            ];
            let tab = &mut self.tab;
            let sql = &self.sql;
            let actions = &mut *env.actions;
            widgets::control_row(
                ui,
                |ui| {
                    if let Some(index) = widgets::segmented(ui, &segments, usize::from(*tab == Tab::Sql), 152.0) {
                        *tab = if index == 0 { Tab::Changes } else { Tab::Sql };
                    }
                },
                |ui| {
                    // Always present, so switching tabs does not shift the row.
                    if widgets::button(ui, icons::CLIPBOARD, "Copy SQL").clicked() {
                        actions.push(Action::CopyText(sql.text.clone()));
                        actions.push(Action::Toast(ToastKind::Info, "SQL script copied to the clipboard.".into()));
                    }
                },
            );

            match self.tab {
                Tab::Changes => self.changes(ui),
                Tab::Sql => self.script(ui),
            }

            modal::footer(ui, |ui| {
                if widgets::button(ui, icons::CLOSE, "Cancel").clicked() {
                    cancelled = true;
                }
                let (icon, label) = if self.to_sql_file {
                    (icons::SQL_FILE, "Export .sql…")
                } else {
                    (icons::DATABASE, "Save to database")
                };
                if widgets::primary_button(ui, icon, label).clicked() {
                    confirmed = true;
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    if self.to_sql_file {
                        ui.label(widgets::caption("Save to .sql: a script is written; the database is not touched."));
                    } else {
                        ui.label(widgets::caption(format!("Target: {}", self.target)));
                    }
                });
            });
        });

        if confirmed {
            env.actions.push(Action::ReviewConfirmed);
            return false;
        }
        if cancelled || outcome.dismissed {
            env.actions.push(Action::SaveCancelled);
            return false;
        }
        true
    }

    fn changes(&self, ui: &mut Ui) {
        TableBuilder::new(ui)
            .id_salt("review-changes")
            .striped(true)
            .cell_layout(Layout::left_to_right(Align::Center))
            .auto_shrink(false)
            .min_scrolled_height(CONTENT_HEIGHT)
            .max_scroll_height(CONTENT_HEIGHT)
            .column(Column::exact(32.0))
            .column(Column::exact(112.0))
            .column(Column::exact(288.0).clip(true))
            .column(Column::exact(128.0).clip(true))
            .column(Column::exact(160.0).clip(true))
            .column(Column::remainder().clip(true))
            .header(theme::HEADER_HEIGHT, |mut header| {
                for title in ["", "Type", "Target", "Field", "Before", "After"] {
                    header.col(|ui| header_cell(ui, title, "", ColumnAlign::Left));
                }
            })
            .body(|body| {
                body.rows(theme::CONTROL_HEIGHT, self.rows.len(), |mut table_row| {
                    let row = &self.rows[table_row.index()];
                    let (icon, tint, description) = kind_style(row.kind);
                    table_row.col(|ui| {
                        ui.centered_and_justified(|ui| {
                            ui.label(RichText::new(icon).size(theme::GLYPH_SIZE).color(tint))
                                .on_hover_text(description);
                        });
                    });
                    table_row.col(|ui| {
                        ui.label(row.category);
                    });
                    table_row.col(|ui| {
                        ui.add(egui::Label::new(row.target.as_str()).truncate()).on_hover_text(&row.target);
                    });
                    table_row.col(|ui| {
                        ui.label(widgets::muted(row.field.as_str()));
                    });
                    table_row.col(|ui| {
                        let text = RichText::new(&row.before).color(color::TEXT_MUTED);
                        let text = if row.kind == ReviewKind::Delete { text.strikethrough() } else { text };
                        ui.add(egui::Label::new(text).truncate()).on_hover_text(&row.before);
                    });
                    table_row.col(|ui| {
                        ui.add(egui::Label::new(RichText::new(&row.after).color(tint)).truncate())
                            .on_hover_text(&row.after);
                    });
                });
            });
    }

    fn script(&self, ui: &mut Ui) {
        // Shorter by its own padding and border, so both tabs are exactly the same height.
        let height = CONTENT_HEIGHT + theme::HEADER_HEIGHT - 2.0 * space::SM - 2.0;
        theme::well().inner_margin(space::SM as i8).show(ui, |ui| {
            egui::ScrollArea::both()
                .id_salt("review-sql")
                .auto_shrink(false)
                .max_height(height)
                .min_scrolled_height(height)
                .show(ui, |ui| {
                    ui.add(egui::Label::new(self.sql.clone()).extend().selectable(true));
                });
        });
    }
}

fn kind_style(kind: ReviewKind) -> (&'static str, Color32, &'static str) {
    match kind {
        ReviewKind::Insert => (icons::INSERTED, color::SUCCESS, "Inserted"),
        ReviewKind::Update => (icons::EDIT, color::WARNING, "Updated"),
        ReviewKind::Delete => (icons::DELETED, color::DANGER, "Deleted"),
    }
}
