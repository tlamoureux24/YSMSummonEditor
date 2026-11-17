//! Left panel: searchable, virtualized list of summons.

use std::time::Instant;

use eframe::egui::{self, Align, Align2, CornerRadius, Key, Layout, Margin, RichText, Sense, Ui, Vec2};
use ysm_core::model::SummonListItem;

use crate::action::Action;
use crate::icons;
use crate::state::{AppState, ConnectionStatus};
use crate::textures::TextureStore;
use crate::theme::{self, color, space};
use crate::widgets;

/// Takes the state mutably because the search box and filter toggles are
/// bound to it directly; everything else still goes through actions.
pub fn show(ui: &mut Ui, state: &mut AppState, textures: &mut TextureStore, actions: &mut Vec<Action>) {
    let frame = egui::Frame::new().fill(color::window_bg()).inner_margin(Margin {
        left: space::SM as i8,
        right: 0,
        top: 2,
        bottom: space::SM as i8,
    });
    egui::Panel::left("summon-list")
        .frame(frame)
        .resizable(true)
        .show_separator_line(false)
        .default_size(300.0)
        .min_size(240.0)
        .show(ui, |ui| {
            theme::card().show(ui, |ui| {
                ui.set_min_height(ui.available_height());
                ui.spacing_mut().item_spacing.y = space::SM;
                search_row(ui, state, actions);
                count_row(ui, state);
                list(ui, state, textures, actions);
            });
        });
}

fn search_row(ui: &mut Ui, state: &mut AppState, actions: &mut Vec<Action>) {
    // Right to left: the filter button takes its fixed width and the search box the rest.
    let row = Vec2::new(ui.available_width(), theme::CONTROL_HEIGHT);
    ui.allocate_ui_with_layout(row, Layout::right_to_left(Align::Center), |ui| {
        let filter_active = state.filter.include_riding_only || state.filter.final_only;
        let tint = if filter_active { color::accent_bright() } else { color::text() };
        let icon = RichText::new(icons::FILTER).size(theme::GLYPH_SIZE).color(tint);
        let filter = egui::Button::new(icon).min_size(Vec2::splat(theme::CONTROL_HEIGHT));
        let keep_open =
            egui::containers::menu::MenuConfig::new().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
        widgets::square_button_padding(ui);
        egui::containers::menu::MenuButton::from_button(filter)
            .config(keep_open)
            .ui(ui, |ui| {
                ui.set_min_width(220.0);
                let mut changed = ui.checkbox(&mut state.filter.final_only, "Final evolutions only").changed();
                changed |= ui.checkbox(&mut state.filter.include_riding_only, "Include riding-only summons").changed();
                if changed {
                    actions.push(Action::Search);
                }
            })
            .0
            .on_hover_text("Filter the list");

        let search = ui.add(
            widgets::text_field(&mut state.search_text)
                .hint_text(format!("{}  Search by ID, name or card", icons::SEARCH))
                .desired_width(f32::INFINITY),
        );
        if std::mem::take(&mut state.focus_search) {
            search.request_focus();
        }
        if search.changed() {
            state.search_edited_at = Some(Instant::now());
        }
        if search.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter)) {
            state.search_edited_at = None;
            actions.push(Action::Search);
        }
    });
}

fn count_row(ui: &mut Ui, state: &AppState) {
    ui.horizontal(|ui| {
        ui.set_height(16.0);
        if state.summons_loading {
            ui.add(egui::Spinner::new().size(theme::CAPTION_SIZE));
            ui.label(widgets::caption("Loading summons…"));
        } else {
            ui.label(widgets::caption(format!("{} summons", state.summons.len())));
        }
    });
}

fn list(ui: &mut Ui, state: &AppState, textures: &mut TextureStore, actions: &mut Vec<Action>) {
    if state.summons.is_empty() && !state.summons_loading {
        match &state.connection {
            ConnectionStatus::NotConfigured | ConnectionStatus::Failed(_) => {
                widgets::empty_state(ui, icons::NO_CONNECTION, "No database connection", "Set it up to load summons.");
                ui.vertical_centered(|ui| {
                    if widgets::button(ui, icons::SETTINGS, "Open Settings").clicked() {
                        actions.push(Action::OpenSettings);
                    }
                });
            }
            ConnectionStatus::Connecting => widgets::empty_state(ui, icons::CONNECTION, "Connecting…", ""),
            _ => widgets::empty_state(ui, icons::SEARCH, "No summons found", "Try a different search."),
        }
        return;
    }

    ui.spacing_mut().item_spacing.y = 0.0;
    egui::ScrollArea::vertical().auto_shrink(false).show_rows(
        ui,
        theme::LIST_ROW_HEIGHT,
        state.summons.len(),
        |ui, range| {
            for summon in &state.summons[range] {
                let selected = state.selected_summon == Some(summon.summon_id);
                if row(ui, summon, selected, textures).clicked() && !selected {
                    actions.push(Action::SelectSummon(summon.clone()));
                }
            }
        },
    );
}

fn row(ui: &mut Ui, summon: &SummonListItem, selected: bool, textures: &mut TextureStore) -> egui::Response {
    let size = Vec2::new(ui.available_width(), theme::LIST_ROW_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if !ui.is_rect_visible(rect) {
        return response;
    }

    // A 2 px inset keeps neighbouring highlights from touching.
    let highlight = rect.shrink2(Vec2::new(0.0, 1.0));
    if selected {
        ui.painter().rect_filled(highlight, CornerRadius::same(theme::RADIUS), color::row_selected());
    } else if response.hovered() {
        ui.painter().rect_filled(highlight, CornerRadius::same(theme::RADIUS), color::row_hover());
    }

    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(rect.left() + space::SM + theme::ICON_SIZE / 2.0, rect.center().y),
        Vec2::splat(theme::ICON_SIZE),
    );
    let texture = textures.icon(summon.face_file_name.as_deref());
    widgets::paint_game_icon(ui, texture, icon_rect, egui::Color32::WHITE);

    // Two lines of text, centred as a block on the row's midline.
    let text_left = icon_rect.right() + space::MD;
    let painter = ui.painter().with_clip_rect(rect.shrink2(Vec2::new(space::SM, 0.0)));
    painter.text(
        egui::pos2(text_left, rect.center().y - 9.0),
        Align2::LEFT_CENTER,
        &summon.summon_name,
        theme::bold(theme::BODY_SIZE),
        color::text(),
    );
    let mut detail = summon.summon_id.to_string();
    if summon.is_riding_only {
        detail.push_str(&format!("   {}", icons::MOUNT));
    }
    painter.text(
        egui::pos2(text_left, rect.center().y + 9.0),
        Align2::LEFT_CENTER,
        detail,
        theme::caption(),
        color::text_muted(),
    );

    response.on_hover_ui(|ui| {
        ui.label(RichText::new(&summon.summon_name).font(theme::bold(theme::BODY_SIZE)));
        ui.label(widgets::caption(format!(
            "ID {}  ·  Form {}  ·  Evolves into {}",
            summon.summon_id,
            summon.form,
            if summon.evolve_target == 0 { "—".to_owned() } else { summon.evolve_target.to_string() }
        )));
        ui.label(widgets::caption(format!(
            "{}  {} ({})",
            icons::CARD,
            summon.card_name.as_deref().unwrap_or("Unknown card"),
            summon.card_id
        )));
    })
}
