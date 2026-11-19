//! Debounced skill search shared by the skill picker and the random group editor.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Align2, Color32, CornerRadius, Key, Layout, Response, Sense, Ui, Vec2};
use ysm_core::model::SkillDisplay;
use ysm_core::random_group::MAX_GROUP_SKILLS;

use super::{DialogContext, next_generation};
use crate::action::Action;
use crate::icons;
use crate::theme::{self, color, space};
use crate::widgets::{self, game_tooltip};
use crate::worker::{Command, Event};
use ysm_core::i18n::{tr, trf};

const DEBOUNCE: Duration = Duration::from_millis(250);

/// What the user did with the result list this frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchInteraction {
    None,
    /// Add, double-click, or Enter in the search box.
    Activated(SkillDisplay),
}

pub struct SkillSearch {
    query: String,
    edited_at: Option<Instant>,
    /// Generation of the request whose answer is awaited.
    awaiting: Option<u64>,
    started: bool,
    results: Vec<SkillDisplay>,
    error: Option<String>,
    selected: Option<i32>,
}

impl SkillSearch {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            edited_at: None,
            awaiting: None,
            started: false,
            results: Vec::new(),
            error: None,
            selected: None,
        }
    }

    pub fn selected_skill(&self) -> Option<&SkillDisplay> {
        self.results.iter().find(|skill| Some(skill.skill_id) == self.selected)
    }

    pub fn on_event(&mut self, event: &Event) {
        let Event::SkillsFound { generation, result } = event else {
            return;
        };
        if self.awaiting != Some(*generation) {
            return;
        }
        self.awaiting = None;
        match result {
            Ok(found) => {
                self.results = found.clone();
                self.error = None;
                if self.selected_skill().is_none() {
                    self.selected = None;
                }
            }
            Err(error) => {
                self.results.clear();
                self.error = Some(error.message.clone());
            }
        }
    }

    fn request(&mut self, actions: &mut Vec<Action>) {
        let generation = next_generation();
        self.awaiting = Some(generation);
        self.edited_at = None;
        actions.push(Action::Command(Command::SearchSkills { text: self.query.trim().to_owned(), generation }));
    }

    /// The state of the search as a caption: a spinner, an error or the result count.
    pub fn status(&self, ui: &mut Ui) {
        if let Some(error) = &self.error {
            ui.label(widgets::caption(trf("{}  Search failed", &[&icons::ERROR])).color(color::danger()))
                .on_hover_text(error.as_str());
        } else if self.awaiting.is_some() {
            ui.label(widgets::caption(tr("Searching…")));
            ui.add(egui::Spinner::new().size(theme::CAPTION_SIZE));
        } else {
            ui.label(widgets::caption(trf("{} skills", &[&self.results.len()])));
        }
    }

    /// Draws the search box and, below it, the result list of `list_height`.
    /// In a random group, each result has an Add button and respects membership and capacity.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        env: &mut DialogContext<'_>,
        list_height: f32,
        group_members: Option<&[i32]>,
    ) -> SearchInteraction {
        let mut interaction = SearchInteraction::None;
        let add_blocked = |id| {
            let members = group_members?;
            if members.contains(&id) {
                Some(tr("Already in this group"))
            } else if members.len() >= MAX_GROUP_SKILLS {
                Some(tr("The group is full. Remove a candidate to make room."))
            } else {
                None
            }
        };

        let search = ui.add(
            widgets::text_field(&mut self.query)
                .hint_text(trf("{}  Skill ID or name", &[&icons::SEARCH]))
                .desired_width(f32::INFINITY),
        );
        if !self.started {
            self.started = true;
            search.request_focus();
            self.request(env.actions);
        }
        if search.changed() {
            self.edited_at = Some(Instant::now());
        }
        let enter = search.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
        if enter {
            // Enter confirms the highlighted (or only sensible) result.
            let candidate = self
                .selected_skill()
                .filter(|skill| add_blocked(skill.skill_id).is_none())
                .or_else(|| self.results.iter().find(|skill| add_blocked(skill.skill_id).is_none()))
                .cloned();
            match candidate {
                Some(skill) if self.edited_at.is_none() && (group_members.is_none() || self.awaiting.is_none()) => {
                    interaction = SearchInteraction::Activated(skill);
                }
                _ if group_members.is_none()
                    || self.edited_at.is_some()
                    || (self.awaiting.is_none() && self.results.is_empty()) =>
                {
                    self.request(env.actions);
                }
                _ => {}
            }
            search.request_focus();
        } else if let Some(edited_at) = self.edited_at {
            if edited_at.elapsed() >= DEBOUNCE {
                self.request(env.actions);
            } else {
                ui.ctx().request_repaint_after(DEBOUNCE - edited_at.elapsed());
            }
        }

        skill_list(ui, "skill-search-results", list_height, |ui| {
            if group_members.is_some() && (self.awaiting.is_some() || self.edited_at.is_some()) {
                widgets::empty_state(ui, icons::SEARCH, tr("Searching…"), "");
                return;
            }
            if group_members.is_some() && self.results.is_empty() {
                if let Some(error) = &self.error {
                    widgets::empty_state(
                        ui,
                        icons::ERROR,
                        tr("Search failed"),
                        tr("Press Enter in the search box to retry."),
                    );
                    ui.vertical_centered(|ui| {
                        ui.label(widgets::caption(tr("Error details"))).on_hover_text(error.as_str());
                    });
                } else {
                    widgets::empty_state(
                        ui,
                        icons::SEARCH,
                        tr("No matching skills"),
                        tr("Try another name or skill ID."),
                    );
                }
                return;
            }

            for skill in &self.results {
                let blocked = add_blocked(skill.skill_id);
                let selected = Some(self.selected == Some(skill.skill_id));
                let (response, added) = if let Some(members) = group_members {
                    let in_group = members.contains(&skill.skill_id);
                    skill_row_with_action(ui, env, skill, selected, |ui| {
                        let (icon, text) = if in_group { (icons::CHECK, tr("Added")) } else { (icons::ADD, tr("Add")) };
                        let response = widgets::button_enabled(ui, blocked.is_none(), icon, text);
                        if let Some(reason) = blocked { response.on_disabled_hover_text(reason) } else { response }
                    })
                } else {
                    (skill_row(ui, env, skill, selected), false)
                };
                if response.clicked() {
                    self.selected = Some(skill.skill_id);
                }
                if added || (response.double_clicked() && blocked.is_none()) {
                    interaction = SearchInteraction::Activated(skill.clone());
                }
            }
        });

        interaction
    }
}

/// A recessed, scrollable list of skills with a fixed height.
pub fn skill_list(ui: &mut Ui, id_salt: &str, height: f32, add_rows: impl FnOnce(&mut Ui)) {
    theme::well().show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::vertical()
            .id_salt(id_salt)
            .auto_shrink(false)
            .max_height(height)
            .min_scrolled_height(height)
            .show(ui, add_rows);
    });
}

/// A skill with an action on the right. The name takes the remaining width.
pub fn skill_row_with_action(
    ui: &mut Ui,
    env: &mut DialogContext<'_>,
    skill: &SkillDisplay,
    selected: Option<bool>,
    action: impl FnOnce(&mut Ui) -> Response,
) -> (Response, bool) {
    ui.push_id(skill.skill_id, |ui| {
        let size = Vec2::new(ui.available_width(), theme::LIST_ROW_HEIGHT);
        ui.allocate_ui_with_layout(size, Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(space::XS);
            let clicked = action(ui).clicked();
            let row = ui.with_layout(Layout::left_to_right(Align::Center), |ui| skill_row(ui, env, skill, selected));
            (row.inner, clicked)
        })
        .inner
    })
    .inner
}

/// One skill in a list. Rows with selection can be activated independently of their action.
fn skill_row(ui: &mut Ui, env: &mut DialogContext<'_>, skill: &SkillDisplay, selected: Option<bool>) -> Response {
    let size = Vec2::new(ui.available_width(), theme::LIST_ROW_HEIGHT);
    let sense = if selected.is_some() { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let name = game_tooltip::plain_skill_name(&skill.skill_name);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, name.as_ref()));

    let highlight = rect.shrink2(Vec2::new(0.0, 1.0));
    if selected == Some(true) {
        ui.painter().rect_filled(highlight, CornerRadius::same(theme::RADIUS), color::row_selected());
    } else if response.hovered() {
        ui.painter().rect_filled(highlight, CornerRadius::same(theme::RADIUS), color::row_hover());
    }

    let left = rect.left() + space::SM;
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(left + theme::ICON_SIZE / 2.0, rect.center().y),
        Vec2::splat(theme::ICON_SIZE),
    );
    let texture = env.textures.icon(skill.icon_file_name.as_deref());
    widgets::paint_game_icon(ui, texture, icon_rect, Color32::WHITE);

    let text_left = icon_rect.right() + space::MD;
    let painter = ui.painter().with_clip_rect(rect.shrink2(Vec2::new(space::SM, 0.0)));
    painter.text(egui::pos2(text_left, rect.center().y - 9.0), Align2::LEFT_CENTER, name, theme::body(), color::text());
    painter.text(
        egui::pos2(text_left, rect.center().y + 9.0),
        Align2::LEFT_CENTER,
        skill.skill_id.to_string(),
        theme::caption(),
        color::text_muted(),
    );

    response.on_hover_ui(|ui| game_tooltip::skill(ui, skill.skill_id, Some(skill)))
}
