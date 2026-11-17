//! Database connection and dump folder settings.
//!
//! A two-column form: labels in a fixed column, every control starting on
//! one shared line, help text under the control it belongs to.

use eframe::egui::{self, RichText, Ui};
use ysm_data::{AppSettings, DbProvider};

use super::DialogContext;
use crate::action::Action;
use crate::icons;
use crate::theme::{self, color, space};
use crate::widgets::{self, modal};
use crate::worker::{Command, Event};

/// Width of ordinary fields; the port field is a short numeric.
const FIELD_WIDTH: f32 = 280.0;
const PORT_WIDTH: f32 = 80.0;

enum TestState {
    Idle,
    Running,
    Passed(String),
    Failed(String),
}

pub struct SettingsDialog {
    original: AppSettings,
    working: AppSettings,
    original_port: String,
    port_text: String,
    show_password: bool,
    test: TestState,
    validation: Option<String>,
    /// The connection cannot be tested in demo mode; say so instead of failing oddly.
    demo: bool,
}

impl SettingsDialog {
    pub fn new(current: &AppSettings, demo: bool) -> Self {
        let port = current.database.port.to_string();
        Self {
            original: current.clone(),
            working: current.clone(),
            original_port: port.clone(),
            port_text: port,
            show_password: false,
            test: TestState::Idle,
            validation: None,
            demo,
        }
    }

    pub fn on_event(&mut self, event: &Event) {
        if let Event::ConnectionTested(result) = event {
            self.test = match result {
                Ok(server) => TestState::Passed(server.clone()),
                Err(error) => TestState::Failed(error.message.clone()),
            };
        }
    }

    fn is_dirty(&self) -> bool {
        self.working != self.original || self.port_text.trim() != self.original_port
    }

    /// Copies the port text into the settings; returns a message when the form is invalid.
    fn validate(&mut self) -> Option<String> {
        let database = &mut self.working.database;
        match self.port_text.trim().parse::<u16>() {
            Ok(port) if port > 0 => database.port = i32::from(port),
            _ => return Some("Port must be a number between 1 and 65535.".to_owned()),
        }
        database.server = database.server.trim().to_owned();
        database.database = database.database.trim().to_owned();
        database.user_id = database.user_id.trim().to_owned();
        self.working.dump_folder_path = self.working.dump_folder_path.trim().to_owned();
        if !database.is_configured() {
            return Some("Server, Database and User ID are required.".to_owned());
        }
        None
    }

    pub fn show(&mut self, ctx: &egui::Context, id: egui::Id, env: &mut DialogContext<'_>) -> bool {
        let mut save = false;
        let mut cancelled = false;

        let outcome = modal::show(ctx, id, icons::SETTINGS, color::ACCENT_BRIGHT, "Settings", 560.0, |ui| {
            ui.spacing_mut().item_spacing.y = space::MD;

            widgets::section_title(ui, "Database");
            self.database_form(ui, env);

            ui.add_space(space::XS);
            widgets::divider(ui);
            ui.add_space(space::XS);

            widgets::section_title(ui, "Client dump folder");
            widgets::form_row(ui, "Folder", |ui| {
                ui.add(widgets::sized_text_field(&mut self.working.dump_folder_path, FIELD_WIDTH));
                if widgets::button(ui, icons::FOLDER, "Browse").clicked()
                    && let Some(folder) = rfd::FileDialog::new().set_title("Select dump folder").pick_folder()
                {
                    self.working.dump_folder_path = folder.to_string_lossy().into_owned();
                }
            });
            help(ui, "Scanned recursively for summon and skill images (png, jpg, bmp, webp, tga, dds).");

            if let Some(message) = &self.validation {
                widgets::form_note(ui, |ui| modal::note(ui, icons::WARNING, message, color::DANGER));
            }

            let dirty = self.is_dirty();
            modal::footer(ui, |ui| {
                if widgets::button(ui, icons::CLOSE, "Cancel").clicked() {
                    cancelled = true;
                }
                let clicked = widgets::primary_button_enabled(ui, dirty, icons::SAVE, "Save").clicked();
                if clicked || (dirty && modal::enter_pressed(ui)) {
                    self.validation = self.validate();
                    save = self.validation.is_none();
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    if dirty {
                        ui.label(widgets::caption("Unsaved changes"));
                    }
                });
            });
        });

        if save {
            env.actions.push(Action::ApplySettings(Box::new(self.working.clone())));
            return false;
        }
        // Escape and a click outside only close a dialog with nothing to lose.
        !(cancelled || (outcome.dismissed && !self.is_dirty()))
    }

    fn database_form(&mut self, ui: &mut Ui, env: &mut DialogContext<'_>) {
        let database = &mut self.working.database;
        let mut edited = false;

        widgets::form_row(ui, "Server type", |ui| {
            let segments = DbProvider::ALL.map(|provider| widgets::Segment {
                icon: icons::DATABASE,
                label: provider.label(),
                tooltip: match provider {
                    DbProvider::Mssql => "Microsoft SQL Server",
                    DbProvider::PostgreSql => "PostgreSQL; tables are found through the database's search_path",
                },
            });
            let selected = DbProvider::ALL.iter().position(|p| *p == database.provider).unwrap_or(0);
            if let Some(index) = widgets::segmented(ui, &segments, selected, 112.0) {
                let provider = DbProvider::ALL[index];
                if provider != database.provider {
                    // Follow the default port unless the user typed another one.
                    if self.port_text.trim() == database.provider.default_port().to_string() {
                        self.port_text = provider.default_port().to_string();
                    }
                    database.provider = provider;
                    edited = true;
                }
            }
        });
        widgets::form_row(ui, "Server", |ui| {
            let field =
                widgets::sized_text_field(&mut database.server, FIELD_WIDTH).hint_text("Host name or IP address");
            edited |= ui.add(field).changed();
        });
        widgets::form_row(ui, "Port", |ui| {
            edited |= ui.add(widgets::sized_text_field(&mut self.port_text, PORT_WIDTH)).changed();
        });
        widgets::form_row(ui, "Database", |ui| {
            edited |= ui.add(widgets::sized_text_field(&mut database.database, FIELD_WIDTH)).changed();
        });
        widgets::form_row(ui, "User ID", |ui| {
            edited |= ui.add(widgets::sized_text_field(&mut database.user_id, FIELD_WIDTH)).changed();
        });
        widgets::form_row(ui, "Password", |ui| {
            // Narrower by the reveal button, so the row ends where the other fields end.
            let width = FIELD_WIDTH - theme::CONTROL_HEIGHT - space::SM;
            let field = widgets::sized_text_field(&mut database.password, width).password(!self.show_password);
            edited |= ui.add(field).changed();
            let (icon, hint) =
                if self.show_password { (icons::HIDE, "Hide password") } else { (icons::SHOW, "Show password") };
            if widgets::icon_button(ui, icon, hint).clicked() {
                self.show_password = !self.show_password;
            }
        });
        match database.provider {
            DbProvider::Mssql => {
                help(ui, "Stored in plain text in settings.json under %LOCALAPPDATA%\\YSMSummonEditor.");
                widgets::form_row(ui, "Trust server certificate", |ui| {
                    edited |= widgets::toggle(ui, &mut database.trust_server_certificate, "Trust server certificate")
                        .on_hover_text(
                            "Accept the server's certificate without validating it; usual for a local server.",
                        )
                        .changed();
                });
            }
            DbProvider::PostgreSql => help(
                ui,
                "Leave empty to read it from the PostgreSQL password file (PGPASSFILE, ~/.pgpass or \
                 %APPDATA%\\postgresql\\pgpass.conf). A password typed here is stored in plain text in settings.json.",
            ),
        }

        if edited {
            self.test = TestState::Idle;
            self.validation = None;
        }

        widgets::form_row(ui, "", |ui| {
            let running = matches!(self.test, TestState::Running);
            let label = if running { "Testing…" } else { "Test connection" };
            let test = widgets::button_enabled(ui, !running && !self.demo, icons::CONNECTION, label)
                .on_disabled_hover_text(if self.demo { "Not available in demo mode" } else { "Test in progress" });
            if test.clicked() {
                self.validation = self.validate();
                if self.validation.is_none() {
                    self.test = TestState::Running;
                    env.actions.push(Action::Command(Command::TestConnection(self.working.database.clone())));
                }
            }
            match &self.test {
                TestState::Idle => {}
                TestState::Running => {
                    ui.spinner();
                }
                TestState::Passed(server) => {
                    ui.label(RichText::new(format!("{}  Connected", icons::SUCCESS)).color(color::SUCCESS))
                        .on_hover_text(server.as_str());
                }
                TestState::Failed(error) => {
                    ui.label(RichText::new(format!("{}  Failed", icons::ERROR)).color(color::DANGER))
                        .on_hover_text(error.as_str());
                }
            }
        });
        if let TestState::Failed(error) = &self.test {
            // The first line says what went wrong; the tooltip above holds the rest.
            let first_line = error.lines().next().unwrap_or_default();
            widgets::form_note(ui, |ui| {
                ui.add(egui::Label::new(widgets::caption(first_line).color(color::DANGER)).wrap());
            });
        }
    }
}

/// Help text under the control of the row above it.
fn help(ui: &mut Ui, text: &str) {
    ui.add_space(-space::SM);
    widgets::form_note(ui, |ui| {
        ui.add(egui::Label::new(widgets::caption(text)).wrap());
    });
}
