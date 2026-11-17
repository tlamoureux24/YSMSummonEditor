//! Reusable widgets shared by views and dialogs.
//!
//! Every control built here is [`theme::CONTROL_HEIGHT`] high, so anything
//! placed in one row lines up without per-view tweaking.

pub mod game_tooltip;
pub mod int_cell;
pub mod modal;
pub mod toast;

use eframe::egui::text::{LayoutJob, TextFormat, TextWrapping};
use eframe::egui::{
    self, Align, Color32, CornerRadius, Layout, Margin, Rangef, Rect, Response, RichText, Sense, Stroke, StrokeKind,
    TextureId, Ui, Vec2, WidgetInfo, WidgetType,
};

use crate::theme::{self, color, space};

/// `icon` followed by `text`, for buttons and menu items.
pub fn icon_text(icon: &str, text: &str) -> String {
    format!("{icon}  {text}")
}

// ---- buttons ---------------------------------------------------------------

fn text_button<'a>(icon: &str, text: &str) -> egui::Button<'a> {
    egui::Button::new(icon_text(icon, text)).min_size(Vec2::new(theme::BUTTON_MIN_WIDTH, theme::CONTROL_HEIGHT))
}

/// A secondary action.
pub fn button(ui: &mut Ui, icon: &str, text: &str) -> Response {
    ui.add(text_button(icon, text))
}

/// A secondary action that can be disabled.
pub fn button_enabled(ui: &mut Ui, enabled: bool, icon: &str, text: &str) -> Response {
    ui.add_enabled(enabled, text_button(icon, text))
}

/// The main action of a dialog or toolbar.
pub fn primary_button(ui: &mut Ui, icon: &str, text: &str) -> Response {
    primary_button_enabled(ui, true, icon, text)
}

pub fn primary_button_enabled(ui: &mut Ui, enabled: bool, icon: &str, text: &str) -> Response {
    let (fill, tint) = if enabled {
        (color::primary(), Color32::WHITE)
    } else {
        (color::primary_disabled(), color::on_primary_disabled())
    };
    let label = RichText::new(icon_text(icon, text)).color(tint);
    let button = egui::Button::new(label)
        .fill(fill)
        .stroke(Stroke::NONE)
        .min_size(Vec2::new(theme::BUTTON_MIN_WIDTH, theme::CONTROL_HEIGHT));
    ui.add_enabled(enabled, button)
}

/// A destructive action.
pub fn danger_button(ui: &mut Ui, icon: &str, text: &str) -> Response {
    let label = RichText::new(icon_text(icon, text)).color(Color32::WHITE);
    let button = egui::Button::new(label)
        .fill(color::danger_bg())
        .stroke(Stroke::NONE)
        .min_size(Vec2::new(theme::BUTTON_MIN_WIDTH, theme::CONTROL_HEIGHT));
    ui.add(button)
}

/// A square button showing only an icon, with a tooltip.
pub fn icon_button(ui: &mut Ui, icon: &str, tooltip: &str) -> Response {
    icon_button_enabled(ui, true, icon, tooltip)
}

pub fn icon_button_enabled(ui: &mut Ui, enabled: bool, icon: &str, tooltip: &str) -> Response {
    let glyph = RichText::new(icon).size(theme::GLYPH_SIZE);
    ui.scope(|ui| {
        square_button_padding(ui);
        ui.add_enabled(enabled, egui::Button::new(glyph).min_size(Vec2::splat(theme::CONTROL_HEIGHT)))
    })
    .inner
    .on_hover_text(tooltip)
    .on_disabled_hover_text(tooltip)
}

/// Sets the padding with which a button holding one icon glyph comes out square.
pub fn square_button_padding(ui: &mut Ui) {
    ui.spacing_mut().button_padding.x = (theme::CONTROL_HEIGHT - theme::GLYPH_SIZE) / 2.0;
}

// ---- inputs ----------------------------------------------------------------

/// A single-line text input of the standard control height.
pub fn text_field(text: &mut String) -> egui::TextEdit<'_> {
    egui::TextEdit::singleline(text).margin(Margin::symmetric(FIELD_PADDING as i8, 7)).vertical_align(Align::Center)
}

/// Horizontal padding inside a text field.
const FIELD_PADDING: f32 = 10.0;

/// A text input whose outer width is exactly `width`.
pub fn sized_text_field(text: &mut String, width: f32) -> egui::TextEdit<'_> {
    text_field(text).desired_width(width - 2.0 * FIELD_PADDING)
}

/// An on/off switch. Returns a response that is `changed()` when toggled.
pub fn toggle(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let size = Vec2::new(36.0, 20.0);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), *on, label));

    if ui.is_rect_visible(rect) {
        let how_on = ui.ctx().animate_bool_with_time(response.id, *on, 0.12);
        let track = if *on { color::primary() } else { color::control_hover() };
        let radius = rect.height() / 2.0;
        ui.painter().rect_filled(rect, radius, track);
        if !*on {
            ui.painter().rect_stroke(rect, radius, Stroke::new(1.0, color::control_border()), StrokeKind::Inside);
        }
        let knob_x = egui::lerp((rect.left() + 10.0)..=(rect.right() - 10.0), how_on);
        let knob = if *on { Color32::WHITE } else { color::text_muted() };
        ui.painter().circle_filled(egui::pos2(knob_x, rect.center().y), 7.0, knob);
    }
    response
}

/// One option of a [`segmented`] control.
pub struct Segment<'a> {
    pub icon: &'a str,
    pub label: &'a str,
    pub tooltip: &'a str,
}

/// A row of mutually exclusive options of equal width. Returns the option clicked.
pub fn segmented(ui: &mut Ui, segments: &[Segment<'_>], selected: usize, segment_width: f32) -> Option<usize> {
    let size = Vec2::new(segment_width * segments.len() as f32, theme::CONTROL_HEIGHT);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let radius = CornerRadius::same(theme::RADIUS);
    ui.painter().rect(rect, radius, color::control_bg(), Stroke::new(1.0, color::control_border()), StrokeKind::Inside);

    let mut clicked = None;
    for (index, segment) in segments.iter().enumerate() {
        let segment_rect = Rect::from_min_size(
            rect.min + Vec2::new(segment_width * index as f32, 0.0),
            Vec2::new(segment_width, theme::CONTROL_HEIGHT),
        );
        let is_selected = index == selected;
        let response = ui
            .interact(segment_rect, ui.id().with(("segment", segment.label)), Sense::click())
            .on_hover_text(segment.tooltip);
        response.widget_info(|| WidgetInfo::selected(WidgetType::RadioButton, true, is_selected, segment.label));

        let inner = segment_rect.shrink(3.0);
        let inner_radius = CornerRadius::same(theme::RADIUS - 2);
        if is_selected {
            ui.painter().rect_filled(inner, inner_radius, color::primary());
        } else if response.hovered() {
            ui.painter().rect_filled(inner, inner_radius, color::control_hover());
        }
        let tint = if is_selected { Color32::WHITE } else { color::text_muted() };
        let text =
            if segment.icon.is_empty() { segment.label.to_owned() } else { icon_text(segment.icon, segment.label) };
        ui.painter().text(segment_rect.center(), egui::Align2::CENTER_CENTER, text, theme::body(), tint);
        if response.clicked() && !is_selected {
            clicked = Some(index);
        }
    }
    clicked
}

/// One tab of a [`tab_strip`].
pub struct Tab {
    pub title: String,
    /// Secondary text after the title, e.g. an ID.
    pub subtitle: String,
    /// Disabled tabs can still be opened; they are only shown dimmed.
    pub enabled: bool,
    /// Shows the "staged changes" dot.
    pub dirty: bool,
    pub tooltip: String,
}

/// Tabs of equal width filling the available width. Returns the tab clicked.
pub fn tab_strip(ui: &mut Ui, tabs: &[Tab], selected: usize) -> Option<usize> {
    if tabs.is_empty() {
        return None;
    }
    let gap = space::XS;
    let count = tabs.len() as f32;
    let width = ((ui.available_width() - gap * (count - 1.0)) / count).floor();
    let radius = CornerRadius::same(theme::RADIUS);
    let mut clicked = None;

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for (index, tab) in tabs.iter().enumerate() {
            let is_selected = index == selected;
            let (rect, response) = ui.allocate_exact_size(Vec2::new(width, theme::CONTROL_HEIGHT), Sense::click());
            response.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, is_selected, &tab.title));

            if ui.is_rect_visible(rect) {
                if is_selected {
                    ui.painter().rect(
                        rect,
                        radius,
                        color::accent_subtle(),
                        Stroke::new(1.0, color::border_focus().gamma_multiply(0.55)),
                        StrokeKind::Inside,
                    );
                } else if response.hovered() {
                    ui.painter().rect_filled(rect, radius, color::control_hover());
                } else {
                    ui.painter().rect_stroke(rect, radius, Stroke::new(1.0, color::border()), StrokeKind::Inside);
                }

                let (title_color, subtitle_color) = match (is_selected, tab.enabled) {
                    (true, _) => (color::text(), color::accent_bright()),
                    (false, true) => (color::text(), color::text_muted()),
                    (false, false) => (color::text_disabled(), color::text_disabled()),
                };
                let mut job = LayoutJob::default();
                let title_font = if is_selected { theme::bold(theme::BODY_SIZE) } else { theme::body() };
                job.append(
                    &tab.title,
                    0.0,
                    TextFormat { font_id: title_font, color: title_color, ..Default::default() },
                );
                if !tab.subtitle.is_empty() {
                    let format = TextFormat { font_id: theme::caption(), color: subtitle_color, ..Default::default() };
                    job.append(&tab.subtitle, space::SM, format);
                }
                // The "staged changes" dot sits after the text and is centred together with it.
                let dot_radius = 3.0;
                let dot_space = if tab.dirty { space::SM + 2.0 * dot_radius } else { 0.0 };
                job.wrap = TextWrapping::truncate_at_width(rect.width() - 2.0 * space::SM - dot_space);
                let galley = ui.painter().layout_job(job);
                let text_size = galley.size();
                let left = rect.center().x - (text_size.x + dot_space) / 2.0;
                ui.painter().galley(egui::pos2(left, rect.center().y - text_size.y / 2.0), galley, title_color);
                if tab.dirty {
                    let center = egui::pos2(left + text_size.x + space::SM + dot_radius, rect.center().y);
                    ui.painter().circle_filled(center, dot_radius, color::dirty_mark());
                }
            }

            if response.on_hover_text(&tab.tooltip).clicked() && !is_selected {
                clicked = Some(index);
            }
        }
    });
    clicked
}

// ---- layout helpers --------------------------------------------------------

/// Width of the label column of a form.
pub const FORM_LABEL_WIDTH: f32 = 160.0;

/// A form row: a label in a fixed-width column and a control starting on
/// the shared alignment line.
pub fn form_row<R>(ui: &mut Ui, label: &str, add_control: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let label_size = Vec2::new(FORM_LABEL_WIDTH, theme::CONTROL_HEIGHT);
        ui.allocate_ui_with_layout(label_size, Layout::left_to_right(Align::Center), |ui| {
            ui.set_width(FORM_LABEL_WIDTH);
            ui.label(label);
        });
        ui.add_space(space::LG);
        ui.spacing_mut().item_spacing.x = space::SM;
        add_control(ui)
    })
    .inner
}

/// Indents content to the control column of a form, for help text and
/// messages that belong under a control.
pub fn form_note(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(FORM_LABEL_WIDTH + space::LG);
        ui.vertical(add_contents);
    });
}

/// A section header inside a card or dialog.
pub fn section_title(ui: &mut Ui, text: &str) -> Response {
    ui.add(egui::Label::new(RichText::new(text).font(theme::bold(theme::SECTION_SIZE))).selectable(false))
}

/// Secondary text.
pub fn muted(text: impl Into<String>) -> RichText {
    RichText::new(text).color(color::text_muted())
}

/// Secondary text at caption size.
pub fn caption(text: impl Into<String>) -> RichText {
    RichText::new(text).color(color::text_muted()).size(theme::CAPTION_SIZE)
}

/// A row of exactly the control height, with its content centred vertically.
/// The right-hand content is laid out right to left.
pub fn control_row(ui: &mut Ui, left: impl FnOnce(&mut Ui), right: impl FnOnce(&mut Ui)) {
    let size = Vec2::new(ui.available_width(), theme::CONTROL_HEIGHT);
    ui.allocate_ui_with_layout(size, Layout::left_to_right(Align::Center), |ui| {
        ui.set_min_size(size);
        left(ui);
        ui.with_layout(Layout::right_to_left(Align::Center), right);
    });
}

/// A hairline with the standard gap above and below it.
pub fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.center().y, Stroke::new(1.0, color::border()));
}

// ---- content ---------------------------------------------------------------

/// A game icon with the thin border every icon in the editor has.
pub fn game_icon(ui: &mut Ui, texture: TextureId, size: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_game_icon(ui, texture, rect, Color32::WHITE);
    response
}

/// Paints a game icon into an already allocated rectangle.
pub fn paint_game_icon(ui: &Ui, texture: TextureId, rect: Rect, tint: Color32) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let radius = CornerRadius::same(4);
    egui::Image::from_texture((texture, rect.size())).corner_radius(radius).tint(tint).paint_at(ui, rect);
    ui.painter().rect_stroke(rect, radius, Stroke::new(1.0, color::control_border()), StrokeKind::Inside);
}

/// A small rounded label, e.g. `RND[301]`.
///
/// Sized from its text and painted directly: a frame would stretch to the
/// height of the row it sits in.
pub fn badge(ui: &mut Ui, text: &str, foreground: Color32) -> Response {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), theme::bold(theme::CAPTION_SIZE), foreground);
    let size = galley.size() + Vec2::new(2.0 * space::SM, space::XS);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, text));

    if ui.is_rect_visible(rect) {
        ui.painter().rect(
            rect,
            CornerRadius::same(4),
            foreground.gamma_multiply(0.14),
            Stroke::new(1.0, foreground.gamma_multiply(0.45)),
            StrokeKind::Inside,
        );
        let position = rect.center() - galley.size() / 2.0;
        ui.painter().galley(position, galley, foreground);
    }
    response
}

/// A centred message for empty lists and panels.
pub fn empty_state(ui: &mut Ui, icon: &str, title: &str, hint: &str) {
    ui.vertical_centered(|ui| {
        ui.set_max_width(ui.available_width().min(360.0));
        ui.add_space((ui.available_height() * 0.25).clamp(space::MD, 120.0));
        ui.label(RichText::new(icon).size(32.0).color(color::text_faint()));
        ui.label(RichText::new(title).font(theme::bold(theme::SECTION_SIZE)).color(color::text_muted()));
        if !hint.is_empty() {
            ui.label(RichText::new(hint).color(color::text_faint()));
        }
    });
}

// ---- grids -----------------------------------------------------------------

/// The horizontal extent and clip rectangle of a grid body, captured once
/// so every row can paint a background across all of its cells.
#[derive(Debug, Clone, Copy)]
pub struct RowBand {
    pub x: Rangef,
    pub clip: Rect,
}

impl RowBand {
    /// Paints `fill` across the whole row. Call from the first cell of the row.
    pub fn paint(self, cell: &Ui, fill: Color32) {
        let y = cell.max_rect().y_range().expand(cell.spacing().item_spacing.y / 2.0);
        let rect = Rect::from_x_y_ranges(self.x, y);
        // A cell's own painter is clipped to that cell; the band spans all of them.
        cell.ctx().layer_painter(cell.layer_id()).with_clip_rect(self.clip).rect_filled(rect, 0.0, fill);
    }
}

/// How the content of a grid column is aligned; headers follow their column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnAlign {
    Left,
    Center,
}

/// A grid column header, aligned like the cells below it.
pub fn header_cell(ui: &mut Ui, text: &str, tooltip: &str, align: ColumnAlign) {
    let label = egui::Label::new(caption(text)).selectable(false);
    let response = match align {
        ColumnAlign::Left => ui.add(label),
        ColumnAlign::Center => ui.centered_and_justified(|ui| ui.add(label)).inner,
    };
    if !tooltip.is_empty() {
        response.on_hover_text(tooltip);
    }
}

/// Dimmed text for zero values, so the meaningful numbers of a grid stand out.
pub fn value_color(value: i32, changed: bool) -> Color32 {
    if changed {
        color::dirty_mark()
    } else if value == 0 {
        color::text_faint()
    } else {
        color::text()
    }
}
