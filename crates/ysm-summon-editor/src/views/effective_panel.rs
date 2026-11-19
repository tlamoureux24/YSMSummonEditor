//! Effective skills card: the skills of the active evolution merged across its trees.

use std::collections::HashSet;

use eframe::egui::{self, Align, Color32, Layout, Margin, RichText, Sense, Ui};
use egui_extras::{Column, TableBuilder, TableRow};
use egui_phosphor::regular::{CARET_DOWN, CARET_RIGHT};
use ysm_core::session::{EffectiveGroup, EffectiveRow, GroupState};

use super::{ViewContext, begin_card, card_divider};
use crate::action::Action;
use crate::icons;
use crate::theme::{self, color, space};
use crate::widgets::int_cell::int_cell;
use crate::widgets::{self, ColumnAlign, RowBand, game_tooltip, header_cell, icon_text, value_color};

const DISCLOSURE_WIDTH: f32 = theme::GLYPH_SIZE + space::SM;
const CANDIDATE_ICON_SIZE: f32 = theme::CONTROL_HEIGHT - space::SM;

#[derive(Clone, Copy)]
enum GridRow<'a> {
    Effective(&'a EffectiveRow),
    Group(i32),
    Candidate(i32),
    Status(GroupState),
}

impl GridRow<'_> {
    fn height(&self) -> f32 {
        match self {
            Self::Effective(_) => theme::ROW_HEIGHT,
            _ => theme::CONTROL_HEIGHT,
        }
    }
}

pub fn show(ui: &mut Ui, view: &mut ViewContext<'_>) {
    // The left margin is the gap to the skill tree card.
    let frame =
        egui::Frame::new().fill(color::WINDOW_BG).inner_margin(Margin { left: space::SM as i8, ..Margin::ZERO });
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
                        ui.label(RichText::new(icons::EFFECTIVE).size(theme::GLYPH_SIZE).color(color::TEXT_MUTED));
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
    ui.visuals_mut().widgets.hovered.bg_fill = Color32::from_white_alpha(8);
    let session = &view.state.session;
    let summon_id = session.chain().and_then(|chain| chain.stage(stage)).map(|summon| summon.summon_id);
    let expansion_id = ui.make_persistent_id(("effective-expanded-groups", stage, summon_id));
    let mut expanded = ui.data(|data| data.get_temp::<HashSet<(i32, i32)>>(expansion_id)).unwrap_or_default();
    let mut toggled = None;

    TableBuilder::new(ui)
        .id_salt(("effective-grid", stage))
        .sense(Sense::click())
        .striped(true)
        .cell_layout(Layout::left_to_right(Align::Center))
        .auto_shrink(false)
        .min_scrolled_height(0.0)
        .column(Column::exact(DISCLOSURE_WIDTH + theme::ICON_SIZE + space::MD))
        .column(Column::exact(44.0))
        .column(Column::remainder().at_least(104.0).clip(true))
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
            let mut draw_effective =
                |table_row: &mut TableRow<'_, '_>, view: &mut ViewContext<'_>, row: &EffectiveRow| {
                    let key = (row.skill_id, row.lv);
                    if grid_row(table_row, view, stage, row, band, expanded.contains(&key)) {
                        toggled = Some(key);
                    }
                };
            if expanded.is_empty() {
                body.rows(theme::ROW_HEIGHT, rows.len(), |mut table_row| {
                    let row = &rows[table_row.index()];
                    draw_effective(&mut table_row, view, row);
                });
                return;
            }

            let mut grid_rows = Vec::with_capacity(rows.len());
            for row in rows {
                grid_rows.push(GridRow::Effective(row));
                if !expanded.contains(&(row.skill_id, row.lv)) {
                    continue;
                }
                for group in &row.groups {
                    if row.groups.len() > 1 {
                        grid_rows.push(GridRow::Group(group.group_id));
                    }
                    match session.overlay_group(group.group_id).map(|group| group.skill_ids()) {
                        Some(ids) if !ids.is_empty() => {
                            grid_rows.extend(ids.iter().copied().map(GridRow::Candidate));
                        }
                        _ => grid_rows.push(GridRow::Status(session.group_state(group.group_id))),
                    }
                }
            }
            body.heterogeneous_rows(grid_rows.iter().map(GridRow::height), |mut table_row| {
                let index = table_row.index();
                match grid_rows[index] {
                    GridRow::Effective(row) => draw_effective(&mut table_row, view, row),
                    row => {
                        let last = grid_rows.get(index + 1).is_none_or(|row| matches!(row, GridRow::Effective(_)));
                        preview_row(&mut table_row, view, band, row, last);
                    }
                }
            });
        });

    if let Some(key) = toggled {
        if !expanded.remove(&key) {
            expanded.insert(key);
        }
        ui.data_mut(|data| data.insert_temp(expansion_id, expanded));
        ui.ctx().request_repaint();
    }
}

fn grid_row(
    table_row: &mut TableRow<'_, '_>,
    view: &mut ViewContext<'_>,
    stage: u8,
    row: &EffectiveRow,
    band: RowBand,
    expanded: bool,
) -> bool {
    let session = &view.state.session;
    let key = (row.skill_id, row.lv);
    let selected = view.state.selected_effective == Some(key);
    let dirty_fill = (row.is_dirty && !selected).then_some(color::ROW_DIRTY);
    let display = session.skill_display(row.skill_id);
    table_row.set_selected(selected);

    let menu = |ui: &mut Ui, actions: &mut Vec<Action>| {
        ui.set_min_width(192.0);
        if ui.button(icon_text(icons::EDIT, "Edit Skill")).clicked() {
            actions.push(Action::EditSkill(row.skill_id));
            ui.close();
        }
        match row.groups.as_slice() {
            [] => {}
            [group] => group_menu_item(ui, group, actions),
            groups => {
                ui.menu_button(icon_text(icons::RANDOM_GROUP, "Edit Random Group"), |ui| {
                    for group in groups {
                        group_menu_item(ui, group, actions);
                    }
                });
            }
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
        let tint = if selected && value == 0 { color::TEXT_MUTED } else { value_color(value, false) };
        ui.centered_and_justified(|ui| {
            ui.add(egui::Label::new(RichText::new(value.to_string()).color(tint)).selectable(false));
        });
    };

    let mut toggled = false;
    // The first cell paints the row and holds the disclosure and skill icon.
    table_row.col(|ui| {
        if let Some(fill) = dirty_fill {
            band.paint(ui, fill);
        }
        ui.add_space(space::XS);
        if row.groups.is_empty() {
            ui.add_space(DISCLOSURE_WIDTH + space::XS);
        } else {
            let button = group_disclosure(ui, key, &row.groups, expanded);
            select(&button, view.actions);
            button.context_menu(|ui| menu(ui, view.actions));
            toggled = button.clicked();
        }
        let icon = view.textures.icon(display.and_then(|d| d.icon_file_name.as_deref()));
        widgets::game_icon(ui, icon, theme::ICON_SIZE).on_hover_ui(|ui| game_tooltip::skill(ui, row.skill_id, display));
    });

    table_row.col(|ui| number(ui, row.skill_id));

    let (_, name_response) = table_row.col(|ui| {
        let tint = if display.is_some() { color::TEXT } else { color::WARNING };
        // One group indicator leaves room for the skill name on merged rows.
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if !row.groups.is_empty() {
                let glyph = RichText::new(icons::RANDOM_GROUP).size(theme::GLYPH_SIZE).color(color::ACCENT_BRIGHT);
                let button = ui
                    .add_sized([theme::CONTROL_HEIGHT; 2], egui::Button::new(glyph))
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                button.widget_info(|| {
                    let label = match row.groups.as_slice() {
                        [group] => format!("Edit Random Group #{}", group.group_id),
                        _ => "Choose Random Group".into(),
                    };
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label)
                });
                select(&button, view.actions);
                if let [group] = row.groups.as_slice() {
                    let clicked = button.clicked();
                    button.on_hover_ui(|ui| {
                        game_tooltip::random_group(ui, session, group.group_id);
                        ui.label(widgets::caption("Click to edit group candidates."));
                    });
                    if clicked {
                        view.actions.push(Action::EditGroup(group.tree, group.uid));
                    }
                } else {
                    egui::Popup::menu(&button).id(egui::Id::new(("effective-group", stage, key))).show(|ui| {
                        for group in &row.groups {
                            group_menu_item(ui, group, view.actions);
                        }
                    });
                    button.on_hover_text("Choose a random group to edit");
                }
            }
            if row.linked > 1 {
                ui.label(widgets::caption(format!("×{}", row.linked)))
                    .on_hover_text(format!("Merged from {} tree rows", row.linked));
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let name = session.skill_name(row.skill_id);
                let name = RichText::new(game_tooltip::plain_skill_name(&name)).color(tint);
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
            let tint = if selected && value == 0 { color::TEXT_MUTED } else { value_color(value, false) };
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
    toggled
}

fn group_disclosure(ui: &mut Ui, key: (i32, i32), groups: &[EffectiveGroup], expanded: bool) -> egui::Response {
    let (icon, verb) = if expanded { (CARET_DOWN, "Hide") } else { (CARET_RIGHT, "Show") };
    let label = match groups {
        [group] => format!("{verb} candidates for random group {}", group.group_id),
        _ => format!("{verb} candidates for random groups"),
    };
    let response = ui
        .push_id(("effective-group-disclosure", key), |ui| {
            ui.spacing_mut().button_padding.x = space::XS;
            let glyph = RichText::new(icon).size(theme::GLYPH_SIZE).color(color::ACCENT_BRIGHT);
            ui.add_sized([DISCLOSURE_WIDTH, theme::CONTROL_HEIGHT], egui::Button::new(glyph).frame_when_inactive(false))
        })
        .inner
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(&label);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
    response
}

fn preview_row(
    table_row: &mut TableRow<'_, '_>,
    view: &mut ViewContext<'_>,
    band: RowBand,
    row: GridRow<'_>,
    last: bool,
) {
    let session = &view.state.session;
    let candidate = match row {
        GridRow::Candidate(id) => Some((id, session.skill_display(id))),
        _ => None,
    };
    table_row.col(|ui| {
        band.paint(ui, color::ACCENT_SUBTLE);
        let rect = ui.max_rect();
        let x = rect.left() + space::XS + DISCLOSURE_WIDTH / 2.0;
        let y = rect.center().y;
        let bottom = if last { y } else { rect.bottom() + space::XS };
        let stroke = egui::Stroke::new(1.0, color::BORDER_STRONG);
        let painter = ui.ctx().layer_painter(ui.layer_id()).with_clip_rect(band.clip);
        painter.vline(x, (rect.top() - space::XS)..=bottom, stroke);
        painter.hline(x..=x + space::MD, y, stroke);
        if let Some((id, display)) = candidate {
            ui.add_space(DISCLOSURE_WIDTH + space::MD);
            let icon = view.textures.icon(display.and_then(|d| d.icon_file_name.as_deref()));
            widgets::game_icon(ui, icon, CANDIDATE_ICON_SIZE).on_hover_ui(|ui| game_tooltip::skill(ui, id, display));
        }
    });
    table_row.col(|ui| {
        if let Some((id, _)) = candidate {
            ui.centered_and_justified(|ui| ui.label(widgets::caption(id.to_string())));
        }
    });
    table_row.col(|ui| {
        // Preview rows have no numeric cells after the skill name.
        let cell = ui.max_rect();
        let span = egui::Rect::from_min_max(cell.min, egui::pos2(band.x.max - space::SM, cell.bottom()));
        let mut preview = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("preview-content")
                .max_rect(span)
                .layout(Layout::left_to_right(Align::Center)),
        );
        preview.set_clip_rect(span.intersect(band.clip));
        if let Some((id, display)) = candidate {
            let name = session.skill_name(id);
            let tint = if display.is_some() { color::TEXT } else { color::WARNING };
            let name = RichText::new(game_tooltip::plain_skill_name(&name)).color(tint);
            let label = egui::Label::new(name).truncate().selectable(false).sense(Sense::click());
            let response = preview.add(label).on_hover_ui(|ui| game_tooltip::skill(ui, id, display));
            if response.double_clicked() {
                view.actions.push(Action::EditSkill(id));
            }
            return;
        }

        let (text, tint) = match row {
            GridRow::Group(id) => (format!("Random group #{id}"), color::ACCENT_BRIGHT),
            GridRow::Status(state) => match state {
                GroupState::Pending => ("No candidate skills (staged)".into(), color::DIRTY_MARK),
                GroupState::Loaded => ("No candidate skills".into(), color::TEXT_MUTED),
                GroupState::MissingInDb => ("Random group not found".into(), color::WARNING),
                GroupState::Loading => ("Loading random group…".into(), color::TEXT_MUTED),
            },
            _ => return,
        };
        let label = egui::Label::new(widgets::caption(&text).color(tint)).truncate().selectable(false);
        preview.add(label).on_hover_text(text);
    });
}

fn group_menu_item(ui: &mut Ui, group: &EffectiveGroup, actions: &mut Vec<Action>) {
    if ui.button(icon_text(icons::RANDOM_GROUP, &format!("Edit Random Group #{}", group.group_id))).clicked() {
        actions.push(Action::EditGroup(group.tree, group.uid));
        ui.close();
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use ysm_core::model::SkillTreeEntry;
    use ysm_core::session::TreeRef;
    use ysm_data::{AppSettings, InMemoryRepository, SummonRepository, loader};

    use super::*;
    use crate::state::AppState;
    use crate::textures::TextureStore;

    const SOURCE_TREE: TreeRef = TreeRef { stage: 3, slot: 3 };

    #[test]
    fn single_group_opens_its_source_row_in_another_tree() {
        let mut harness = harness(false);
        harness.run();
        let uid = harness.state().state.session.tree(SOURCE_TREE).unwrap().rows[0].uid;

        harness.get_by_label("Edit Random Group #399").click();
        harness.run();

        assert!(matches!(harness.state().actions.as_slice(), [Action::SelectEffective(_), ..]));
        assert!(harness.state().actions.iter().any(|action| matches!(
            action, Action::EditGroup(tree, row) if *tree == SOURCE_TREE && *row == uid
        )));
    }

    #[test]
    fn merged_groups_wait_for_the_user_to_pick_the_source() {
        let mut harness = harness(true);
        harness.run();
        let uid = harness.state().state.session.tree(SOURCE_TREE).unwrap().rows[0].uid;

        harness.get_by_label("Choose Random Group").click();
        harness.run();
        assert!(!harness.state().actions.iter().any(|action| matches!(action, Action::EditGroup(..))));
        harness.get_by_label_contains("Random Group #301");

        harness.get_by_label_contains("Random Group #399").click();
        harness.run();

        let opened = harness.state().actions.iter().filter_map(|action| match action {
            Action::EditGroup(tree, row) => Some((*tree, *row)),
            _ => None,
        });
        assert_eq!(opened.collect::<Vec<_>>(), [(SOURCE_TREE, uid)]);
    }

    #[test]
    fn merged_random_groups_preview_live_candidates_inline() {
        use egui_kittest::kittest::By;
        use ysm_core::random_group::RandomSkillGroup;

        let mut harness = harness(true);
        harness.run();
        let vitality = By::new().value("Blessing of Vitality");
        let wisdom = By::new().value("Blessing of Wisdom");
        assert!(harness.query(vitality.clone()).is_none());

        harness.get_by_label("Show candidates for random groups").click_accesskit();
        harness.run();
        let source = harness.get_by_label("Hide candidates for random groups").rect();
        let first_group = harness.get(By::new().value("Random group #301")).rect();
        let second_group = harness.get(By::new().value("Random group #399")).rect();
        let candidate = harness.get(vitality.clone()).rect();
        assert!(source.bottom() <= first_group.top());
        assert!(first_group.bottom() <= candidate.top());
        assert!(candidate.bottom() <= second_group.top());
        harness.get(By::new().value("Random group not found"));

        let session = &mut harness.state_mut().state.session;
        session.stage_random_group(RandomSkillGroup::new(301, [40022]));
        harness.run();
        assert!(harness.query(vitality).is_none());
        harness.get(wisdom.clone()).click();
        harness.get(wisdom.clone()).click();
        harness.run();
        assert!(harness.state().actions.iter().any(|action| matches!(action, Action::EditSkill(40022))));

        harness.get_by_label("Hide candidates for random groups").click();
        harness.run();
        assert!(harness.query(wisdom).is_none());
        assert!(!harness.state().actions.iter().any(|action| matches!(action, Action::EditGroup(..))));
    }

    struct Fixture {
        state: AppState,
        textures: Option<TextureStore>,
        actions: Vec<Action>,
    }

    fn harness(merged: bool) -> Harness<'static, Fixture> {
        let mut repo = InMemoryRepository::demo();
        let summon = repo.summon_by_id(1013).unwrap().unwrap();
        let mut bundle = loader::load_chain(&mut repo, &summon).unwrap();
        bundle.trees = Default::default();
        let row = |slot, group_id| SkillTreeEntry {
            skill_id: 40019,
            skill_group_id: group_id,
            ..SkillTreeEntry::new_row(summon.skill_tree_ids[slot], 60)
        };
        bundle.trees[2][0].push(row(0, if merged { 301 } else { 0 }));
        bundle.trees[2][2].push(row(2, 399));
        let mut state = AppState::new(AppSettings::default(), true);
        state.session.load(bundle);
        state.active_stage = 3;

        Harness::builder().with_size([650.0, 460.0]).with_step_dt(1.0 / 60.0).build_ui_state(
            |ui, fixture: &mut Fixture| {
                let Some(textures) = fixture.textures.as_mut() else {
                    theme::apply(ui.ctx());
                    fixture.textures = Some(TextureStore::new(ui.ctx()));
                    return;
                };
                let mut view = ViewContext { state: &fixture.state, textures, actions: &mut fixture.actions };
                show(ui, &mut view);
            },
            Fixture { state, textures: None, actions: Vec::new() },
        )
    }
}
