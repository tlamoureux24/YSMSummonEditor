//! Editor for the candidate skills of a random group.
//!
//! Two lists of equal size sit either side of a narrow column of buttons:
//! every skill on the left, the group's candidates on the right. Both
//! columns have the same rows above their list, so the lists line up.

use std::collections::HashMap;

use eframe::egui::{self, RichText, Ui, Vec2};
use ysm_core::model::SkillDisplay;
use ysm_core::random_group::{MAX_GROUP_SKILLS, RandomSkillGroup};
use ysm_core::session::{EditSession, TreeRef};

use super::DialogContext;
use super::skill_search::{SearchInteraction, SkillSearch, skill_list, skill_row};
use crate::action::Action;
use crate::icons;
use crate::theme::{self, color, space};
use crate::widgets::{self, modal};
use crate::worker::Event;
use ysm_core::i18n::{tr, trf};

const LIST_HEIGHT: f32 = 384.0;
/// Width of the column holding the Add / Remove / Clear buttons.
const BUTTON_COLUMN_WIDTH: f32 = 112.0;

pub struct RandomGroupDialog {
    tree: TreeRef,
    uid: u64,
    group_id: i32,
    is_new: bool,
    original: Vec<i32>,
    /// Candidate skill IDs, kept sorted and distinct.
    members: Vec<i32>,
    selected_member: Option<i32>,
    /// Display data of skills seen in search results, so members keep their names.
    known: HashMap<i32, SkillDisplay>,
    search: SkillSearch,
    status: String,
}

impl RandomGroupDialog {
    /// `group` is the version to start from; `is_new` marks a group that does not exist yet.
    pub fn new(tree: TreeRef, uid: u64, group: &RandomSkillGroup, is_new: bool) -> Self {
        Self {
            tree,
            uid,
            group_id: group.group_id,
            is_new,
            original: group.skill_ids().to_vec(),
            members: group.skill_ids().to_vec(),
            selected_member: None,
            known: HashMap::new(),
            search: SkillSearch::new(),
            status: String::new(),
        }
    }

    pub fn on_event(&mut self, event: &Event) {
        if let Event::SkillsFound { result: Ok(found), .. } = event {
            self.known.extend(found.iter().map(|skill| (skill.skill_id, skill.clone())));
        }
        self.search.on_event(event);
    }

    fn display(&self, session: &EditSession, skill_id: i32) -> SkillDisplay {
        self.known.get(&skill_id).or_else(|| session.skill_display(skill_id)).cloned().unwrap_or_else(|| SkillDisplay {
            skill_id,
            skill_name: trf("Skill {}", &[&skill_id]),
            icon_file_name: None,
            tooltip_markup: None,
        })
    }

    fn is_dirty(&self) -> bool {
        self.is_new || self.members != self.original
    }

    fn add(&mut self, skill: &SkillDisplay) {
        if self.members.contains(&skill.skill_id) {
            self.status = trf("Skill {} is already in the group.", &[&skill.skill_id]);
        } else if self.members.len() >= MAX_GROUP_SKILLS {
            self.status = trf("A random group can hold at most {} skills.", &[&MAX_GROUP_SKILLS]);
        } else {
            self.known.insert(skill.skill_id, skill.clone());
            self.members.push(skill.skill_id);
            self.members.sort_unstable();
            self.status = trf("Added skill {}.", &[&skill.skill_id]);
        }
    }

    fn remove(&mut self, skill_id: i32) {
        self.members.retain(|id| *id != skill_id);
        if self.selected_member == Some(skill_id) {
            self.selected_member = None;
        }
        self.status = trf("Removed skill {}.", &[&skill_id]);
    }

    pub fn show(&mut self, ctx: &egui::Context, id: egui::Id, env: &mut DialogContext<'_>) -> bool {
        let title = if self.is_new {
            trf("New Random Group · RND[{}]", &[&self.group_id])
        } else {
            trf("Random Group · RND[{}]", &[&self.group_id])
        };
        let mut apply = false;
        let mut cancelled = false;

        let outcome = modal::show(ctx, id, icons::RANDOM_GROUP, color::accent_bright(), &title, 912.0, |ui| {
            ui.label(widgets::muted(
                "A tree row with this group grants one random skill out of the candidates. \
                 Candidates are kept sorted by skill ID, and UseCode always equals their number.",
            ));
            ui.add_space(space::SM);

            let gap = space::LG;
            let list_width = ((ui.available_width() - BUTTON_COLUMN_WIDTH - 2.0 * gap) / 2.0).floor();
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                ui.vertical(|ui| {
                    ui.set_width(list_width);
                    self.all_skills_column(ui, env);
                });
                ui.allocate_ui_with_layout(
                    Vec2::new(BUTTON_COLUMN_WIDTH, 0.0),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        ui.set_width(BUTTON_COLUMN_WIDTH);
                        self.button_column(ui);
                    },
                );
                ui.vertical(|ui| {
                    ui.set_width(list_width);
                    self.candidates_column(ui, env);
                });
            });

            modal::footer(ui, |ui| {
                if widgets::button(ui, icons::CLOSE, tr("Cancel")).clicked() {
                    cancelled = true;
                }
                if widgets::primary_button_enabled(ui, self.is_dirty(), icons::CHECK, tr("Apply")).clicked() {
                    apply = true;
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.label(widgets::caption(self.status.as_str()));
                });
            });
        });

        if apply {
            env.actions.push(Action::ApplyRandomGroup {
                tree: self.tree,
                uid: self.uid,
                group: RandomSkillGroup::new(self.group_id, self.members.iter().copied()),
            });
            return false;
        }
        // A click outside must not throw away an edited group; only Escape and Cancel close it.
        let escaped = outcome.dismissed && ctx.input(|input| input.key_pressed(egui::Key::Escape));
        !(cancelled || escaped)
    }

    /// Left column: title row, search box, result list.
    fn all_skills_column(&mut self, ui: &mut Ui, env: &mut DialogContext<'_>) {
        widgets::control_row(
            ui,
            |ui| {
                widgets::section_title(ui, tr("All skills"));
            },
            |ui| self.search.status(ui),
        );
        let members = self.members.clone();
        let interaction = self.search.show(ui, env, LIST_HEIGHT, |skill_id| members.contains(&skill_id));
        if let SearchInteraction::Activated(skill) = interaction {
            self.add(&skill);
        }
    }

    /// Middle column: the three buttons, centred against the two lists.
    fn button_column(&mut self, ui: &mut Ui) {
        let buttons_height = 3.0 * theme::CONTROL_HEIGHT + 2.0 * space::SM;
        // Skip the title row and the search row, then centre within the list.
        ui.add_space(2.0 * (theme::CONTROL_HEIGHT + space::SM) + (LIST_HEIGHT - buttons_height) / 2.0);

        let size = Vec2::new(BUTTON_COLUMN_WIDTH, theme::CONTROL_HEIGHT);
        let candidate = self.search.selected_skill().cloned();
        let can_add = candidate.as_ref().is_some_and(|skill| !self.members.contains(&skill.skill_id))
            && self.members.len() < MAX_GROUP_SKILLS;

        let add = egui::Button::new(trf("Add  {}", &[&icons::ARROW_RIGHT])).min_size(size);
        if ui.add_enabled(can_add, add).clicked()
            && let Some(skill) = candidate
        {
            self.add(&skill);
        }
        let remove = egui::Button::new(widgets::icon_text(icons::DELETE, tr("Remove"))).min_size(size);
        if ui.add_enabled(self.selected_member.is_some(), remove).clicked()
            && let Some(skill_id) = self.selected_member
        {
            self.remove(skill_id);
        }
        let clear = egui::Button::new(widgets::icon_text(icons::CLOSE, tr("Clear"))).min_size(size);
        if ui.add_enabled(!self.members.is_empty(), clear).clicked() {
            self.members.clear();
            self.selected_member = None;
            self.status = tr("Selection cleared.").to_owned();
        }
    }

    /// Right column: title row, a hint where the left column has its search
    /// box, then the candidate list.
    fn candidates_column(&mut self, ui: &mut Ui, env: &mut DialogContext<'_>) {
        let full = self.members.len() >= MAX_GROUP_SKILLS;
        let count = format!("{} / {MAX_GROUP_SKILLS}", self.members.len());
        widgets::control_row(
            ui,
            |ui| {
                widgets::section_title(ui, tr("Group candidates"));
            },
            |ui| {
                let tint = if full { color::warning() } else { color::text_muted() };
                ui.label(RichText::new(count).size(theme::CAPTION_SIZE).color(tint));
            },
        );
        widgets::control_row(
            ui,
            |ui| {
                ui.label(widgets::caption(tr("Double-click a candidate to remove it.")));
            },
            |ui| {
                ui.label(widgets::caption(trf("UseCode {}", &[&self.members.len()])));
            },
        );

        let mut clicked = None;
        let mut removed = None;
        skill_list(ui, "group-members", LIST_HEIGHT, |ui| {
            if self.members.is_empty() {
                widgets::empty_state(
                    ui,
                    icons::RANDOM_GROUP,
                    tr("No candidates yet"),
                    tr("Double-click a skill on the left to add it."),
                );
            }
            for (index, skill_id) in self.members.iter().enumerate() {
                let display = self.display(env.session, *skill_id);
                let selected = self.selected_member == Some(*skill_id);
                let response = skill_row(ui, env, &display, selected, false, Some(index + 1));
                if response.clicked() {
                    clicked = Some(*skill_id);
                }
                if response.double_clicked() {
                    removed = Some(*skill_id);
                }
            }
        });

        if let Some(skill_id) = clicked {
            self.selected_member = Some(skill_id);
        }
        if let Some(skill_id) = removed {
            self.remove(skill_id);
        }
    }
}
