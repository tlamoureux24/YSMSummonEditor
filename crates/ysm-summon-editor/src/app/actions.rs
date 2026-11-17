//! Reduces [`Action`]s pushed by views and dialogs.

use eframe::egui;
use ysm_core::model::EntryField;
use ysm_core::random_group::RandomSkillGroup;
use ysm_core::session::TreeRef;

use super::EditorApp;
use super::save_flow::SaveOrigin;
use crate::action::{Action, PickerPurpose};
use crate::dialogs::message::{ButtonStyle, MessageKind};
use crate::dialogs::random_group::RandomGroupDialog;
use crate::dialogs::settings::SettingsDialog;
use crate::dialogs::skill_picker::SkillPickerDialog;
use crate::dialogs::{Dialog, MessageDialog};
use crate::icons;
use crate::worker::Command;
use ysm_core::i18n::{tr, trf};

impl EditorApp {
    pub(super) fn handle_action(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::Command(command) => self.worker.send(command),
            Action::Toast(kind, text) => self.toasts.push(kind, text),
            Action::CopyText(text) => ctx.copy_text(text),

            // ---- summon list ----
            Action::Search => self.search_summons(),
            Action::FocusSearch => self.state.focus_search = true,
            Action::SelectSummon(summon) => {
                if self.state.session.has_changes() {
                    let dialog = MessageDialog::new(
                        MessageKind::Question,
                        tr("Unsaved changes"),
                        trf(
                            "The current summon has staged changes. What should happen before opening {}?",
                            &[&summon.summon_name],
                        ),
                    )
                    .button(icons::SAVE, tr("Save"), ButtonStyle::Primary, Some(Action::SaveAndSwitch(summon.clone())))
                    .button(icons::DISCARD, tr("Discard"), ButtonStyle::Danger, Some(Action::SwitchSummon(summon)))
                    .button(icons::CLOSE, tr("Cancel"), ButtonStyle::Plain, None);
                    self.push_dialog(Dialog::message(dialog));
                } else {
                    self.load_chain(summon);
                }
            }
            Action::SwitchSummon(summon) => self.load_chain(summon),
            Action::SaveAndSwitch(summon) => self.start_save(SaveOrigin::SummonSwitch(summon)),
            Action::Reload => {
                let Some(summon) = self.current_summon.clone() else {
                    self.search_summons();
                    return;
                };
                if self.state.session.has_changes() {
                    let dialog = MessageDialog::new(
                        MessageKind::Warning,
                        tr("Reload from database"),
                        tr("Reloading discards every staged change of this summon."),
                    )
                    .button(
                        icons::REFRESH,
                        tr("Discard and reload"),
                        ButtonStyle::Danger,
                        Some(Action::SwitchSummon(summon)),
                    )
                    .button(icons::CLOSE, tr("Cancel"), ButtonStyle::Plain, None);
                    self.push_dialog(Dialog::message(dialog));
                } else {
                    self.load_chain(summon);
                }
            }

            // ---- toolbar ----
            Action::Save => self.start_save(SaveOrigin::Toolbar),
            Action::Undo => {
                if self.state.session.undo() {
                    self.set_status(tr("Undone."));
                }
            }
            Action::Redo => {
                if self.state.session.redo() {
                    self.set_status(tr("Redone."));
                }
            }
            Action::AskDiscardAll => {
                let count = self.state.session.change_count();
                let dialog = MessageDialog::new(
                    MessageKind::Warning,
                    tr("Discard staged changes"),
                    trf("Discard all {} staged change(s)? Trees, skills, stats and random groups return to their database state.", &[&count]),
                )
                .button(icons::DISCARD, tr("Discard all"), ButtonStyle::Danger, Some(Action::DiscardAll))
                .button(icons::CLOSE, tr("Cancel"), ButtonStyle::Plain, None);
                self.push_dialog(Dialog::message(dialog));
            }
            Action::DiscardAll => {
                self.state.session.discard_all();
                self.set_status(tr("Staged changes discarded."));
            }
            Action::OpenStats => {
                if let Some(chain) = self.state.session.chain() {
                    self.worker.send(Command::LoadScaling { summon_id: chain.stage3.summon_id });
                    self.set_status(tr("Loading stat scaling…"));
                }
            }
            Action::Reconnect => {
                self.set_status(tr("Reconnecting…"));
                self.connect();
            }
            Action::OpenSettings => {
                let dialog = SettingsDialog::new(&self.state.settings, self.state.demo);
                self.push_dialog(Dialog::Settings(Box::new(dialog)));
            }
            Action::SetSaveToSql(to_sql) => {
                self.state.settings.save_to_sql_mode = to_sql;
                self.save_settings();
                self.set_status(if to_sql {
                    tr("Save mode set to .sql (staged export).")
                } else {
                    tr("Save mode set to DB.")
                });
            }

            // ---- tree panel ----
            Action::SetStage(stage) => {
                self.state.active_stage = stage;
                self.state.selected_row = None;
                self.state.selected_effective = None;
            }
            Action::SetSlot(slot) => {
                self.state.active_slot = slot;
                self.state.selected_row = None;
            }
            Action::SelectRow(uid) => self.state.selected_row = uid,
            Action::SelectEffective(key) => self.state.selected_effective = key,
            Action::AddRow(tree) => {
                if let Some(uid) = self.state.session.add_row(tree, 0) {
                    self.state.selected_row = Some(uid);
                    self.set_status(tr("Row added. Enter a skill ID or use Change Skill."));
                }
            }
            Action::OpenSkillPicker(purpose) => {
                self.push_dialog(Dialog::SkillPicker(Box::new(SkillPickerDialog::new(purpose))));
            }
            Action::SkillPicked { purpose, skill } => {
                let (skill_id, name) = (skill.skill_id, skill.skill_name.clone());
                self.state.session.ingest_skill_display([skill]);
                match purpose {
                    PickerPurpose::AddRow(tree) => {
                        if let Some(uid) = self.state.session.add_row(tree, skill_id) {
                            self.state.selected_row = Some(uid);
                            self.set_status(trf("Added {} ({}).", &[&name, &skill_id]));
                        }
                    }
                    PickerPurpose::ChangeSkill(tree, uid) => {
                        if self.state.session.set_field(tree, uid, EntryField::SkillId, skill_id) {
                            self.set_status(trf("Skill changed to {} ({}).", &[&name, &skill_id]));
                        }
                    }
                }
            }
            Action::DuplicateRow(tree, uid) => {
                if let Some(copy) = self.state.session.duplicate_row(tree, uid) {
                    self.state.selected_row = Some(copy);
                    self.set_status(tr("Row duplicated."));
                }
            }
            Action::AskDeleteRow(tree, uid) => self.ask_delete_row(tree, uid),
            Action::DeleteRow(tree, uid) => {
                if self.state.session.delete_row(tree, uid) {
                    self.set_status(tr("Row deleted."));
                }
            }
            Action::SetField { tree, uid, field, value } => {
                if self.state.session.set_field(tree, uid, field, value) {
                    self.set_status(trf("{} set to {}.", &[&field.column(), &value]));
                }
            }
            Action::SetEffectiveLevels { stage, skill_id, lv, min, max } => {
                if self.state.session.set_effective_levels(stage, skill_id, lv, min, max) {
                    self.set_status(tr("Matrix change staged."));
                }
            }
            Action::AskDeleteEffective { stage, skill_id, lv } => {
                let linked = self
                    .state
                    .session
                    .effective_rows(stage)
                    .iter()
                    .find(|row| (row.skill_id, row.lv) == (skill_id, lv))
                    .map_or(0, |row| row.linked);
                if linked == 0 {
                    return;
                }
                let dialog = MessageDialog::new(
                    MessageKind::Warning,
                    tr("Confirm delete"),
                    trf(
                        "Delete {} ({}, Lv {}) from Evolution {}? This removes {} row(s).",
                        &[&self.state.session.skill_name(skill_id), &skill_id, &lv, &stage, &linked],
                    ),
                )
                .button(
                    icons::DELETE,
                    tr("Delete"),
                    ButtonStyle::Danger,
                    Some(Action::DeleteEffective { stage, skill_id, lv }),
                )
                .button(icons::CLOSE, tr("Cancel"), ButtonStyle::Plain, None);
                self.push_dialog(Dialog::message(dialog));
            }
            Action::DeleteEffective { stage, skill_id, lv } => {
                let removed = self.state.session.delete_effective(stage, skill_id, lv);
                self.set_status(trf("Deleted {} row(s).", &[&removed]));
            }

            // ---- skills, groups, stats ----
            Action::EditSkill(skill_id) => {
                self.worker.send(Command::LoadSkillRecord { skill_id });
                self.set_status(trf("Loading skill {}…", &[&skill_id]));
            }
            Action::StageSkillUpdate(update) => {
                let (skill_id, count) = (update.skill_id, update.changes.len());
                self.state.session.stage_skill_update(update);
                self.set_status(if count == 0 {
                    trf("Skill {} has no staged changes.", &[&skill_id])
                } else {
                    trf("Skill {} staged for save ({} column(s)).", &[&skill_id, &count])
                });
            }
            Action::CreateGroup(tree, uid) => self.worker.send(Command::NextRandomGroupId { tree, uid }),
            Action::EditGroup(tree, uid) => {
                let session = &self.state.session;
                let Some(group_id) = session
                    .tree(tree)
                    .and_then(|t| t.row(uid))
                    .map(|row| row.entry.skill_group_id)
                    .filter(|id| *id > 0)
                else {
                    return;
                };
                let group =
                    session.overlay_group(group_id).cloned().unwrap_or_else(|| RandomSkillGroup::empty(group_id));
                let dialog = RandomGroupDialog::new(tree, uid, &group, false);
                self.push_dialog(Dialog::RandomGroup(Box::new(dialog)));
            }
            Action::AskDetachGroup(tree, uid) => {
                let Some(group_id) =
                    self.state.session.tree(tree).and_then(|t| t.row(uid)).map(|row| row.entry.skill_group_id)
                else {
                    return;
                };
                let dialog = MessageDialog::new(
                    MessageKind::Question,
                    tr("Detach random group"),
                    trf("Detach random group {} from this tree row? The group itself is not deleted.", &[&group_id]),
                )
                .button(icons::DETACH, tr("Detach"), ButtonStyle::Primary, Some(Action::DetachGroup(tree, uid)))
                .button(icons::CLOSE, tr("Cancel"), ButtonStyle::Plain, None);
                self.push_dialog(Dialog::message(dialog));
            }
            Action::DetachGroup(tree, uid) => {
                if self.state.session.detach_group(tree, uid) {
                    self.set_status(tr("Random group detached from the selected row."));
                }
            }
            Action::ApplyRandomGroup { tree, uid, group } => {
                let (group_id, use_code) = (group.group_id, group.use_code());
                if self.state.session.attach_group(tree, uid, group) {
                    self.set_status(trf("Random group {} staged (UseCode: {}).", &[&group_id, &use_code]));
                }
            }
            Action::ApplyScaling(result) => {
                self.state.session.apply_scaling_edit(result);
                self.set_status(tr("Stat scaling changes staged."));
            }

            // ---- settings ----
            Action::ApplySettings(settings) => {
                let save_to_sql_mode = self.state.settings.save_to_sql_mode;
                self.state.settings = *settings;
                self.state.settings.save_to_sql_mode = save_to_sql_mode;
                self.save_settings();
                self.set_status(tr("Settings saved."));
                if !self.state.demo {
                    self.connect();
                }
                self.rebuild_assets();
            }

            // ---- save flow ----
            Action::SaveAnswer(answer) => self.on_save_answer(answer),
            Action::SaveCancelled => self.on_save_cancelled(),
            Action::ReviewConfirmed => self.on_review_confirmed(),
            Action::CommitConfirmed => self.on_commit_confirmed(),

            // ---- closing the window ----
            Action::AskExit => {
                self.exit_prompt_open = true;
                let dialog = MessageDialog::new(
                    MessageKind::Warning,
                    tr("Unsaved changes"),
                    trf("{} staged change(s) have not been saved. Exit anyway?", &[&self.state.session.change_count()]),
                )
                .button(icons::DISCARD, tr("Discard and exit"), ButtonStyle::Danger, Some(Action::Exit))
                .button(icons::CLOSE, tr("Stay"), ButtonStyle::Plain, Some(Action::CancelExit))
                .on_dismiss(Action::CancelExit);
                self.push_dialog(Dialog::message(dialog));
            }
            Action::CancelExit => self.exit_prompt_open = false,
            Action::Exit => {
                self.exit_confirmed = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn ask_delete_row(&mut self, tree: TreeRef, uid: u64) {
        let session = &self.state.session;
        let Some(row) = session.tree(tree).and_then(|t| t.row(uid)) else {
            return;
        };
        let skill_id = row.entry.skill_id;
        // A freshly added empty row is not worth a confirmation.
        if row.is_new() && skill_id <= 0 {
            self.state.session.delete_row(tree, uid);
            return;
        }
        let dialog = MessageDialog::new(
            MessageKind::Warning,
            tr("Confirm delete"),
            trf(
                "Delete {} ({}) from Evolution {}, Tree {}?",
                &[&session.skill_name(skill_id), &skill_id, &tree.stage, &tree.slot],
            ),
        )
        .button(icons::DELETE, tr("Delete"), ButtonStyle::Danger, Some(Action::DeleteRow(tree, uid)))
        .button(icons::CLOSE, tr("Cancel"), ButtonStyle::Plain, None);
        self.push_dialog(Dialog::message(dialog));
    }
}
