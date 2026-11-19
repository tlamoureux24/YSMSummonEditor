//! Skill tree card: tree tabs, row actions and the editable grid.

use std::collections::HashSet;

use eframe::egui::{self, Align, Layout, RichText, Sense, Ui};
use egui_extras::{Column, TableBuilder, TableRow};
use ysm_core::model::{EntryField, TREE_SLOTS};
use ysm_core::session::{EntryRow, GroupState, TreeRef, TreeState};

use super::{ViewContext, begin_card, card_divider};
use crate::action::{Action, PickerPurpose};
use crate::icons;
use crate::theme::{self, color, space};
use crate::widgets::int_cell::int_cell;
use crate::widgets::{self, ColumnAlign, RowBand, Tab, game_tooltip, header_cell, icon_text, value_color};
use ysm_core::i18n::{tr, trf};

/// Room the horizontal scroll bar needs below the grid.
const SCROLL_BAR_ALLOWANCE: f32 = 0.0;
const DISCLOSURE_WIDTH: f32 = theme::GLYPH_SIZE + space::SM;
const ICON_COLUMN_WIDTH: f32 = DISCLOSURE_WIDTH + theme::ICON_SIZE + space::MD;
const SKILL_COLUMN_MIN_WIDTH: f32 = 136.0;
const CANDIDATE_ROW_HEIGHT: f32 = theme::CONTROL_HEIGHT;
const CANDIDATE_ICON_SIZE: f32 = CANDIDATE_ROW_HEIGHT - space::SM;

#[derive(Clone, Copy)]
enum GridRow<'a> {
    Entry(&'a EntryRow),
    Candidate { skill_id: i32, last: bool },
    GroupStatus(GroupState),
}

impl GridRow<'_> {
    fn height(&self) -> f32 {
        match self {
            Self::Entry(_) => theme::ROW_HEIGHT,
            Self::Candidate { .. } | Self::GroupStatus(_) => CANDIDATE_ROW_HEIGHT,
        }
    }
}

/// Width of a number column: IDs need room for five digits, levels do not.
fn column_width(field: EntryField) -> f32 {
    match field {
        EntryField::SkillId => 56.0,
        EntryField::SkillGroupId => 48.0 + theme::CONTROL_HEIGHT + space::XS,
        EntryField::NeedSkillId(_) => 48.0,
        _ => 36.0,
    }
}

pub fn show(ui: &mut Ui, view: &mut ViewContext<'_>) {
    theme::card().show(ui, |ui| {
        ui.set_min_size(ui.available_size());
        let state = view.state;
        let tree = state.active_tree();

        begin_card(ui);
        tree_tabs(ui, view);
        card_divider(ui);
        let group_to_edit = action_bar(ui, view, tree);

        match tree {
            None => {
                widgets::empty_state(
                    ui,
                    icons::TREE,
                    tr("No skill trees loaded"),
                    tr("Select a summon to edit its skill trees."),
                );
            }
            Some(tree) if tree.tree_id <= 0 => {
                widgets::empty_state(
                    ui,
                    icons::TREE,
                    tr("No skill tree in this slot"),
                    tr("The summon has no tree assigned here."),
                );
            }
            Some(tree) if tree.rows.is_empty() => {
                widgets::empty_state(
                    ui,
                    icons::ADD_SKILL,
                    tr("This tree is empty"),
                    tr("Use Add Row or Add Skill to start."),
                );
            }
            Some(tree) => grid(ui, view, tree),
        }
        if let Some(action) = group_to_edit {
            view.actions.push(action);
        }
    });
}

fn tree_tabs(ui: &mut Ui, view: &mut ViewContext<'_>) {
    let state = view.state;
    let tabs: Vec<Tab> = (1..=TREE_SLOTS as u8)
        .map(|slot| {
            let tree = state.session.tree(TreeRef::new(state.active_stage, slot));
            let tree_id = tree.map_or(0, |t| t.tree_id);
            Tab {
                title: trf("SkillTree {}", &[&slot]),
                subtitle: if tree_id > 0 { format!("#{tree_id}") } else { String::new() },
                enabled: tree_id > 0,
                dirty: tree.is_some_and(TreeState::is_changed),
                tooltip: if tree_id > 0 {
                    trf("Tree slot {}, skill_tree_id {}", &[&slot, &tree_id])
                } else {
                    trf("Tree slot {} is empty", &[&slot])
                },
            }
        })
        .collect();

    if let Some(index) = widgets::tab_strip(ui, &tabs, usize::from(state.active_slot.saturating_sub(1))) {
        view.actions.push(Action::SetSlot(index as u8 + 1));
    }
}

fn action_bar(ui: &mut Ui, view: &mut ViewContext<'_>, tree: Option<&TreeState>) -> Option<Action> {
    let state = view.state;
    let selected = tree.and_then(|tree| state.selected_row.and_then(|uid| tree.row(uid)));
    let editable = tree.is_some_and(TreeState::is_editable);
    let has_row = selected.is_some();
    let has_skill = selected.is_some_and(|row| row.entry.skill_id > 0);
    let has_group = selected.is_some_and(|row| row.entry.skill_group_id > 0);

    // The right-hand side only reads, so it is prepared before the buttons borrow the action list.
    let summary = tree.filter(|tree| tree.tree_id > 0).map(|tree| {
        let dirty = tree.rows.iter().filter(|row| row.is_dirty()).count();
        if dirty > 0 {
            trf("{} rows · {} changed", &[&tree.rows.len(), &dirty])
        } else {
            trf("{} rows", &[&tree.rows.len()])
        }
    });
    let shared = tree.map(|tree| state.session.trees_sharing_id(tree.tree)).unwrap_or_default();
    let shared_note = (!shared.is_empty()).then(|| {
        let places: Vec<String> =
            shared.iter().map(|t| trf("Evolution {} / SkillTree {}", &[&t.stage, &t.slot])).collect();
        trf(
            "This tree is also used by {} in this chain. Edit it in one place only: each tab is saved separately.",
            &[&places.join(", ")],
        )
    });

    let mut group_to_edit = None;
    let actions = &mut *view.actions;
    widgets::control_row(
        ui,
        |ui| {
            let Some(tree) = tree else {
                ui.label(widgets::muted(tr("No tree selected.")));
                return;
            };
            if widgets::button_enabled(ui, editable, icons::ADD, tr("Add Row"))
                .on_hover_text(tr("Append an empty row (Insert)"))
                .clicked()
            {
                actions.push(Action::AddRow(tree.tree));
            }
            if widgets::button_enabled(ui, editable, icons::ADD_SKILL, tr("Add Skill…"))
                .on_hover_text(tr("Search for a skill by name and append it"))
                .clicked()
            {
                actions.push(Action::OpenSkillPicker(PickerPurpose::AddRow(tree.tree)));
            }
            ui.add(egui::Separator::default().vertical().spacing(space::SM));
            if widgets::button_enabled(ui, has_row, icons::DUPLICATE, tr("Duplicate")).clicked()
                && let Some(row) = selected
            {
                actions.push(Action::DuplicateRow(tree.tree, row.uid));
            }
            if widgets::button_enabled(ui, has_row, icons::DELETE, tr("Delete"))
                .on_hover_text(tr("Delete the selected row (Del)"))
                .clicked()
                && let Some(row) = selected
            {
                actions.push(Action::AskDeleteRow(tree.tree, row.uid));
            }
            if has_group {
                if widgets::button(ui, icons::RANDOM_GROUP, tr("Edit Group")).clicked()
                    && let Some(row) = selected
                {
                    // Let the grid commit an in-progress ID edit before opening the group.
                    group_to_edit = Some(Action::EditGroup(tree.tree, row.uid));
                }
            } else if widgets::button_enabled(ui, has_skill, icons::EDIT, tr("Edit Skill")).clicked()
                && let Some(row) = selected
            {
                actions.push(Action::EditSkill(row.entry.skill_id));
            }
        },
        |ui| {
            if let Some(summary) = &summary {
                ui.label(widgets::caption(summary.as_str()));
            }
            if let Some(note) = &shared_note {
                ui.label(
                    RichText::new(icon_text(icons::WARNING, tr("Shared in chain")))
                        .size(theme::CAPTION_SIZE)
                        .color(color::warning()),
                )
                .on_hover_text(note.as_str());
            }
        },
    );
    group_to_edit
}

fn grid(ui: &mut Ui, view: &mut ViewContext<'_>, tree: &TreeState) {
    let available_height = ui.available_height() - SCROLL_BAR_ALLOWANCE;
    let expansion_id = ui.make_persistent_id(("tree-expanded-groups", tree.tree));
    let mut expanded = ui.data(|data| data.get_temp::<HashSet<u64>>(expansion_id)).unwrap_or_default();
    let mut toggled = None;

    egui::ScrollArea::horizontal().id_salt(("tree-scroll", tree.tree)).auto_shrink(false).show(ui, |ui| {
        ui.spacing_mut().item_spacing = egui::Vec2::splat(space::XS);
        // A light tint, so hovering never hides the colour of a changed row.
        ui.visuals_mut().widgets.hovered.bg_fill = color::hover_tint();

        let mut table = TableBuilder::new(ui)
            .id_salt(("tree-grid", tree.tree))
            .sense(Sense::click())
            .striped(true)
            .cell_layout(Layout::left_to_right(Align::Center))
            .auto_shrink(false)
            .min_scrolled_height(0.0)
            .max_scroll_height(available_height)
            .column(Column::exact(ICON_COLUMN_WIDTH))
            .column(Column::exact(column_width(EntryField::SkillId)))
            .column(Column::remainder().at_least(SKILL_COLUMN_MIN_WIDTH).clip(true));
        for field in EntryField::ALL.into_iter().skip(1) {
            table = table.column(Column::exact(column_width(field)));
        }

        table
            .header(theme::HEADER_HEIGHT, |mut header| {
                header.col(|_| {});
                header.col(|ui| {
                    header_cell(
                        ui,
                        EntryField::SkillId.header(),
                        EntryField::SkillId.description(),
                        ColumnAlign::Center,
                    );
                });
                header.col(|ui| {
                    header_cell(ui, tr("Skill"), tr("Double-click a skill name to edit the skill."), ColumnAlign::Left)
                });
                for field in EntryField::ALL.into_iter().skip(1) {
                    header.col(|ui| {
                        let tooltip = if field == EntryField::SkillGroupId {
                            tr("Type a group ID. The dice opens the group editor and the chevron previews candidates.")
                        } else {
                            field.description()
                        };
                        header_cell(ui, field.header(), tooltip, ColumnAlign::Center);
                    });
                }
            })
            .body(|mut body| {
                let band = RowBand { x: body.max_rect().x_range(), clip: body.ui_mut().clip_rect() };
                let session = &view.state.session;
                let mut draw_row = |mut table_row: TableRow<'_, '_>, row| match row {
                    GridRow::Entry(entry) => {
                        if grid_row(&mut table_row, view, tree, entry, band, expanded.contains(&entry.uid)) {
                            toggled = Some(entry.uid);
                        }
                    }
                    GridRow::Candidate { skill_id, last } => {
                        candidate_row(&mut table_row, view, band, skill_id, last);
                    }
                    GridRow::GroupStatus(state) => group_status_row(&mut table_row, band, state),
                };
                if expanded.is_empty() {
                    body.rows(theme::ROW_HEIGHT, tree.rows.len(), |table_row| {
                        let entry = &tree.rows[table_row.index()];
                        draw_row(table_row, GridRow::Entry(entry));
                    });
                } else {
                    let mut rows = Vec::with_capacity(tree.rows.len());
                    for entry in &tree.rows {
                        rows.push(GridRow::Entry(entry));
                        let group_id = entry.entry.skill_group_id;
                        if group_id <= 0 || !expanded.contains(&entry.uid) {
                            continue;
                        }
                        match session.overlay_group(group_id).map(|group| group.skill_ids()) {
                            Some(ids) if !ids.is_empty() => {
                                rows.extend(
                                    ids.iter().enumerate().map(|(i, &skill_id)| GridRow::Candidate {
                                        skill_id,
                                        last: i + 1 == ids.len(),
                                    }),
                                );
                            }
                            _ => rows.push(GridRow::GroupStatus(session.group_state(group_id))),
                        }
                    }
                    body.heterogeneous_rows(rows.iter().map(GridRow::height), |table_row| {
                        let row = rows[table_row.index()];
                        draw_row(table_row, row);
                    });
                }
            });
    });

    if let Some(uid) = toggled {
        expanded.retain(|uid| tree.row(*uid).is_some());
        if !expanded.remove(&uid) {
            expanded.insert(uid);
        }
        ui.data_mut(|data| data.insert_temp(expansion_id, expanded));
        ui.ctx().request_repaint();
    }
}

fn grid_row(
    table_row: &mut TableRow<'_, '_>,
    view: &mut ViewContext<'_>,
    tree: &TreeState,
    row: &EntryRow,
    band: RowBand,
    expanded: bool,
) -> bool {
    let state = view.state;
    let session = &state.session;
    let selected = state.selected_row == Some(row.uid);
    let dirty_fill = (row.is_dirty() && !selected).then_some(color::row_dirty());
    let skill_id = row.entry.skill_id;
    let display = session.skill_display(skill_id);
    table_row.set_selected(selected);

    let menu = |ui: &mut Ui, actions: &mut Vec<Action>| row_menu(ui, actions, tree, row);
    let select = |response: &egui::Response, actions: &mut Vec<Action>| {
        if (response.clicked() || response.secondary_clicked() || response.gained_focus()) && !selected {
            actions.push(Action::SelectRow(Some(row.uid)));
        }
    };

    let mut toggled = false;
    // The first cell paints the row and holds the disclosure and skill icon.
    table_row.col(|ui| {
        if let Some(fill) = dirty_fill {
            band.paint(ui, fill);
        }
        ui.add_space(space::XS);
        let group_id = row.entry.skill_group_id;
        if group_id > 0 {
            let button = group_disclosure(ui, row.uid, group_id, expanded);
            select(&button, view.actions);
            button.context_menu(|ui| menu(ui, view.actions));
            toggled = button.clicked();
        } else {
            ui.add_space(DISCLOSURE_WIDTH + space::XS);
        }
        let icon = view.textures.icon(display.and_then(|d| d.icon_file_name.as_deref()));
        let response = widgets::game_icon(ui, icon, theme::ICON_SIZE);
        if skill_id > 0 {
            response.on_hover_ui(|ui| game_tooltip::skill(ui, skill_id, display));
        }
    });

    // Skill ID
    table_row.col(|ui| number_cell(ui, view.actions, tree, row, EntryField::SkillId, selected, &menu, &select));

    // The skill name opens the individual skill editor.
    table_row.col(|ui| {
        let (name, tint) = if skill_id <= 0 {
            (tr("New skill").to_owned(), color::text_faint())
        } else if display.is_none() {
            (session.skill_name(skill_id), color::warning())
        } else {
            (session.skill_name(skill_id), color::text())
        };
        let name = RichText::new(game_tooltip::plain_skill_name(&name)).color(tint);
        let response = ui.add(egui::Label::new(name).truncate().selectable(false).sense(Sense::click()));
        select(&response, view.actions);
        response.context_menu(|ui| menu(ui, view.actions));
        if skill_id > 0 {
            if response.double_clicked() {
                view.actions.push(Action::EditSkill(skill_id));
            }
            response.on_hover_ui(|ui| game_tooltip::skill(ui, skill_id, display));
        }
    });

    for field in EntryField::ALL.into_iter().skip(1) {
        table_row.col(|ui| {
            let mut edit_group = false;
            if field == EntryField::SkillGroupId {
                let group_id = row.entry.skill_group_id;
                if group_id > 0 {
                    let icon = RichText::new(icons::RANDOM_GROUP).size(theme::GLYPH_SIZE).color(color::accent_bright());
                    let button = ui
                        .add_sized([theme::CONTROL_HEIGHT; 2], egui::Button::new(icon))
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_ui(|ui| {
                            game_tooltip::random_group(ui, session, group_id);
                            ui.label(widgets::caption(tr("Click to edit group candidates.")));
                        });
                    button.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tr("Edit Random Group"))
                    });
                    select(&button, view.actions);
                    button.context_menu(|ui| menu(ui, view.actions));
                    edit_group = button.clicked();
                } else {
                    ui.add_space(theme::CONTROL_HEIGHT + space::XS);
                }
            }
            number_cell(ui, view.actions, tree, row, field, selected, &menu, &select);
            if edit_group {
                view.actions.push(Action::EditGroup(tree.tree, row.uid));
            }
        });
    }

    let response = table_row.response();
    select(&response, view.actions);
    response.context_menu(|ui| menu(ui, view.actions));
    toggled
}

fn group_disclosure(ui: &mut Ui, uid: u64, group_id: i32, expanded: bool) -> egui::Response {
    use egui_phosphor::regular::{CARET_DOWN, CARET_RIGHT};

    let (icon, verb) = if expanded { (CARET_DOWN, tr("Hide")) } else { (CARET_RIGHT, tr("Show")) };
    let label = trf("{} candidates for random group {}", &[&verb, &group_id]);
    let scope = egui::UiBuilder::new().id(egui::Id::new(("tree-group-disclosure", uid)));
    let response = ui
        .scope_builder(scope, |ui| {
            ui.spacing_mut().button_padding.x = space::XS;
            let glyph = RichText::new(icon).size(theme::GLYPH_SIZE).color(color::accent_bright());
            ui.add_sized([DISCLOSURE_WIDTH, theme::CONTROL_HEIGHT], egui::Button::new(glyph).frame_when_inactive(false))
        })
        .inner
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(&label);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
    response
}

fn candidate_row(
    table_row: &mut TableRow<'_, '_>,
    view: &mut ViewContext<'_>,
    band: RowBand,
    skill_id: i32,
    last: bool,
) {
    let display = view.state.session.skill_display(skill_id);
    table_row.col(|ui| {
        candidate_branch(ui, band, last);
        ui.add_space(DISCLOSURE_WIDTH + space::MD);
        let icon = view.textures.icon(display.and_then(|d| d.icon_file_name.as_deref()));
        widgets::game_icon(ui, icon, CANDIDATE_ICON_SIZE).on_hover_ui(|ui| game_tooltip::skill(ui, skill_id, display));
    });
    table_row.col(|ui| {
        ui.centered_and_justified(|ui| {
            ui.label(widgets::caption(skill_id.to_string()));
        });
    });
    table_row.col(|ui| {
        let name = view.state.session.skill_name(skill_id);
        let tint = if display.is_some() { color::text() } else { color::warning() };
        let name = RichText::new(game_tooltip::plain_skill_name(&name)).color(tint);
        let label = egui::Label::new(name).truncate().selectable(false).sense(Sense::click());
        let response = ui.add(label).on_hover_ui(|ui| game_tooltip::skill(ui, skill_id, display));
        if response.double_clicked() {
            view.actions.push(Action::EditSkill(skill_id));
        }
    });
}

fn group_status_row(table_row: &mut TableRow<'_, '_>, band: RowBand, state: GroupState) {
    let (text, tint) = match state {
        GroupState::Pending => (tr("No candidate skills (staged)"), color::dirty_mark()),
        GroupState::Loaded => (tr("No candidate skills"), color::text_muted()),
        GroupState::MissingInDb => (tr("Random group not found"), color::warning()),
        GroupState::Loading => (tr("Loading random group…"), color::text_muted()),
    };
    table_row.col(|ui| candidate_branch(ui, band, true));
    table_row.col(|_| {});
    table_row.col(|ui| {
        let label = egui::Label::new(RichText::new(text).color(tint)).truncate().selectable(false);
        ui.add(label).on_hover_text(text);
    });
}

fn candidate_branch(ui: &Ui, band: RowBand, last: bool) {
    band.paint(ui, color::accent_subtle());
    let rect = ui.max_rect();
    let x = rect.left() + space::XS + DISCLOSURE_WIDTH / 2.0;
    let y = rect.center().y;
    let bottom = if last { y } else { rect.bottom() + space::XS };
    let stroke = egui::Stroke::new(1.0, color::border_strong());
    let painter = ui.ctx().layer_painter(ui.layer_id()).with_clip_rect(band.clip);
    painter.vline(x, (rect.top() - space::XS)..=bottom, stroke);
    painter.hline(x..=x + space::MD, y, stroke);
}

fn number_cell(
    ui: &mut Ui,
    actions: &mut Vec<Action>,
    tree: &TreeState,
    row: &EntryRow,
    field: EntryField,
    selected: bool,
    menu: &impl Fn(&mut Ui, &mut Vec<Action>),
    select: &impl Fn(&egui::Response, &mut Vec<Action>),
) {
    let value = row.entry.get(field);
    let changed = row.original.is_none_or(|original| original.get(field) != value);
    let id = egui::Id::new(("tree-cell", row.uid, field));

    // Dimmed zeros would vanish on the selection colour.
    let tint = if selected && !changed && value == 0 { color::text_muted() } else { value_color(value, changed) };
    let output = int_cell(ui, id, value, tint);
    select(&output.response, actions);
    if let Some(value) = output.committed {
        actions.push(Action::SetField { tree: tree.tree, uid: row.uid, field, value });
    }
    output.response.context_menu(|ui| menu(ui, actions));
}

fn row_menu(ui: &mut Ui, actions: &mut Vec<Action>, tree: &TreeState, row: &EntryRow) {
    ui.set_min_width(216.0);
    let has_skill = row.entry.skill_id > 0;
    let has_group = row.entry.skill_group_id > 0;
    let mut item = |ui: &mut Ui, enabled: bool, icon: &str, text: &str, action: Action| {
        if ui.add_enabled(enabled, egui::Button::new(icon_text(icon, text))).clicked() {
            actions.push(action);
            ui.close();
        }
    };

    item(ui, tree.is_editable(), icons::ADD, tr("Add Row"), Action::AddRow(tree.tree));
    item(
        ui,
        tree.is_editable(),
        icons::ADD_SKILL,
        tr("Add Skill…"),
        Action::OpenSkillPicker(PickerPurpose::AddRow(tree.tree)),
    );
    item(
        ui,
        true,
        icons::SWAP,
        tr("Change Skill…"),
        Action::OpenSkillPicker(PickerPurpose::ChangeSkill(tree.tree, row.uid)),
    );
    item(ui, true, icons::DUPLICATE, tr("Duplicate Row"), Action::DuplicateRow(tree.tree, row.uid));
    item(ui, has_skill, icons::EDIT, tr("Edit Skill"), Action::EditSkill(row.entry.skill_id));
    item(ui, true, icons::DELETE, tr("Delete From Tree"), Action::AskDeleteRow(tree.tree, row.uid));
    ui.separator();
    item(ui, !has_group, icons::RANDOM_GROUP, tr("Create Random Group"), Action::CreateGroup(tree.tree, row.uid));
    item(ui, has_group, icons::RANDOM_GROUP, tr("Edit Random Group"), Action::EditGroup(tree.tree, row.uid));
    item(ui, has_group, icons::DETACH, tr("Detach Random Group"), Action::AskDetachGroup(tree.tree, row.uid));
}

#[cfg(test)]
mod tests {
    use eframe::egui::accesskit::Role;
    use egui_kittest::Harness;
    use egui_kittest::kittest::{By, Queryable};
    use ysm_data::{AppSettings, InMemoryRepository, SummonRepository, loader};

    use super::*;
    use crate::state::AppState;
    use crate::textures::TextureStore;

    struct Fixture {
        state: AppState,
        textures: Option<TextureStore>,
        actions: Vec<Action>,
    }

    /// The tree panel showing the first tree of the demo summon 1013.
    fn harness<'a>() -> Harness<'a, Fixture> {
        let mut repo = InMemoryRepository::demo();
        let summon = repo.summon_by_id(1013).unwrap().unwrap();
        let mut state = AppState::new(AppSettings::default(), true);
        state.session.load(loader::load_chain(&mut repo, &summon).unwrap());
        state.active_stage = 3;

        Harness::builder().with_size([1100.0, 700.0]).build_ui_state(
            |ui, fixture: &mut Fixture| {
                // Fonts set through the context apply from the next frame, so the first one draws nothing.
                let Some(textures) = fixture.textures.as_mut() else {
                    crate::theme::apply(ui.ctx());
                    fixture.textures = Some(TextureStore::new(ui.ctx()));
                    return;
                };
                let mut view = ViewContext { state: &fixture.state, textures, actions: &mut fixture.actions };
                show(ui, &mut view);
            },
            Fixture { state, textures: None, actions: Vec::new() },
        )
    }

    fn first_row_uid(harness: &Harness<'_, Fixture>) -> u64 {
        harness.state().state.active_tree().unwrap().rows[0].uid
    }

    #[test]
    fn right_click_on_a_cell_opens_the_row_menu() {
        let mut harness = harness();
        harness.run();
        let uid = first_row_uid(&harness);

        harness.get_all_by_role(Role::TextInput).next().unwrap().click_secondary();
        harness.run();
        harness.get_by_label_contains("Delete From Tree").click();
        harness.run();

        let actions = &harness.state().actions;
        assert!(actions.iter().any(|a| matches!(a, Action::SelectRow(Some(selected)) if *selected == uid)));
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, Action::AskDeleteRow(tree, row) if *tree == TreeRef::new(3, 1) && *row == uid))
        );
    }

    #[test]
    fn typing_in_a_cell_pushes_a_field_edit() {
        let mut harness = harness();
        harness.run();
        let uid = first_row_uid(&harness);

        // The first text input of a row is its skill ID.
        harness.get_all_by_role(Role::TextInput).next().unwrap().focus();
        harness.run();
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.run();
        harness.get_all_by_role(Role::TextInput).next().unwrap().type_text("40010");
        harness.run();
        harness.key_press(egui::Key::Enter);
        harness.run();

        assert!(harness.state().actions.iter().any(|a| matches!(
            a,
            Action::SetField { uid: row, field: EntryField::SkillId, value: 40010, .. } if *row == uid
        )));
    }

    #[test]
    fn add_row_button_is_wired() {
        let mut harness = harness();
        harness.run();
        harness.get_by_label_contains("Add Row").click();
        harness.run();

        assert!(
            harness.state().actions.iter().any(|a| matches!(a, Action::AddRow(tree) if *tree == TreeRef::new(3, 1)))
        );
    }

    #[test]
    fn random_group_button_opens_the_group_editor() {
        let mut harness = harness();
        harness.run();
        let tree = harness.state().state.active_tree().unwrap();
        let tree_ref = tree.tree;
        let row = tree.rows.iter().find(|row| row.entry.skill_group_id > 0).unwrap();
        let uid = row.uid;
        let original_id = row.entry.skill_group_id.to_string();
        let group_cell = By::new().role(Role::TextInput).value(&original_id);

        harness.get(group_cell.clone()).focus();
        harness.run();
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.run();
        harness.get(group_cell).type_text("1234");
        harness.run();
        harness.get_by_label("Edit Random Group").click();
        harness.run();

        let actions = &harness.state().actions;
        let edit = actions
            .iter()
            .position(|a| matches!(a, Action::EditGroup(tree, row) if *tree == tree_ref && *row == uid))
            .unwrap();
        assert!(actions[..edit].iter().any(|a| matches!(
            a,
            Action::SetField { tree, uid: row, field: EntryField::SkillGroupId, value: 1234 }
                if *tree == tree_ref && *row == uid
        )));
        assert!(!actions.iter().any(|a| matches!(a, Action::EditSkill(_))));
    }

    #[test]
    fn expanding_a_group_previews_its_staged_candidates_inline() {
        let mut harness = harness();
        harness.run();
        let vitality = By::new().value("Blessing of Vitality");
        let wisdom = By::new().value("Blessing of Wisdom");
        assert!(harness.query(vitality.clone()).is_none());

        harness.get_by_label("Show candidates for random group 301").click();
        harness.run();
        harness.get_by_label("Hide candidates for random group 301").scroll_down();
        harness.run();
        let source = harness.get_by_label("Hide candidates for random group 301").rect();
        let candidate = harness.get(vitality.clone()).rect();
        assert!(candidate.top() >= source.bottom());
        assert!(harness.query(By::new().role(Role::Label).value("40020")).is_some());

        let group = ysm_core::random_group::RandomSkillGroup::new(301, [40022]);
        harness.state_mut().state.session.stage_random_group(group);
        harness.run();
        assert!(harness.query(vitality).is_none());
        assert!(harness.query(wisdom.clone()).is_some());

        harness.get_by_label("Hide candidates for random group 301").click_accesskit();
        harness.run();
        assert!(harness.query(wisdom).is_none());
        assert!(!harness.state().actions.iter().any(|a| matches!(a, Action::EditGroup(_, _))));
    }

    #[test]
    fn tree_tabs_switch_the_slot() {
        let mut harness = harness();
        harness.run();
        harness.get_by_label("SkillTree 4").click();
        harness.run();

        assert!(harness.state().actions.iter().any(|a| matches!(a, Action::SetSlot(4))));
    }
}
