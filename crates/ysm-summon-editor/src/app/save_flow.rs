//! The save pipeline:
//!
//! analyze usage → ask about shared data → review → (export .sql | confirm → commit).
//!
//! Each step ends by showing a dialog or sending a worker command; the
//! matching `on_*` method picks the flow up again. Cancelling at any point
//! leaves every staged change in place.

use chrono::Local;
use ysm_core::commit::{CommitRequest, CommitResult};
use ysm_core::model::SummonListItem;
use ysm_core::review::build_review_rows;
use ysm_core::save_plan::{SaveAnswer, SavePlanner, SaveQuestion, UsageReport};
use ysm_core::sql_script::render_script;

use super::EditorApp;
use crate::action::Action;
use crate::dialogs::message::{ButtonStyle, MessageKind};
use crate::dialogs::save_review::SaveReviewDialog;
use crate::dialogs::{Dialog, MessageDialog};
use crate::icons;
use crate::widgets::toast::ToastKind;
use crate::worker::{Command, FailureKind, Outcome};

/// What triggered the save, which decides what happens once it is done.
pub(super) enum SaveOrigin {
    Toolbar,
    /// The user chose "Save" when switching to another summon.
    SummonSwitch(SummonListItem),
}

pub(super) enum SaveFlow {
    Idle,
    Analyzing(SaveOrigin),
    Asking { origin: SaveOrigin, planner: Box<SavePlanner> },
    Reviewing { origin: SaveOrigin, request: CommitRequest },
    Confirming { origin: SaveOrigin, request: CommitRequest },
    Committing(SaveOrigin),
}

impl EditorApp {
    fn take_save_flow(&mut self) -> SaveFlow {
        std::mem::replace(&mut self.save, SaveFlow::Idle)
    }

    pub(super) fn start_save(&mut self, origin: SaveOrigin) {
        if !matches!(self.save, SaveFlow::Idle) {
            return;
        }

        let request = self.state.session.build_commit_request();
        if request.is_empty() {
            match origin {
                SaveOrigin::Toolbar => self.toasts.push(ToastKind::Info, "No changes to save."),
                SaveOrigin::SummonSwitch(summon) => self.load_chain(summon),
            }
            return;
        }
        if !self.state.connection.is_ready() {
            self.push_dialog(Dialog::message(MessageDialog::error(
                "Save",
                "The database is not connected, so shared data cannot be checked.",
                "Open Settings and fix the connection, then save again.",
            )));
            return;
        }

        let session_tree_ids = self.state.session.editable_tree_commits().iter().map(|t| t.original_tree_id).collect();
        self.worker.send(Command::AnalyzeSave { request, session_tree_ids });
        self.busy = Some("Checking where the edited data is used…");
        self.save = SaveFlow::Analyzing(origin);
    }

    pub(super) fn on_save_analyzed(&mut self, result: Outcome<UsageReport>) {
        self.busy = None;
        let SaveFlow::Analyzing(origin) = self.take_save_flow() else {
            return;
        };
        match result {
            Ok(usage) => {
                let session = &self.state.session;
                let chain_ids = session.chain().map(|chain| chain.summon_ids()).unwrap_or_default();
                let planner = SavePlanner::new(
                    session.build_commit_request(),
                    usage,
                    chain_ids,
                    session.editable_tree_commits(),
                    session.reserved_group_ids(),
                );
                self.ask_or_review(origin, planner);
            }
            Err(error) => self.save_failed("Could not check where the edited data is used.", error.message),
        }
    }

    /// Shows the planner's next question, or the review once nothing is left to ask.
    fn ask_or_review(&mut self, origin: SaveOrigin, planner: SavePlanner) {
        if let Some(question) = planner.current() {
            let dialog = question_dialog(question);
            self.push_dialog(Dialog::message(dialog));
            self.save = SaveFlow::Asking { origin, planner: Box::new(planner) };
            return;
        }

        let request = planner.finish();
        let rows = build_review_rows(&self.state.session, &request);
        let dialect = self.state.settings.database.provider.sql_dialect();
        let sql = render_script(&request, &Local::now().format("%Y-%m-%d %H:%M:%S").to_string(), dialect);
        let settings = &self.state.settings;
        let target =
            if self.state.demo { "demo data (in memory)".to_owned() } else { settings.database.display_target() };
        let dialog = SaveReviewDialog::new(rows, sql, settings.save_to_sql_mode, target);
        self.push_dialog(Dialog::SaveReview(Box::new(dialog)));
        self.save = SaveFlow::Reviewing { origin, request };
    }

    pub(super) fn on_save_answer(&mut self, answer: SaveAnswer) {
        match self.take_save_flow() {
            SaveFlow::Asking { origin, mut planner } => {
                planner.answer(answer);
                self.ask_or_review(origin, *planner);
            }
            other => self.save = other,
        }
    }

    pub(super) fn on_save_cancelled(&mut self) {
        self.save = SaveFlow::Idle;
        self.set_status("Save cancelled. Staged changes are kept.");
    }

    pub(super) fn on_review_confirmed(&mut self) {
        let SaveFlow::Reviewing { origin, request } = self.take_save_flow() else {
            return;
        };

        if self.state.settings.save_to_sql_mode {
            self.export_sql(origin, &request);
            return;
        }

        let database = &self.state.settings.database;
        let target = if self.state.demo {
            "Target: demo data (in memory)".to_owned()
        } else {
            format!("Server: {},{}\nDatabase: {}", database.server, database.effective_port(), database.database)
        };
        let details = format!(
            "{target}\n\nSkillTree changes: {}\nSkill field updates: {}\nStatResource updates: {}\nGrowth bonus updates: {}\nRandom group updates: {}",
            request.trees.len(),
            request.skills.len(),
            request.stats.len(),
            request.level_bonuses.len(),
            request.random_groups.len()
        );
        let dialog = MessageDialog::new(
            MessageKind::Question,
            "Confirm database save",
            "Apply the staged changes to the database now?",
        )
        .details(details)
        .button(icons::DATABASE, "Save to database", ButtonStyle::Primary, Some(Action::CommitConfirmed))
        .button(icons::CLOSE, "Cancel", ButtonStyle::Plain, Some(Action::SaveCancelled))
        .on_dismiss(Action::SaveCancelled);
        self.push_dialog(Dialog::message(dialog));
        self.save = SaveFlow::Confirming { origin, request };
    }

    fn export_sql(&mut self, origin: SaveOrigin, request: &CommitRequest) {
        let now = Local::now();
        let Some(path) = rfd::FileDialog::new()
            .set_title("Export SQL script")
            .set_file_name(format!("summon-editor-diff-{}.sql", now.format("%Y%m%d-%H%M%S")))
            .add_filter("SQL files", &["sql"])
            .add_filter("All files", &["*"])
            .save_file()
        else {
            self.on_save_cancelled();
            return;
        };

        let dialect = self.state.settings.database.provider.sql_dialect();
        let sql = render_script(request, &now.format("%Y-%m-%d %H:%M:%S").to_string(), dialect);
        if let Err(error) = std::fs::write(&path, sql) {
            self.save_failed("The SQL file could not be written.", error.to_string());
            return;
        }

        self.toasts.push(ToastKind::Success, "SQL file generated. No database write was performed.");
        match origin {
            SaveOrigin::Toolbar => {
                self.set_status(format!("SQL diff saved: {}. Pending changes remain staged.", path.display()));
            }
            SaveOrigin::SummonSwitch(summon) => {
                self.load_chain(summon);
                self.set_status(format!(
                    "SQL diff saved: {}. Staged changes cleared for summon switch.",
                    path.display()
                ));
            }
        }
    }

    pub(super) fn on_commit_confirmed(&mut self) {
        let SaveFlow::Confirming { origin, request } = self.take_save_flow() else {
            return;
        };
        self.worker.send(Command::Commit { request });
        self.busy = Some("Saving changes to the database…");
        self.save = SaveFlow::Committing(origin);
    }

    pub(super) fn on_committed(&mut self, result: Outcome<CommitResult>) {
        self.busy = None;
        let SaveFlow::Committing(origin) = self.take_save_flow() else {
            return;
        };
        match result {
            Ok(result) => {
                let summary = format!(
                    "Saved trees: {}, skills: {}, random groups: {}.",
                    result.updated_trees, result.updated_skills, result.updated_random_groups
                );
                self.toasts.push(ToastKind::Success, format!("Changes saved successfully. {summary}"));

                // Reload from the database so the editor shows exactly what was written.
                let next = match origin {
                    SaveOrigin::Toolbar => self.current_summon.clone(),
                    SaveOrigin::SummonSwitch(summon) => Some(summon),
                };
                match next {
                    Some(summon) => self.load_chain(summon),
                    None => self.state.session.clear(),
                }
                self.set_status(summary);
            }
            Err(error) => self.save_failed(commit_failure_text(error.kind), error.message),
        }
    }

    fn save_failed(&mut self, text: &str, details: String) {
        self.save = SaveFlow::Idle;
        self.set_status("Save failed (see details).");
        self.push_dialog(Dialog::message(MessageDialog::error("Save failed", text, details)));
    }
}

/// What to tell the user after a failed commit. Only a rejection by the
/// server guarantees that the transaction was rolled back.
fn commit_failure_text(kind: FailureKind) -> &'static str {
    match kind {
        FailureKind::Request | FailureKind::NotConnected => {
            "Could not save the staged changes. The transaction was rolled back and nothing was written."
        }
        FailureKind::Connection | FailureKind::Timeout | FailureKind::Internal => {
            "The save was interrupted before the database confirmed it. The transaction is normally rolled back, \
             but this could not be verified. Reload the summon (F5) to see what the database contains before \
             saving again. Your staged changes are still here."
        }
    }
}

/// Turns a planner question into a dialog whose buttons answer it.
fn question_dialog(question: &SaveQuestion) -> MessageDialog {
    let (title, text, items): (&str, String, Vec<String>) = match question {
        SaveQuestion::SkillImpact { rows } => (
            "Global skill impact",
            "The edited skills are also used by other summons. The change applies to all of them.".to_owned(),
            rows.iter()
                .map(|r| {
                    format!("Skill {} → {} ({}), tree {}", r.skill_id, r.summon_name, r.summon_id, r.skill_tree_id)
                })
                .collect(),
        ),
        SaveQuestion::SharedRandomGroup { group_id, rows } => (
            "Shared random group",
            format!(
                "Random group {group_id} is used by other summons. Clone it and point this chain at the copy, or edit it in place for everyone?"
            ),
            rows.iter()
                .map(|r| {
                    format!(
                        "{} ({}) · tree slot {} (SkillTree {})",
                        r.summon_name, r.summon_id, r.tree_slot, r.skill_tree_id
                    )
                })
                .collect(),
        ),
        SaveQuestion::SharedTree { key, rows } => (
            "Shared skill tree",
            format!(
                "Tree {} (summon {}, slot {}) is shared with other summons. Clone it for this summon slot, or edit it in place for everyone?",
                key.tree_id, key.summon_id, key.slot
            ),
            rows.iter().map(|r| format!("{} ({}) · slot {}", r.summon_name, r.summon_id, r.tree_slot)).collect(),
        ),
        SaveQuestion::SharedStat { summon_id, stat_id, rows } => (
            "Shared stat row",
            format!(
                "StatResource {stat_id} is shared with other summons. Clone it for summon {summon_id}, or edit it in place for everyone?"
            ),
            rows.iter().map(|r| format!("{} ({})", r.summon_name, r.summon_id)).collect(),
        ),
        SaveQuestion::LevelBonusImpact { rows } => (
            "Shared growth bonus",
            "The edited growth bonus rows are also used by other summons. The change applies to all of them."
                .to_owned(),
            rows.iter()
                .map(|r| {
                    format!(
                        "Bonus {} → {} ({}), growth stage {}",
                        r.bonus_id, r.summon_name, r.summon_id, r.stage_index
                    )
                })
                .collect(),
        ),
    };

    let dialog = MessageDialog::new(MessageKind::Warning, title, text).items(items);
    let dialog = if question.is_choice() {
        dialog
            .button(icons::DUPLICATE, "Clone", ButtonStyle::Primary, Some(Action::SaveAnswer(SaveAnswer::Clone)))
            .button(icons::EDIT, "Edit in place", ButtonStyle::Danger, Some(Action::SaveAnswer(SaveAnswer::InPlace)))
    } else {
        dialog.button(icons::CHECK, "Continue", ButtonStyle::Primary, Some(Action::SaveAnswer(SaveAnswer::Continue)))
    };
    dialog
        .button(icons::CLOSE, "Cancel save", ButtonStyle::Plain, Some(Action::SaveCancelled))
        .on_dismiss(Action::SaveCancelled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_server_rejection_promises_that_nothing_was_written() {
        assert!(commit_failure_text(FailureKind::Request).contains("nothing was written"));
        for uncertain in [FailureKind::Connection, FailureKind::Timeout, FailureKind::Internal] {
            let text = commit_failure_text(uncertain);
            assert!(!text.contains("nothing was written"), "{text}");
            assert!(text.contains("could not be verified"), "{text}");
        }
    }
}
