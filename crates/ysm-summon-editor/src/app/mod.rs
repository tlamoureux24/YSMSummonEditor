//! The application: owns all state and wires views, dialogs and the worker together.
//!
//! One frame: apply worker events, draw the views (which push actions), draw
//! the dialogs, then reduce the collected actions. See `events`, `actions`
//! and `save_flow` for the three halves of the logic.

mod actions;
#[cfg(debug_assertions)]
mod devtools;
mod events;
mod save_flow;

use std::path::PathBuf;
use std::time::Duration;

use eframe::egui::{self, Key, Margin, Modifiers};
use ysm_core::model::SummonListItem;
use ysm_data::settings::ensure_default_dump_folder;
use ysm_data::{AppSettings, SettingsStore, env_local};

use crate::action::Action;
use crate::dialogs::{Dialog, DialogContext};
use crate::state::{AppState, ConnectionStatus};
use crate::textures::TextureStore;
use crate::theme::{self, color, space};
use crate::views::{self, ViewContext};
use crate::widgets::toast::{ToastKind, Toasts};
use crate::worker::{Command, WorkerHandle};

use save_flow::SaveFlow;
use ysm_core::i18n::{self, Language, tr, trf};

/// Delay between the last keystroke in the search box and the live search.
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(250);

pub struct EditorApp {
    state: AppState,
    /// The operating system's language, used until one is chosen in Settings.
    system_language: Language,
    worker: WorkerHandle,
    textures: TextureStore,
    settings_store: SettingsStore,
    dialogs: Vec<Dialog>,
    toasts: Toasts,
    save: SaveFlow,
    /// Label of a blocking background operation; shown as a modal spinner.
    busy: Option<&'static str>,
    /// The summon the user picked; its chain is what the session holds.
    current_summon: Option<SummonListItem>,
    summon_generation: u64,
    chain_generation: u64,
    /// Frames to wait before a keyboard-triggered save, so a focused cell commits first.
    save_countdown: Option<u8>,
    exit_prompt_open: bool,
    exit_confirmed: bool,
    worker_failure_reported: bool,
    /// Scripted UI playback, see `devtools`.
    #[cfg(debug_assertions)]
    dev_script: Option<devtools::DevScript>,
}

impl EditorApp {
    pub fn new(cc: &eframe::CreationContext<'_>, demo: bool) -> Self {
        let ctx = &cc.egui_ctx;
        theme::apply(ctx);
        let system_language = sys_locale::get_locale().map_or(Language::English, |l| Language::from_locale(&l));
        i18n::set_language(system_language);

        let mut toasts = Toasts::default();
        let mut notes: Vec<String> = Vec::new();
        let settings_store = SettingsStore::at_default_location();
        let mut settings = settings_store.load().unwrap_or_else(|error| {
            toasts.push(ToastKind::Error, trf("Settings could not be read, using defaults. {}", &[&error]));
            AppSettings::default()
        });

        if !demo
            && !settings.database.is_configured()
            && let Some((database, path)) = env_local::load_db_settings()
        {
            settings.database = database;
            let file = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
            notes.push(trf("Loaded DB defaults from {}.", &[&file]));
        }

        match ensure_default_dump_folder(&mut settings) {
            Ok(true) => {
                notes.push(tr("Dump folder defaulted to Desktop\\Dump.").to_owned());
                if let Err(error) = settings_store.save(&settings) {
                    toasts.push(ToastKind::Warning, trf("Settings could not be saved. {}", &[&error]));
                }
            }
            Ok(false) => {}
            Err(error) => {
                toasts.push(ToastKind::Warning, trf("Default dump folder could not be created. {}", &[&error]))
            }
        }

        i18n::set_language(settings.language.unwrap_or(system_language));
        let mut state = AppState::new(settings, demo);
        state.status = notes.join(" ");

        let mut app = Self {
            state,
            system_language,
            worker: WorkerHandle::spawn(ctx.clone()),
            textures: TextureStore::new(ctx),
            settings_store,
            dialogs: Vec::new(),
            toasts,
            save: SaveFlow::Idle,
            busy: None,
            current_summon: None,
            summon_generation: 0,
            chain_generation: 0,
            save_countdown: None,
            exit_prompt_open: false,
            exit_confirmed: false,
            worker_failure_reported: false,
            #[cfg(debug_assertions)]
            dev_script: devtools::DevScript::from_env(),
        };
        app.connect();
        app.rebuild_assets();
        app
    }

    /// (Re)connects to the configured database, or to the demo data.
    fn connect(&mut self) {
        if self.state.demo {
            self.state.connection = ConnectionStatus::Connecting;
            self.worker.send(Command::UseDemo);
        } else if self.state.settings.database.is_configured() {
            self.state.connection = ConnectionStatus::Connecting;
            self.worker.send(Command::Connect(self.state.settings.database.clone(), self.language()));
        } else {
            self.state.connection = ConnectionStatus::NotConfigured;
            self.append_status(tr("Database is not configured. Add .env.local or set values in Settings."));
        }
    }

    /// The interface language: the one chosen in Settings, else the system's.
    fn language(&self) -> Language {
        self.state.settings.language.unwrap_or(self.system_language)
    }

    fn rebuild_assets(&mut self) {
        self.state.asset_count = None;
        self.worker.send(Command::BuildAssetIndex(PathBuf::from(self.state.settings.dump_folder_path.trim())));
    }

    fn set_status(&mut self, text: impl Into<String>) {
        self.state.status = text.into();
    }

    fn append_status(&mut self, text: &str) {
        if self.state.status.is_empty() {
            self.state.status = text.to_owned();
        } else {
            self.state.status = format!("{} {text}", self.state.status);
        }
    }

    fn push_dialog(&mut self, dialog: Dialog) {
        self.dialogs.push(dialog);
    }

    fn save_settings(&mut self) {
        if let Err(error) = self.settings_store.save(&self.state.settings) {
            self.toasts.push(ToastKind::Error, trf("Settings could not be saved. {}", &[&error]));
        }
    }

    fn search_summons(&mut self) {
        self.state.search_edited_at = None;
        if !self.state.connection.is_ready() {
            self.set_status(tr("Search skipped. Database is not connected."));
            return;
        }
        self.summon_generation += 1;
        self.state.summons_loading = true;
        self.set_status(tr("Loading summons…"));
        self.worker
            .send(Command::SearchSummons { filter: self.state.summon_filter(), generation: self.summon_generation });
    }

    /// Replaces the session with the chain of `summon`. Staged changes are dropped.
    fn load_chain(&mut self, summon: SummonListItem) {
        self.chain_generation += 1;
        self.state.session.clear();
        self.state.selected_summon = Some(summon.summon_id);
        self.state.selected_row = None;
        self.state.selected_effective = None;
        self.state.chain_loading = true;
        self.set_status(tr("Loading evolution chain…"));
        self.worker.send(Command::LoadChain { summon: summon.clone(), generation: self.chain_generation });
        self.current_summon = Some(summon);
    }

    /// Asks the worker for skills and random groups the session refers to but does not know yet.
    fn request_missing_data(&mut self) {
        let skills = self.state.session.take_missing_skill_ids();
        if !skills.is_empty() {
            self.worker.send(Command::LoadSkillDisplay { ids: skills });
        }
        let groups = self.state.session.take_missing_group_ids();
        if !groups.is_empty() {
            self.worker.send(Command::LoadRandomGroups { ids: groups });
        }
    }

    /// Drops selections that no longer point at an existing row.
    fn validate_selection(&mut self) {
        let state = &mut self.state;
        if let Some(uid) = state.selected_row
            && state.session.tree(state.active_tree_ref()).and_then(|tree| tree.row(uid)).is_none()
        {
            state.selected_row = None;
        }
        if let Some((skill_id, lv)) = state.selected_effective
            && !state
                .session
                .effective_rows(state.active_stage)
                .iter()
                .any(|row| (row.skill_id, row.lv) == (skill_id, lv))
        {
            state.selected_effective = None;
        }
    }

    fn debounced_search(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let Some(edited_at) = self.state.search_edited_at else {
            return;
        };
        let elapsed = edited_at.elapsed();
        if elapsed >= SEARCH_DEBOUNCE {
            self.state.search_edited_at = None;
            actions.push(Action::Search);
        } else {
            ctx.request_repaint_after(SEARCH_DEBOUNCE - elapsed);
        }
    }

    /// Keeps the window open while changes are staged, until the user confirms.
    fn guard_close(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let close_requested = ctx.input(|input| input.viewport().close_requested());
        if close_requested && !self.exit_confirmed && self.state.session.has_changes() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if !self.exit_prompt_open {
                actions.push(Action::AskExit);
            }
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let editing_text = ctx.text_edit_focused();
        let state = &self.state;
        let tree = state.active_tree_ref();
        let selected_row = state.selected_row;

        let mut save = false;
        ctx.input_mut(|input| {
            save = input.consume_key(Modifiers::COMMAND, Key::S);
            if input.consume_key(Modifiers::COMMAND, Key::F) {
                actions.push(Action::FocusSearch);
            }
            if input.consume_key(Modifiers::NONE, Key::F5) {
                actions.push(Action::Reload);
            }
            // While a text field has focus these keys belong to it.
            if editing_text {
                return;
            }
            if input.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z)
                || input.consume_key(Modifiers::COMMAND, Key::Y)
            {
                actions.push(Action::Redo);
            } else if input.consume_key(Modifiers::COMMAND, Key::Z) {
                actions.push(Action::Undo);
            }
            if input.consume_key(Modifiers::NONE, Key::Insert) {
                actions.push(Action::AddRow(tree));
            }
            if input.consume_key(Modifiers::NONE, Key::Delete)
                && let Some(uid) = selected_row
            {
                actions.push(Action::AskDeleteRow(tree, uid));
            }
        });

        if save {
            if editing_text {
                // Let the focused cell commit its value before the save reads the session.
                ctx.memory_mut(|memory| {
                    if let Some(focused) = memory.focused() {
                        memory.surrender_focus(focused);
                    }
                });
                self.save_countdown = Some(1);
                ctx.request_repaint();
            } else {
                actions.push(Action::Save);
            }
        }
    }

    fn show_dialogs(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let mut env = DialogContext { session: &self.state.session, textures: &mut self.textures, actions };
        let mut index = 0;
        self.dialogs.retain_mut(|dialog| {
            let keep = dialog.show(ctx, egui::Id::new(("dialog", index)), &mut env);
            index += 1;
            keep
        });
    }

    fn show_busy(&self, ctx: &egui::Context) {
        let Some(label) = self.busy else {
            return;
        };
        egui::Modal::new(egui::Id::new("busy")).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(label);
            });
        });
    }
}

impl eframe::App for EditorApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        theme::follow(&ctx, self.state.settings.theme);
        i18n::set_language(self.language());

        self.textures.poll();
        if !self.worker.is_alive() && !std::mem::replace(&mut self.worker_failure_reported, true) {
            // Nothing will ever answer, so stop waiting for it.
            self.busy = None;
            self.state.summons_loading = false;
            self.state.chain_loading = false;
            self.toasts.push(
                ToastKind::Error,
                tr("The background worker stopped, so the database can no longer be reached. Restart the application."),
            );
        }
        while let Some(event) = self.worker.try_recv() {
            for dialog in &mut self.dialogs {
                dialog.on_event(&event);
            }
            self.handle_event(event);
        }

        let mut actions = Vec::new();
        self.debounced_search(&ctx, &mut actions);
        self.guard_close(&ctx, &mut actions);
        #[cfg(debug_assertions)]
        self.run_dev_script(&ctx, &mut actions);
        if self.dialogs.is_empty() && self.busy.is_none() {
            self.shortcuts(&ctx, &mut actions);
        }

        {
            let mut view = ViewContext { state: &self.state, textures: &mut self.textures, actions: &mut actions };
            views::toolbar::show(ui, &mut view);
            views::status_bar::show(ui, &mut view);
        }
        views::summon_list::show(ui, &mut self.state, &mut self.textures, &mut actions);
        {
            let mut view = ViewContext { state: &self.state, textures: &mut self.textures, actions: &mut actions };
            // Every card is separated from its neighbours and the window edge by the same gap.
            let gap = space::SM as i8;
            let frame = egui::Frame::new().fill(color::window_bg()).inner_margin(Margin {
                left: gap,
                right: gap,
                top: 2,
                bottom: gap,
            });
            egui::CentralPanel::default().frame(frame).show(ui, |ui| {
                egui::Panel::top("summon-header")
                    .frame(egui::Frame::NONE)
                    .resizable(false)
                    .show_separator_line(false)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = space::SM;
                        views::summon_header::show(ui, &mut view);
                        views::stage_tabs::show(ui, &mut view);
                        ui.add_space(0.0);
                    });
                views::effective_panel::show(ui, &mut view);
                egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
                    views::tree_panel::show(ui, &mut view);
                });
            });
        }

        // A keyboard save waits one frame so the cell that had focus can commit first.
        match self.save_countdown {
            Some(0) => {
                self.save_countdown = None;
                actions.push(Action::Save);
            }
            Some(frames) => {
                self.save_countdown = Some(frames - 1);
                ctx.request_repaint();
            }
            None => {}
        }

        self.show_dialogs(&ctx, &mut actions);
        self.show_busy(&ctx);
        self.toasts.show(&ctx);

        for action in actions {
            self.handle_action(&ctx, action);
        }
        self.validate_selection();
        self.request_missing_data();
    }
}
