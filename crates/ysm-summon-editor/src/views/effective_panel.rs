//! Effective skills card: the skills of the active evolution merged across its trees.

use eframe::egui::{self, Align, Layout, Margin, RichText, Sense, Ui};
use egui_extras::{Column, TableBuilder};
use ysm_core::session::EffectiveRow;

use super::{ViewContext, begin_card, card_divider};
use crate::action::Action;
use crate::icons;
use crate::theme::{self, color, space};
use crate::widgets::int_cell::int_cell;
use crate::widgets::{self, ColumnAlign, RowBand, game_tooltip, header_cell, icon_text, value_color};

pub fn show(ui: &mut Ui, view: &mut ViewContext<'_>) {
    // The left margin is the gap to the skill tree card.
    let frame =
        egui::Frame::new().fill(color::window_bg()).inner_margin(Margin { left: space::SM as i8, ..Margin::ZERO });
    egui::Panel::right("effective-skills")
        .frame(frame)
        .resizable(true)
        .show_separator_line(false)
        .default_size(384.0)
        .min_size(352.0)
        .show(ui, |ui| {
            theme::card().show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                let state = view.state;
                let stage = state.active_stage;
                let loaded = state.session.is_loaded();
                let rows = state.session.effective_rows(stage);

                begin_card(ui);
                widgets::control_row(
                    ui,
                    |ui| {
                        ui.label(RichText::new(icons::EFFECTIVE).size(theme::GLYPH_SIZE).color(color::text_muted()));
                        widgets::section_title(ui, "Effective Skills");
                    },
                    |ui| {
                        if loaded {
                            ui.label(widgets::caption(format!("Evolution {stage}")));
                        }
                    },
                );
                card_divider(ui);
                widgets::control_row(
                    ui,
                    |ui| {
                        ui.label(widgets::caption("All five trees, merged by skill and level"))
                            .on_hover_text("Editing Min or Max here applies to every merged tree row.");
                    },
                    |ui| {
                        if loaded {
                            ui.label(widgets::caption(format!("{} skills", rows.len())));
                        }
                    },
                );

                if !loaded {
                    widgets::empty_state(ui, icons::EFFECTIVE, "No skills loaded", "Select a summon first.");
                } else if rows.is_empty() {
                    widgets::empty_state(
                        ui,
                        icons::EFFECTIVE,
                        "No skills",
                        "This evolution has no skills in its trees.",
                    );
                } else {
                    grid(ui, view, stage, &rows);
                }
            });
        });
}

fn grid(ui: &mut Ui, view: &mut ViewContext<'_>, stage: u8, rows: &[EffectiveRow]) {
    ui.spacing_mut().item_spacing = egui::Vec2::splat(space::XS);
    ui.visuals_mut().widgets.hovered.bg_fill = color::hover_tint();

    TableBuilder::new(ui)
        .id_salt(("effective-grid", stage))
        .sense(Sense::click())
        .striped(true)
        .cell_layout(Layout::left_to_right(Align::Center))
        .auto_shrink(false)
        .min_scrolled_height(0.0)
        .column(Column::exact(theme::ICON_SIZE + space::SM))
        .column(Column::exact(56.0))
        .column(Column::remainder().at_least(120.0).clip(true))
        .column(Column::exact(36.0))
        .column(Column::exact(36.0))
        .column(Column::exact(36.0))
        .header(theme::HEADER_HEIGHT, |mut header| {
            header.col(|_| {});
            header.col(|ui| header_cell(ui, "ID", "Skill ID", ColumnAlign::Center));
            header.col(|ui| header_cell(ui, "Skill", "Skill name; double-click to edit the skill", ColumnAlign::Left));
            header
                .col(|ui| header_cell(ui, "Min", "Lowest minimum skill level of the merged rows", ColumnAlign::Center));
            header.col(|ui| {
                header_cell(ui, "Max", "Highest maximum skill level of the merged rows", ColumnAlign::Center)
            });
            header.col(|ui| header_cell(ui, "Lv", "Required summon level", ColumnAlign::Center));
        })
        .body(|mut body| {
            let band = RowBand { x: body.max_rect().x_range(), clip: body.ui_mut().clip_rect() };
            body.rows(theme::ROW_HEIGHT, rows.len(), |mut table_row| {
                let row = &rows[table_row.index()];
                grid_row(&mut table_row, view, stage, row, band);
            });
        });
}

fn grid_row(
    table_row: &mut egui_extras::TableRow<'_, '_>,
    view: &mut ViewContext<'_>,
    stage: u8,
    row: &EffectiveRow,
    band: RowBand,
) {
    let session = &view.state.session;
    let key = (row.skill_id, row.lv);
    let selected = view.state.selected_effective == Some(key);
    let dirty_fill = (row.is_dirty && !selected).then_some(color::row_dirty());
    let display = session.skill_display(row.skill_id);
    table_row.set_selected(selected);

    let menu = |ui: &mut Ui, actions: &mut Vec<Action>| {
        ui.set_min_width(192.0);
        if ui.button(icon_text(icons::EDIT, "Edit Skill")).clicked() {
            actions.push(Action::EditSkill(row.skill_id));
            ui.close();
        }
        if ui.button(icon_text(icons::DELETE, "Delete From Tree")).clicked() {
            actions.push(Action::AskDeleteEffective { stage, skill_id: row.skill_id, lv: row.lv });
            ui.close();
        }
    };
    let select = |response: &egui::Response, actions: &mut Vec<Action>| {
        if (response.clicked() || response.secondary_clicked() || response.gained_focus()) && !selected {
            actions.push(Action::SelectEffective(Some(key)));
        }
    };
    let number = |ui: &mut Ui, value: i32| {
        // Dimmed zeros would vanish on the selection colour.
        let tint = if selected && value == 0 { color::text_muted() } else { value_color(value, false) };
        ui.centered_and_justified(|ui| {
            ui.add(egui::Label::new(RichText::new(value.to_string()).color(tint)).selectable(false));
        });
    };

    // Icon. The first cell also paints the background of the whole row.
    table_row.col(|ui| {
        if let Some(fill) = dirty_fill {
            band.paint(ui, fill);
        }
        // Centres the icon in its column.
        ui.add_space(space::XS);
        let icon = view.textures.icon(display.and_then(|d| d.icon_file_name.as_deref()));
        widgets::game_icon(ui, icon, theme::ICON_SIZE).on_hover_ui(|ui| game_tooltip::skill(ui, row.skill_id, display));
    });

    table_row.col(|ui| number(ui, row.skill_id));

    let (_, name_response) = table_row.col(|ui| {
        let tint = if display.is_some() { color::text() } else { color::warning() };
        // Badges and the merge count sit at the right edge; the name truncates in what is left.
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            for group_id in row.group_ids.iter().rev() {
                widgets::badge(ui, &format!("RND[{group_id}]"), color::accent_bright())
                    .on_hover_ui(|ui| game_tooltip::random_group(ui, session, *group_id));
            }
            if row.linked > 1 {
                ui.label(widgets::caption(format!("×{}", row.linked)))
                    .on_hover_text(format!("Merged from {} tree rows", row.linked));
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let name = RichText::new(session.skill_name(row.skill_id)).color(tint);
                ui.add(egui::Label::new(name).truncate().selectable(false))
                    .on_hover_ui(|ui| game_tooltip::skill(ui, row.skill_id, display));
            });
        });
    });
    if name_response.double_clicked() {
        view.actions.push(Action::EditSkill(row.skill_id));
    }

    for (is_min, value) in [(true, row.min_skill_lv), (false, row.max_skill_lv)] {
        table_row.col(|ui| {
            let id = egui::Id::new(("effective-cell", stage, key, is_min));
            let tint = if selected && value == 0 { color::text_muted() } else { value_color(value, false) };
            let output = int_cell(ui, id, value, tint);
            select(&output.response, view.actions);
            if let Some(new_value) = output.committed {
                let (min, max) = if is_min { (new_value, row.max_skill_lv) } else { (row.min_skill_lv, new_value) };
                view.actions.push(Action::SetEffectiveLevels { stage, skill_id: row.skill_id, lv: row.lv, min, max });
            }
            output.response.context_menu(|ui| menu(ui, view.actions));
        });
    }

    table_row.col(|ui| number(ui, row.lv));

    let response = table_row.response();
    select(&response, view.actions);
    response.context_menu(|ui| menu(ui, view.actions));
}
