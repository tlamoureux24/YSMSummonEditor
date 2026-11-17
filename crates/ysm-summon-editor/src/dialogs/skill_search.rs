//! Debounced skill search shared by the skill picker and the random group editor.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, CornerRadius, Key, Sense, Ui, Vec2};
use ysm_core::model::SkillDisplay;

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
    /// Double-click, or Enter in the search box.
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
    pub selected: Option<i32>,
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
    /// `is_disabled` greys out skills that cannot be picked.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        env: &mut DialogContext<'_>,
        list_height: f32,
        is_disabled: impl Fn(i32) -> bool,
    ) -> SearchInteraction {
        let mut interaction = SearchInteraction::None;

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
            let candidate = self.selected_skill().or_else(|| self.results.first()).cloned();
            match candidate {
                Some(skill) if self.edited_at.is_none() && !is_disabled(skill.skill_id) => {
                    interaction = SearchInteraction::Activated(skill);
                }
                _ => self.request(env.actions),
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
            for skill in &self.results {
                let disabled = is_disabled(skill.skill_id);
                let selected = self.selected == Some(skill.skill_id);
                let response = skill_row(ui, env, skill, selected, disabled, None);
                if response.clicked() {
                    self.selected = Some(skill.skill_id);
                }
                if response.double_clicked() && !disabled {
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

/// One skill in a list: icon, name, ID and an optional leading slot number.
pub fn skill_row(
    ui: &mut Ui,
    env: &mut DialogContext<'_>,
    skill: &SkillDisplay,
    selected: bool,
    disabled: bool,
    slot: Option<usize>,
) -> egui::Response {
    let size = Vec2::new(ui.available_width(), theme::LIST_ROW_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if !ui.is_rect_visible(rect) {
        return response;
    }

    let highlight = rect.shrink2(Vec2::new(0.0, 1.0));
    if selected {
        ui.painter().rect_filled(highlight, CornerRadius::same(theme::RADIUS), color::row_selected());
    } else if response.hovered() {
        ui.painter().rect_filled(highlight, CornerRadius::same(theme::RADIUS), color::row_hover());
    }

    let mut left = rect.left() + space::SM;
    if let Some(slot) = slot {
        // Slot numbers are right-aligned in a fixed column, so the icons line up.
        let column = 20.0;
        ui.painter().text(
            egui::pos2(left + column, rect.center().y),
            Align2::RIGHT_CENTER,
            slot.to_string(),
            theme::caption(),
            color::text_muted(),
        );
        left += column + space::SM;
    }

    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(left + theme::ICON_SIZE / 2.0, rect.center().y),
        Vec2::splat(theme::ICON_SIZE),
    );
    let texture = env.textures.icon(skill.icon_file_name.as_deref());
    let tint = if disabled { Color32::from_gray(110) } else { Color32::WHITE };
    widgets::paint_game_icon(ui, texture, icon_rect, tint);

    let text_left = icon_rect.right() + space::MD;
    let painter = ui.painter().with_clip_rect(rect.shrink2(Vec2::new(space::SM, 0.0)));
    painter.text(
        egui::pos2(text_left, rect.center().y - 9.0),
        Align2::LEFT_CENTER,
        &skill.skill_name,
        theme::body(),
        if disabled { color::text_disabled() } else { color::text() },
    );
    let detail =
        if disabled { trf("{}  ·  already in the group", &[&skill.skill_id]) } else { skill.skill_id.to_string() };
    painter.text(
        egui::pos2(text_left, rect.center().y + 9.0),
        Align2::LEFT_CENTER,
        detail,
        theme::caption(),
        if disabled { color::text_disabled() } else { color::text_muted() },
    );

    response.on_hover_ui(|ui| game_tooltip::skill(ui, skill.skill_id, Some(skill)))
}
