//! Editor for the candidate skills of a random group.

use std::collections::HashMap;

use eframe::egui::{self, Ui};
use ysm_core::model::SkillDisplay;
use ysm_core::random_group::{MAX_GROUP_SKILLS, RandomSkillGroup};
use ysm_core::session::{EditSession, GroupState, TreeRef};

use super::DialogContext;
use super::skill_search::{SearchInteraction, SkillSearch, skill_list, skill_row_with_action};
use crate::action::Action;
use crate::icons;
use crate::theme::{color, space};
use crate::widgets::{self, modal};
use crate::worker::{Command, Event};

const LIST_HEIGHT: f32 = 384.0;

pub struct RandomGroupDialog {
    tree: TreeRef,
    uid: u64,
    group_id: i32,
    is_new: bool,
    original: Vec<i32>,
    /// Candidate skill IDs, kept sorted and distinct.
    members: Vec<i32>,
    /// Skills added from search keep their names before the session loads them.
    known: HashMap<i32, SkillDisplay>,
    search: SkillSearch,
    load_error: Option<String>,
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
            known: HashMap::new(),
            search: SkillSearch::new(),
            load_error: None,
        }
    }

    pub fn on_event(&mut self, event: &Event) {
        if let Event::RandomGroupsLoaded { requested, result: Err(error) } = event
            && requested.contains(&self.group_id)
        {
            self.load_error = Some(error.message.clone());
        }
        self.search.on_event(event);
    }

    fn display(&self, session: &EditSession, skill_id: i32) -> SkillDisplay {
        self.known.get(&skill_id).or_else(|| session.skill_display(skill_id)).cloned().unwrap_or_else(|| SkillDisplay {
            skill_id,
            skill_name: format!("Skill {skill_id}"),
            icon_file_name: None,
            tooltip_markup: None,
        })
    }

    fn is_dirty(&self) -> bool {
        self.is_new || self.members != self.original
    }

    fn add(&mut self, skill: SkillDisplay) {
        if self.members.len() < MAX_GROUP_SKILLS
            && let Err(index) = self.members.binary_search(&skill.skill_id)
        {
            self.members.insert(index, skill.skill_id);
            self.known.insert(skill.skill_id, skill);
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, id: egui::Id, env: &mut DialogContext<'_>) -> bool {
        let group_state = env.session.group_state(self.group_id);
        let loading = !self.is_new && group_state == GroupState::Loading;
        if !self.is_dirty()
            && let Some(group) = env.session.overlay_group(self.group_id)
            && self.original != group.skill_ids()
        {
            self.original = group.skill_ids().to_vec();
            self.members.clone_from(&self.original);
        }
        let title = if self.is_new {
            format!("New Random Group #{}", self.group_id)
        } else {
            format!("Random Group #{}", self.group_id)
        };
        let mut apply = false;
        let mut cancelled = false;

        let outcome = modal::show(ctx, id, icons::RANDOM_GROUP, color::ACCENT_BRIGHT, &title, 848.0, |ui| {
            if loading {
                if let Some(error) = &self.load_error {
                    ui.label(widgets::muted("Could not load group candidates.").color(color::DANGER))
                        .on_hover_text(error);
                } else {
                    ui.label(widgets::muted("Loading group candidates…"));
                }
                if widgets::button(ui, icons::REFRESH, "Reload Group").clicked() {
                    self.load_error = None;
                    env.actions.push(Action::Command(Command::LoadRandomGroups { ids: vec![self.group_id] }));
                }
            } else {
                self.editor(ui, env, group_state);
            }

            modal::footer(ui, |ui| {
                if widgets::button(ui, icons::CLOSE, "Cancel").clicked() {
                    cancelled = true;
                }
                if !loading
                    && widgets::primary_button_enabled(ui, self.is_dirty(), icons::CHECK, "Apply Group").clicked()
                {
                    apply = true;
                }
            });
        });

        if apply {
            env.actions.push(Action::ApplyRandomGroup {
                attach_to: self.is_new.then_some((self.tree, self.uid)),
                group: RandomSkillGroup::new(self.group_id, self.members.iter().copied()),
            });
            return false;
        }
        // A click outside must not throw away an edited group; only Escape and Cancel close it.
        let escaped = outcome.dismissed && ctx.input(|input| input.key_pressed(egui::Key::Escape));
        !(cancelled || escaped)
    }

    fn editor(&mut self, ui: &mut Ui, env: &mut DialogContext<'_>, group_state: GroupState) {
        if !self.is_new && group_state == GroupState::MissingInDb {
            modal::note(
                ui,
                icons::WARNING,
                "This group is missing from the database. Applying changes stages a new group.",
                color::WARNING,
            );
        } else {
            ui.label(widgets::muted("A tree row linked to this group grants one of these skills at random."));
        }
        ui.add_space(space::SM);
        ui.columns_const(|[candidates, search]| {
            self.candidates_column(candidates, env);
            widgets::control_row(
                search,
                |ui| {
                    widgets::section_title(ui, "Find Skills");
                },
                |_| {},
            );
            if let SearchInteraction::Activated(skill) = self.search.show(search, env, LIST_HEIGHT, Some(&self.members))
            {
                self.add(skill);
            }
        });
    }

    fn candidates_column(&mut self, ui: &mut Ui, env: &mut DialogContext<'_>) {
        let full = self.members.len() >= MAX_GROUP_SKILLS;
        let count = if full {
            format!("Group full · {MAX_GROUP_SKILLS}/{MAX_GROUP_SKILLS}")
        } else {
            format!("{} of {MAX_GROUP_SKILLS} slots used", self.members.len())
        };
        widgets::control_row(
            ui,
            |ui| {
                widgets::section_title(ui, "Candidates");
            },
            |_| {},
        );
        widgets::control_row(
            ui,
            |ui| {
                let tint = if full { color::WARNING } else { color::TEXT_MUTED };
                ui.label(widgets::caption(count).color(tint));
            },
            |ui| {
                if widgets::button_enabled(ui, !self.members.is_empty(), icons::CLOSE, "Clear All").clicked() {
                    self.members.clear();
                }
            },
        );

        let mut removed = None;
        skill_list(ui, "group-members", LIST_HEIGHT, |ui| {
            if self.members.is_empty() {
                widgets::empty_state(
                    ui,
                    icons::RANDOM_GROUP,
                    "No candidates yet",
                    "Use Find Skills to add a candidate.",
                );
            }
            for skill_id in &self.members {
                let display = self.display(env.session, *skill_id);
                let (_, clicked) =
                    skill_row_with_action(ui, env, &display, None, |ui| widgets::button(ui, icons::CLOSE, "Remove"));
                if clicked {
                    removed = Some(*skill_id);
                }
            }
        });

        if let Some(skill_id) = removed {
            self.members.retain(|id| *id != skill_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;
    use crate::textures::TextureStore;

    struct Fixture {
        dialog: RandomGroupDialog,
        session: EditSession,
        textures: Option<TextureStore>,
        actions: Vec<Action>,
    }

    const TREE: TreeRef = TreeRef { stage: 3, slot: 1 };

    fn harness(session: EditSession, group: &RandomSkillGroup) -> Harness<'static, Fixture> {
        Harness::builder().with_size([1100.0, 700.0]).build_ui_state(
            |ui, fixture: &mut Fixture| {
                let ctx = ui.ctx();
                let Some(textures) = fixture.textures.as_mut() else {
                    crate::theme::apply(ctx);
                    fixture.textures = Some(TextureStore::new(ctx));
                    return;
                };
                let mut env = DialogContext { session: &fixture.session, textures, actions: &mut fixture.actions };
                fixture.dialog.show(ctx, egui::Id::new("random-group-test"), &mut env);
            },
            Fixture {
                dialog: RandomGroupDialog::new(TREE, 7, group, false),
                session,
                textures: None,
                actions: Vec::new(),
            },
        )
    }

    #[test]
    fn candidate_buttons_stage_the_updated_group() {
        let group = RandomSkillGroup::new(42, [100]);
        let mut session = EditSession::new();
        session.ingest_random_groups(&[42], [group.clone()]);
        let mut harness = harness(session, &group);
        harness.run();
        let generation = harness
            .state()
            .actions
            .iter()
            .find_map(|action| match action {
                Action::Command(Command::SearchSkills { generation, .. }) => Some(*generation),
                _ => None,
            })
            .unwrap();
        harness.state_mut().dialog.on_event(&Event::SkillsFound {
            generation,
            result: Ok(vec![SkillDisplay {
                skill_id: 200,
                skill_name: "New candidate".into(),
                icon_file_name: None,
                tooltip_markup: None,
            }]),
        });
        harness.run();

        harness.get_by_label_contains("Add").click();
        harness.run();
        assert_eq!(harness.state().dialog.members, [100, 200]);

        harness.get_all_by_label_contains("Remove").next().unwrap().click();
        harness.run();
        harness.get_by_label_contains("Apply Group").click();
        harness.run();

        assert!(harness.state().actions.iter().any(|action| matches!(
            action,
            Action::ApplyRandomGroup { attach_to: None, group }
                if group.group_id == 42 && group.skill_ids() == [200]
        )));
    }

    #[test]
    fn opening_an_unloaded_group_waits_for_its_candidates() {
        let mut harness = harness(EditSession::new(), &RandomSkillGroup::empty(42));
        harness.run();
        assert!(harness.state().actions.is_empty(), "search must wait until the group is loaded");

        let group = RandomSkillGroup::new(42, [100]);
        harness.state_mut().session.ingest_random_groups(&[42], [group]);
        harness.run();

        assert_eq!(harness.state().dialog.members, [100]);
        assert!(!harness.state().dialog.is_dirty());
    }
}
