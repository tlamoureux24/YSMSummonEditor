//! Bottom bar: last status message on the left; image index, connection and
//! version on the right.

use eframe::egui::{self, Align, Color32, Layout, Margin, RichText, Stroke, Ui};

use super::ViewContext;
use crate::action::Action;
use crate::icons;
use crate::state::ConnectionStatus;
use crate::theme::{self, color, space};
use crate::widgets;
use ysm_core::i18n::{tr, trf};

const BAR_HEIGHT: f32 = 24.0;

pub fn show(ui: &mut Ui, view: &mut ViewContext<'_>) {
    let state = view.state;
    let frame = egui::Frame::new().fill(color::window_bg()).inner_margin(Margin::symmetric(space::MD as i8, 4));
    egui::Panel::bottom("status-bar").frame(frame).show_separator_line(false).show(ui, |ui| {
        let top = ui.max_rect().top() - 4.0;
        ui.painter().hline(ui.ctx().content_rect().x_range(), top, Stroke::new(1.0, color::border()));

        ui.horizontal(|ui| {
            ui.set_height(BAR_HEIGHT);
            ui.label(widgets::caption(state.status.as_str()));

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = space::MD;
                ui.label(widgets::caption(concat!("v", env!("CARGO_PKG_VERSION"))));
                connection(ui, view);
                assets(ui, view);
            });
        });
    });
}

fn item(icon: &str, text: &str, tint: Color32) -> RichText {
    RichText::new(format!("{icon}  {text}")).size(theme::CAPTION_SIZE).color(tint)
}

fn connection(ui: &mut Ui, view: &mut ViewContext<'_>) {
    let state = view.state;
    let (icon, text, tint, detail) = match &state.connection {
        ConnectionStatus::Connected(server) => {
            (icons::CONNECTION, state.settings.database.display_target(), color::success(), server.clone())
        }
        ConnectionStatus::Demo => (
            icons::DATABASE,
            tr("Demo data").to_owned(),
            color::warning(),
            tr("Built-in sample data; start without --demo to use a database.").to_owned(),
        ),
        ConnectionStatus::Connecting => {
            (icons::CONNECTION, tr("Connecting…").to_owned(), color::text_muted(), String::new())
        }
        ConnectionStatus::Failed(error) => {
            (icons::NO_CONNECTION, tr("Connection failed").to_owned(), color::danger(), error.clone())
        }
        ConnectionStatus::Lost(error) => (
            icons::NO_CONNECTION,
            tr("Connection lost").to_owned(),
            color::danger(),
            trf("{}\n\nThe next request reconnects automatically.", &[&error]),
        ),
        ConnectionStatus::NotConfigured => (
            icons::NO_CONNECTION,
            tr("Database not configured").to_owned(),
            color::warning(),
            tr("Open Settings to set up the connection.").to_owned(),
        ),
    };

    if state.connection.can_reconnect() {
        let reconnect = egui::Button::new(item(icons::REFRESH, tr("Reconnect"), color::text())).small();
        if ui.add(reconnect).clicked() {
            view.actions.push(Action::Reconnect);
        }
    }
    if state.connection == ConnectionStatus::Connecting {
        ui.add(egui::Spinner::new().size(theme::CAPTION_SIZE));
    }
    let label = ui.label(item(icon, &text, tint));
    if !detail.is_empty() {
        label.on_hover_text(detail);
    }
}

fn assets(ui: &mut Ui, view: &ViewContext<'_>) {
    let state = view.state;
    match state.asset_count {
        Some(count) => {
            let tint = if count == 0 { color::warning() } else { color::text_muted() };
            ui.label(item(icons::IMAGES, &trf("{} images", &[&count]), tint))
                .on_hover_text(trf("Dump folder: {}", &[&state.settings.dump_folder_path]));
        }
        None => {
            ui.add(egui::Spinner::new().size(theme::CAPTION_SIZE));
            ui.label(item(icons::IMAGES, tr("Indexing images…"), color::text_muted()));
        }
    }
}
