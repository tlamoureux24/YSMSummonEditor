//! Common chrome for modal dialogs: padding, title and footer.
//!
//! Every dialog has 24 px of padding, an 18 px title, and a footer separated
//! by a hairline with the buttons right-aligned, the affirmative one first.

use eframe::egui::{self, Color32, Margin, RichText, Stroke, Ui, Vec2};

use crate::theme::{self, color, space};
use crate::widgets;

pub struct ModalOutcome {
    /// The user pressed Escape or clicked outside the dialog.
    pub dismissed: bool,
}

/// Shows a modal with an icon, a title and a fixed content width.
pub fn show(
    ctx: &egui::Context,
    id: egui::Id,
    icon: &str,
    icon_color: Color32,
    title: &str,
    width: f32,
    add_contents: impl FnOnce(&mut Ui),
) -> ModalOutcome {
    let frame = egui::Frame::popup(&ctx.global_style())
        .fill(color::panel_bg())
        .stroke(Stroke::new(1.0, color::control_border()))
        .corner_radius(theme::CARD_RADIUS)
        .inner_margin(Margin::same(space::XL as i8));

    let response = egui::Modal::new(id).frame(frame).show(ctx, |ui| {
        ui.set_width(width.min(ui.ctx().content_rect().width() - 96.0));
        ui.spacing_mut().item_spacing = Vec2::splat(space::SM);

        ui.horizontal(|ui| {
            ui.set_height(24.0);
            ui.label(RichText::new(icon).size(theme::TITLE_SIZE).color(icon_color));
            ui.label(RichText::new(title).font(theme::bold(theme::TITLE_SIZE)));
        });
        ui.add_space(space::SM);
        add_contents(ui);
    });
    ModalOutcome { dismissed: response.should_close() }
}

/// The button row of a dialog. Buttons are added right to left, so add
/// Cancel first and the affirmative button after it.
pub fn footer<R>(ui: &mut Ui, add_buttons: impl FnOnce(&mut Ui) -> R) -> R {
    ui.add_space(space::SM);
    widgets::divider(ui);
    ui.add_space(space::SM);
    // A bounded row: an unbounded right-to-left layout would claim all remaining height.
    let size = Vec2::new(ui.available_width(), theme::CONTROL_HEIGHT);
    ui.allocate_ui_with_layout(size, egui::Layout::right_to_left(egui::Align::Center), add_buttons).inner
}

/// A one-line status or validation message inside a dialog.
pub fn note(ui: &mut Ui, icon: &str, text: &str, tint: Color32) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(format!("{icon}  {text}")).size(theme::CAPTION_SIZE).color(tint));
    });
}

/// Whether Enter was pressed this frame, to trigger a dialog's default action.
pub fn enter_pressed(ui: &Ui) -> bool {
    ui.input(|input| input.key_pressed(egui::Key::Enter))
}
