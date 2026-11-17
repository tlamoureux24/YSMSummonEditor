//! Skill tree card: tree tabs, row actions and the editable grid.

use eframe::egui::{self, Align, Layout, RichText, Sense, Ui};
use egui_extras::{Column, TableBuilder};
use ysm_core::model::{EntryField, TREE_SLOTS};
use ysm_core::session::{EntryRow, TreeRef, TreeState};

use super::{ViewContext, begin_card, card_divider};
use crate::action::{Action, PickerPurpose};
use crate::icons;
use crate::theme::{self, color, space};
use crate::widgets::int_cell::int_cell;
use crate::widgets::{self, ColumnAlign, RowBand, Tab, game_tooltip, header_cell, icon_text, value_color};
use ysm_core::i18n::{tr, trf};

/// Room the horizontal scroll bar needs below the grid.
const SCROLL_BAR_ALLOWANCE: f32 = 0.0;
const ICON_COLUMN_WIDTH: f32 = theme::ICON_SIZE + space::SM;
const SKILL_COLUMN_MIN_WIDTH: f32 = 136.0;

/// Width of a number column: IDs need room for five digits, levels do not.
fn column_width(field: EntryField) -> f32 {
    match field {
        EntryField::SkillId => 56.0,
        EntryField::SkillGroupId | EntryField::NeedSkillId(_) => 48.0,
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
        action_bar(ui, view, tree);

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

fn action_bar(ui: &mut Ui, view: &mut ViewContext<'_>, tree: Option<&TreeState>) {
    let state = view.state;
    let selected = tree.and_then(|tree| state.selected_row.and_then(|uid| tree.row(uid)));
    let editable = tree.is_some_and(TreeState::is_editable);
    let has_row = selected.is_some();
    let has_skill = selected.is_some_and(|row| row.entry.skill_id > 0);

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
            if widgets::button_enabled(ui, has_skill, icons::EDIT, tr("Edit Skill")).clicked()
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
}

fn grid(ui: &mut Ui, view: &mut ViewContext<'_>, tree: &TreeState) {
    let available_height = ui.available_height() - SCROLL_BAR_ALLOWANCE;

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
                    header_cell(ui, tr("Skill"), tr("Skill name; double-click to edit the skill"), ColumnAlign::Left)
                });
                for field in EntryField::ALL.into_iter().skip(1) {
                    header.col(|ui| header_cell(ui, field.header(), field.description(), ColumnAlign::Center));
                }
            })
            .body(|mut body| {
                let band = RowBand { x: body.max_rect().x_range(), clip: body.ui_mut().clip_rect() };
                body.rows(theme::ROW_HEIGHT, tree.rows.len(), |mut table_row| {
                    let row = &tree.rows[table_row.index()];
                    grid_row(&mut table_row, view, tree, row, band);
                });
            });
    });
}

fn grid_row(
    table_row: &mut egui_extras::TableRow<'_, '_>,
    view: &mut ViewContext<'_>,
    tree: &TreeState,
    row: &EntryRow,
    band: RowBand,
) {
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

    // Icon. The first cell also paints the background of the whole row.
    table_row.col(|ui| {
        if let Some(fill) = dirty_fill {
            band.paint(ui, fill);
        }
        // Centres the icon in its column.
        ui.add_space(space::XS);
        let icon = view.textures.icon(display.and_then(|d| d.icon_file_name.as_deref()));
        let response = widgets::game_icon(ui, icon, theme::ICON_SIZE);
        if skill_id > 0 {
            response.on_hover_ui(|ui| game_tooltip::skill(ui, skill_id, display));
        }
    });

    // Skill ID
    table_row.col(|ui| number_cell(ui, view.actions, tree, row, EntryField::SkillId, selected, &menu, &select));

    // Skill name and random group badge
    let (_, name_response) = table_row.col(|ui| {
        let (name, tint) = if skill_id <= 0 {
            (tr("New skill").to_owned(), color::text_faint())
        } else if display.is_none() {
            (session.skill_name(skill_id), color::warning())
        } else {
            (session.skill_name(skill_id), color::text())
        };
        // The badge takes its place at the right edge first; the name truncates in what is left.
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let group_id = row.entry.skill_group_id;
            if group_id > 0 {
                widgets::badge(ui, &format!("RND[{group_id}]"), color::accent_bright())
                    .on_hover_ui(|ui| game_tooltip::random_group(ui, session, group_id));
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let label = ui.add(egui::Label::new(RichText::new(name).color(tint)).truncate().selectable(false));
                if skill_id > 0 {
                    label.on_hover_ui(|ui| game_tooltip::skill(ui, skill_id, display));
                }
            });
        });
    });
    if name_response.double_clicked() && skill_id > 0 {
        view.actions.push(Action::EditSkill(skill_id));
    }

    for field in EntryField::ALL.into_iter().skip(1) {
        table_row.col(|ui| number_cell(ui, view.actions, tree, row, field, selected, &menu, &select));
    }

    let response = table_row.response();
    select(&response, view.actions);
    response.context_menu(|ui| menu(ui, view.actions));
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
    use egui_kittest::kittest::Queryable;
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
    fn tree_tabs_switch_the_slot() {
        let mut harness = harness();
        harness.run();
        harness.get_by_label("SkillTree 4").click();
        harness.run();

        assert!(harness.state().actions.iter().any(|a| matches!(a, Action::SetSlot(4))));
    }
}
