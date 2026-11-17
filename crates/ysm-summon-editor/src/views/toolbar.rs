//! Top bar: branding on the left, global actions on the right.
//!
//! Actions are grouped by purpose and separated by dividers:
//! history (undo, redo, discard) · save target and Save · stats · settings.

use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{self, Align, Layout, Margin, Stroke, Ui, Vec2};

use super::ViewContext;
use crate::action::Action;
use crate::icons;
use crate::theme::{self, color, space};
use crate::widgets::{self, Segment};
use ysm_core::i18n::{tr, trf};

pub fn show(ui: &mut Ui, view: &mut ViewContext<'_>) {
    let frame = egui::Frame::new().fill(color::window_bg()).inner_margin(Margin::symmetric(space::MD as i8, 6));
    egui::Panel::top("toolbar").frame(frame).show_separator_line(false).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.set_height(theme::CONTROL_HEIGHT);
            branding(ui, view);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| actions(ui, view));
        });
    });
}

fn branding(ui: &mut Ui, view: &ViewContext<'_>) {
    if let Some(logo) = view.textures.logo() {
        let height = 28.0;
        ui.add(egui::Image::from_texture((logo.id(), Vec2::new(height * logo.aspect_ratio(), height))));
    }

    // One text layout, so the title and the byline share a baseline.
    let mut job = LayoutJob::default();
    let title = TextFormat { font_id: theme::bold(theme::TITLE_SIZE), color: color::text(), ..Default::default() };
    let byline = TextFormat { font_id: theme::caption(), color: color::text_muted(), ..Default::default() };
    job.append("Summon Editor", 0.0, title);
    job.append(tr("made by YoSiem"), space::SM, byline);
    ui.add(egui::Label::new(job).selectable(false));

    if view.state.demo {
        widgets::badge(ui, tr("DEMO DATA"), color::warning())
            .on_hover_text(tr("Running on built-in sample data. Nothing is written to a database."));
    }
}

/// Laid out right to left, so the first group here is the rightmost one.
fn actions(ui: &mut Ui, view: &mut ViewContext<'_>) {
    let state = view.state;
    let session = &state.session;
    let changes = session.change_count();

    if widgets::button(ui, icons::SETTINGS, tr("Settings"))
        .on_hover_text(tr("Database connection, dump folder and appearance"))
        .clicked()
    {
        view.actions.push(Action::OpenSettings);
    }
    let stats = widgets::button_enabled(ui, state.can_open_stats(), icons::STATS, tr("Stats"))
        .on_hover_text(tr("Base stats, growth bonuses and the scaling calculator"))
        .on_disabled_hover_text(tr("Select a summon first"));
    if stats.clicked() {
        view.actions.push(Action::OpenStats);
    }

    group_divider(ui);

    let label = if changes > 0 { trf("Save · {}", &[&changes]) } else { tr("Save").to_owned() };
    let save = widgets::primary_button_enabled(ui, changes > 0, icons::SAVE, &label)
        .on_hover_text(tr("Review and save the staged changes (Ctrl+S)"))
        .on_disabled_hover_text(tr("No staged changes"));
    if save.clicked() {
        view.actions.push(Action::Save);
    }

    let to_sql = state.settings.save_to_sql_mode;
    let segments = [
        Segment { icon: icons::DATABASE, label: tr("DB"), tooltip: tr("Save writes directly to the database") },
        Segment {
            icon: icons::SQL_FILE,
            label: ".sql",
            tooltip: tr("Save writes a .sql script; the database is not touched"),
        },
    ];
    if let Some(index) = widgets::segmented(ui, &segments, usize::from(to_sql), 72.0) {
        view.actions.push(Action::SetSaveToSql(index == 1));
    }
    ui.label(widgets::muted(tr("Save to")));

    group_divider(ui);

    if widgets::icon_button_enabled(ui, changes > 0, icons::DISCARD, tr("Discard all staged changes")).clicked() {
        view.actions.push(Action::AskDiscardAll);
    }
    if widgets::icon_button_enabled(ui, session.can_redo(), icons::REDO, tr("Redo (Ctrl+Y)")).clicked() {
        view.actions.push(Action::Redo);
    }
    if widgets::icon_button_enabled(ui, session.can_undo(), icons::UNDO, tr("Undo (Ctrl+Z)")).clicked() {
        view.actions.push(Action::Undo);
    }
}

/// A short vertical hairline between groups of toolbar actions.
fn group_divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(space::SM, theme::CONTROL_HEIGHT), egui::Sense::hover());
    ui.painter().vline(rect.center().x, rect.y_range().shrink(6.0), Stroke::new(1.0, color::control_border()));
}
