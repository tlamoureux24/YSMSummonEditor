//! Picks a skill by name or ID, to add a tree row or change a row's skill.

use eframe::egui;

use super::DialogContext;
use super::skill_search::{SearchInteraction, SkillSearch};
use crate::action::{Action, PickerPurpose};
use crate::icons;
use crate::theme::color;
use crate::widgets::{self, modal};
use crate::worker::Event;
use ysm_core::i18n::tr;

const LIST_HEIGHT: f32 = 432.0;

pub struct SkillPickerDialog {
    purpose: PickerPurpose,
    search: SkillSearch,
}

impl SkillPickerDialog {
    pub fn new(purpose: PickerPurpose) -> Self {
        Self { purpose, search: SkillSearch::new() }
    }

    pub fn on_event(&mut self, event: &Event) {
        self.search.on_event(event);
    }

    pub fn show(&mut self, ctx: &egui::Context, id: egui::Id, env: &mut DialogContext<'_>) -> bool {
        let title = match self.purpose {
            PickerPurpose::AddRow(_) => tr("Add Skill"),
            PickerPurpose::ChangeSkill(..) => tr("Change Skill"),
        };
        let mut picked = None;
        let mut cancelled = false;

        let outcome = modal::show(ctx, id, icons::ADD_SKILL, color::accent_bright(), title, 520.0, |ui| {
            widgets::control_row(
                ui,
                |ui| {
                    widgets::section_title(ui, tr("All skills"));
                },
                |ui| self.search.status(ui),
            );
            if let SearchInteraction::Activated(skill) = self.search.show(ui, env, LIST_HEIGHT, None) {
                picked = Some(skill);
            }

            modal::footer(ui, |ui| {
                if widgets::button(ui, icons::CLOSE, tr("Cancel")).clicked() {
                    cancelled = true;
                }
                let selected = self.search.selected_skill().cloned();
                if widgets::primary_button_enabled(ui, selected.is_some(), icons::CHECK, tr("Select")).clicked() {
                    picked = selected;
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.label(widgets::caption(tr("Double-click a skill or press Enter to pick it.")));
                });
            });
        });

        if let Some(skill) = picked {
            env.actions.push(Action::SkillPicked { purpose: self.purpose, skill });
            return false;
        }
        !(cancelled || outcome.dismissed)
    }
}
