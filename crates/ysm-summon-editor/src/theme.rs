//! Visual identity: palette, type scale, spacing grid and widget styling.
//!
//! Everything visual is defined here once. Views use these tokens and never
//! invent a colour, a font size or a spacing value of their own:
//!
//! - **Surfaces** are layered: shell < content < control < menu.
//! - **Spacing** comes from a 4 px grid: 4, 8, 12, 16, 20, 24, 32, 40.
//! - **Type** has four sizes: 18 title, 15 section, 13 body, 12 caption.
//! - **Controls** are 32 px high with a 6 px radius; cards and dialogs use 8.

use std::sync::Arc;

use eframe::egui::{
    self, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Stroke, TextStyle, Vec2,
};
use ysm_data::ThemeChoice;

pub mod color {
    use std::sync::atomic::{AtomicBool, Ordering};

    use eframe::egui::Color32;

    /// Whether the light palette is in use; set by [`super::apply`].
    static LIGHT: AtomicBool = AtomicBool::new(false);

    pub(crate) fn set_light(light: bool) {
        LIGHT.store(light, Ordering::Relaxed);
    }

    pub fn is_light() -> bool {
        LIGHT.load(Ordering::Relaxed)
    }

    /// Every colour of the interface, once per theme.
    struct Palette {
        window_bg: Color32,
        panel_bg: Color32,
        control_bg: Color32,
        control_hover: Color32,
        control_pressed: Color32,
        panel_alt: Color32,
        input_bg: Color32,
        border: Color32,
        control_border: Color32,
        border_strong: Color32,
        border_focus: Color32,
        text: Color32,
        text_muted: Color32,
        text_faint: Color32,
        text_disabled: Color32,
        accent_bright: Color32,
        primary: Color32,
        primary_disabled: Color32,
        accent_subtle: Color32,
        row_stripe: Color32,
        row_hover: Color32,
        row_selected: Color32,
        row_dirty: Color32,
        success: Color32,
        warning: Color32,
        danger: Color32,
        danger_bg: Color32,
        info: Color32,
        dirty_mark: Color32,
        hover_tint: Color32,
        on_primary_disabled: Color32,
    }

    /// The original dark palette.
    const DARK: Palette = Palette {
        window_bg: Color32::from_rgb(0x14, 0x16, 0x1B),
        panel_bg: Color32::from_rgb(0x1B, 0x1E, 0x24),
        control_bg: Color32::from_rgb(0x23, 0x27, 0x32),
        control_hover: Color32::from_rgb(0x2A, 0x2F, 0x3B),
        control_pressed: Color32::from_rgb(0x1F, 0x23, 0x2C),
        panel_alt: Color32::from_rgb(0x25, 0x2A, 0x35),
        input_bg: Color32::from_rgb(0x14, 0x16, 0x1B),
        border: Color32::from_rgb(0x2A, 0x2E, 0x38),
        control_border: Color32::from_rgb(0x33, 0x39, 0x47),
        border_strong: Color32::from_rgb(0x41, 0x4A, 0x5C),
        border_focus: Color32::from_rgb(0x5B, 0x94, 0xFF),
        text: Color32::from_rgb(0xE8, 0xEB, 0xF0),
        text_muted: Color32::from_rgb(0xA9, 0xB1, 0xC0),
        text_faint: Color32::from_rgb(0x6C, 0x74, 0x84),
        text_disabled: Color32::from_rgb(0x59, 0x60, 0x72),
        accent_bright: Color32::from_rgb(0x79, 0xA8, 0xFF),
        primary: Color32::from_rgb(0x2F, 0x6F, 0xED),
        primary_disabled: Color32::from_rgb(0x22, 0x30, 0x55),
        accent_subtle: Color32::from_rgba_premultiplied(17, 29, 54, 56),
        row_stripe: Color32::from_rgba_premultiplied(5, 5, 5, 5),
        row_hover: Color32::from_rgb(0x2A, 0x2F, 0x3B),
        row_selected: Color32::from_rgba_premultiplied(25, 41, 77, 80),
        row_dirty: Color32::from_rgba_premultiplied(37, 27, 6, 45),
        success: Color32::from_rgb(0x3F, 0xB9, 0x50),
        warning: Color32::from_rgb(0xD2, 0x99, 0x22),
        danger: Color32::from_rgb(0xF8, 0x51, 0x49),
        danger_bg: Color32::from_rgb(0xB6, 0x23, 0x24),
        info: Color32::from_rgb(0x79, 0xA8, 0xFF),
        dirty_mark: Color32::from_rgb(0xD2, 0x99, 0x22),
        hover_tint: Color32::from_rgba_premultiplied(8, 8, 8, 8),
        on_primary_disabled: Color32::from_rgba_premultiplied(115, 115, 115, 115),
    };

    /// Same roles on light surfaces; text and accents darkened for contrast.
    const LIGHT_PALETTE: Palette = Palette {
        window_bg: Color32::from_rgb(0xE6, 0xE9, 0xEF),
        panel_bg: Color32::from_rgb(0xF7, 0xF8, 0xFA),
        control_bg: Color32::from_rgb(0xFF, 0xFF, 0xFF),
        control_hover: Color32::from_rgb(0xEE, 0xF1, 0xF6),
        control_pressed: Color32::from_rgb(0xE1, 0xE5, 0xEC),
        panel_alt: Color32::from_rgb(0xFF, 0xFF, 0xFF),
        input_bg: Color32::from_rgb(0xEC, 0xEF, 0xF3),
        border: Color32::from_rgb(0xD5, 0xDA, 0xE2),
        control_border: Color32::from_rgb(0xC3, 0xC9, 0xD4),
        border_strong: Color32::from_rgb(0xA3, 0xAB, 0xB9),
        border_focus: Color32::from_rgb(0x2F, 0x6F, 0xED),
        text: Color32::from_rgb(0x1B, 0x1F, 0x27),
        text_muted: Color32::from_rgb(0x4E, 0x56, 0x66),
        text_faint: Color32::from_rgb(0x7C, 0x84, 0x94),
        text_disabled: Color32::from_rgb(0xA0, 0xA7, 0xB4),
        accent_bright: Color32::from_rgb(0x1F, 0x5F, 0xD6),
        primary: Color32::from_rgb(0x2F, 0x6F, 0xED),
        primary_disabled: Color32::from_rgb(0xB9, 0xCB, 0xF2),
        accent_subtle: Color32::from_rgba_premultiplied(6, 13, 28, 31),
        row_stripe: Color32::from_rgba_premultiplied(0, 0, 0, 5),
        row_hover: Color32::from_rgb(0xE8, 0xEC, 0xF3),
        row_selected: Color32::from_rgba_premultiplied(8, 18, 38, 41),
        row_dirty: Color32::from_rgba_premultiplied(29, 21, 5, 36),
        success: Color32::from_rgb(0x1A, 0x7F, 0x37),
        warning: Color32::from_rgb(0x9A, 0x67, 0x00),
        danger: Color32::from_rgb(0xCF, 0x22, 0x2E),
        danger_bg: Color32::from_rgb(0xCF, 0x22, 0x2E),
        info: Color32::from_rgb(0x1F, 0x5F, 0xD6),
        dirty_mark: Color32::from_rgb(0x9A, 0x67, 0x00),
        hover_tint: Color32::from_black_alpha(8),
        on_primary_disabled: Color32::from_rgba_premultiplied(230, 230, 230, 230),
    };

    fn palette() -> &'static Palette {
        if is_light() { &LIGHT_PALETTE } else { &DARK }
    }

    // Surfaces: shell < content < control < menu. Borders, text, accent, rows and semantic colours follow.
    /// Application chrome: toolbar, status bar and the gaps between cards.
    pub fn window_bg() -> Color32 {
        palette().window_bg
    }
    /// Cards and dialogs: the surface the user works on.
    pub fn panel_bg() -> Color32 {
        palette().panel_bg
    }
    /// Inputs and secondary buttons at rest.
    pub fn control_bg() -> Color32 {
        palette().control_bg
    }
    pub fn control_hover() -> Color32 {
        palette().control_hover
    }
    pub fn control_pressed() -> Color32 {
        palette().control_pressed
    }
    /// Menus, popovers, tooltips and toasts.
    pub fn panel_alt() -> Color32 {
        palette().panel_alt
    }
    /// Recessed areas inside a card or dialog: lists, code, charts.
    pub fn input_bg() -> Color32 {
        palette().input_bg
    }
    /// Hairlines: separators and card outlines.
    pub fn border() -> Color32 {
        palette().border
    }
    /// Control outlines at rest.
    pub fn control_border() -> Color32 {
        palette().control_border
    }
    pub fn border_strong() -> Color32 {
        palette().border_strong
    }
    pub fn border_focus() -> Color32 {
        palette().border_focus
    }
    pub fn text() -> Color32 {
        palette().text
    }
    /// Secondary text: captions, help, inactive items.
    pub fn text_muted() -> Color32 {
        palette().text_muted
    }
    /// Placeholders and de-emphasised values such as zeros in a grid.
    pub fn text_faint() -> Color32 {
        palette().text_faint
    }
    pub fn text_disabled() -> Color32 {
        palette().text_disabled
    }
    /// Links, active tabs and selected items.
    pub fn accent_bright() -> Color32 {
        palette().accent_bright
    }
    pub fn primary() -> Color32 {
        palette().primary
    }
    pub fn primary_disabled() -> Color32 {
        palette().primary_disabled
    }
    /// Fill of the active tab or segment.
    pub fn accent_subtle() -> Color32 {
        palette().accent_subtle
    }
    pub fn row_stripe() -> Color32 {
        palette().row_stripe
    }
    pub fn row_hover() -> Color32 {
        palette().row_hover
    }
    pub fn row_selected() -> Color32 {
        palette().row_selected
    }
    /// A row with staged changes.
    pub fn row_dirty() -> Color32 {
        palette().row_dirty
    }
    pub fn success() -> Color32 {
        palette().success
    }
    pub fn warning() -> Color32 {
        palette().warning
    }
    pub fn danger() -> Color32 {
        palette().danger
    }
    /// Fill of destructive buttons; dark enough for white text.
    pub fn danger_bg() -> Color32 {
        palette().danger_bg
    }
    pub fn info() -> Color32 {
        palette().info
    }
    /// Values and markers of staged, unsaved edits.
    pub fn dirty_mark() -> Color32 {
        palette().dirty_mark
    }
    /// Hover tint of grid rows, over any surface.
    pub fn hover_tint() -> Color32 {
        palette().hover_tint
    }
    /// Text of a disabled primary button.
    pub fn on_primary_disabled() -> Color32 {
        palette().on_primary_disabled
    }

    /// A colour from the game data made readable on the current surfaces.
    ///
    /// Game strings colour their text for the game's dark tooltips. On the
    /// light palette a colour too pale against the panel is darkened, keeping
    /// its hue, until it reaches a 4.5:1 contrast ratio.
    pub fn readable(color: Color32) -> Color32 {
        if !is_light() {
            return color;
        }
        let background = panel_alt();
        let mut adjusted = color;
        for _ in 0..20 {
            if contrast(adjusted, background) >= 4.5 {
                break;
            }
            let [r, g, b, a] = adjusted.to_srgba_unmultiplied();
            let darker = |c: u8| (f32::from(c) * 0.85) as u8;
            adjusted = Color32::from_rgba_unmultiplied(darker(r), darker(g), darker(b), a);
        }
        adjusted
    }

    /// WCAG contrast ratio of two opaque colours.
    pub fn contrast(a: Color32, b: Color32) -> f32 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    fn luminance(color: Color32) -> f32 {
        let channel = |c: u8| {
            let c = f32::from(c) / 255.0;
            if c <= 0.039_28 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        let [r, g, b, _] = color.to_srgba_unmultiplied();
        0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
    }
}

/// Spacing grid. No other spacing values are used anywhere.
pub mod space {
    pub const XS: f32 = 4.0;
    pub const SM: f32 = 8.0;
    pub const MD: f32 = 12.0;
    pub const LG: f32 = 16.0;
    pub const XL: f32 = 24.0;
}

/// Height of every button, input and tab.
pub const CONTROL_HEIGHT: f32 = 32.0;
/// Minimum width of a text button.
pub const BUTTON_MIN_WIDTH: f32 = 88.0;
pub const RADIUS: u8 = 6;
pub const CARD_RADIUS: u8 = 8;
/// Game icons are 36 px squares.
pub const ICON_SIZE: f32 = 36.0;
/// Height of grid rows that carry an icon.
pub const ROW_HEIGHT: f32 = 40.0;
/// Height of list rows that carry an icon and two lines of text.
pub const LIST_ROW_HEIGHT: f32 = 48.0;
/// Height of a grid header row.
pub const HEADER_HEIGHT: f32 = 28.0;

pub const TITLE_SIZE: f32 = 18.0;
pub const SECTION_SIZE: f32 = 15.0;
pub const BODY_SIZE: f32 = 13.0;
pub const CAPTION_SIZE: f32 = 12.0;
/// Size of icon glyphs inside controls.
pub const GLYPH_SIZE: f32 = 16.0;

/// Font family used for emphasis; falls back to the regular face when no bold font is installed.
pub fn bold_family() -> FontFamily {
    FontFamily::Name("bold".into())
}

pub fn bold(size: f32) -> FontId {
    FontId::new(size, bold_family())
}

pub fn body() -> FontId {
    FontId::proportional(BODY_SIZE)
}

pub fn caption() -> FontId {
    FontId::proportional(CAPTION_SIZE)
}

/// A rounded container for a section of the window.
pub fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(color::panel_bg())
        .stroke(Stroke::new(1.0, color::border()))
        .corner_radius(CornerRadius::same(CARD_RADIUS))
        .inner_margin(Margin::same(space::MD as i8))
}

/// A recessed area inside a card or dialog.
pub fn well() -> egui::Frame {
    egui::Frame::new()
        .fill(color::input_bg())
        .stroke(Stroke::new(1.0, color::border()))
        .corner_radius(CornerRadius::same(RADIUS))
        .inner_margin(Margin::same(space::XS as i8))
}

/// Applies fonts and the dark style to the whole application; [`follow`]
/// then switches to the configured theme.
pub fn apply(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    set_light(ctx, false);
}

/// Switches palette and style when the configured theme, or the system's
/// when the choice is [`ThemeChoice::System`], no longer matches. Cheap
/// enough to call every frame.
pub fn follow(ctx: &egui::Context, choice: ThemeChoice) {
    let light = match choice {
        ThemeChoice::Dark => false,
        ThemeChoice::Light => true,
        ThemeChoice::System => ctx.system_theme() == Some(egui::Theme::Light),
    };
    if light != color::is_light() {
        set_light(ctx, light);
    }
}

fn set_light(ctx: &egui::Context, light: bool) {
    color::set_light(light);
    ctx.set_theme(if light { egui::Theme::Light } else { egui::Theme::Dark });
    ctx.all_styles_mut(apply_style);
}

fn apply_style(style: &mut egui::Style) {
    style.text_styles = [
        (TextStyle::Small, FontId::new(CAPTION_SIZE, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(BODY_SIZE, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(BODY_SIZE, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(BODY_SIZE, FontFamily::Monospace)),
        (TextStyle::Heading, bold(TITLE_SIZE)),
    ]
    .into();

    let spacing = &mut style.spacing;
    spacing.item_spacing = Vec2::splat(space::SM);
    spacing.button_padding = Vec2::new(space::MD, 6.0);
    spacing.interact_size = Vec2::new(40.0, CONTROL_HEIGHT);
    spacing.window_margin = Margin::same(space::XL as i8);
    spacing.menu_margin = Margin::same(space::XS as i8);
    spacing.icon_width = 16.0;
    spacing.icon_spacing = space::SM;
    spacing.scroll = egui::style::ScrollStyle::floating();
    spacing.scroll.bar_width = 8.0;
    spacing.scroll.dormant_handle_opacity = 0.5;
    spacing.scroll.active_handle_opacity = 0.7;
    spacing.scroll.interact_handle_opacity = 1.0;

    let radius = CornerRadius::same(RADIUS);
    let visuals = &mut style.visuals;
    *visuals = if color::is_light() { egui::Visuals::light() } else { egui::Visuals::dark() };
    visuals.override_text_color = Some(color::text());
    visuals.weak_text_color = Some(color::text_muted());
    visuals.panel_fill = color::window_bg();
    visuals.window_fill = color::panel_bg();
    visuals.window_stroke = Stroke::new(1.0, color::control_border());
    visuals.window_corner_radius = CornerRadius::same(CARD_RADIUS);
    visuals.menu_corner_radius = CornerRadius::same(CARD_RADIUS);
    visuals.extreme_bg_color = color::control_bg();
    visuals.text_edit_bg_color = Some(color::control_bg());
    visuals.faint_bg_color = color::row_stripe();
    visuals.code_bg_color = color::input_bg();
    visuals.hyperlink_color = color::accent_bright();
    visuals.warn_fg_color = color::warning();
    visuals.error_fg_color = color::danger();
    visuals.selection.bg_fill = color::row_selected();
    visuals.selection.stroke = Stroke::new(1.0, color::border_focus());

    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = color::panel_bg();
    widgets.noninteractive.weak_bg_fill = color::panel_bg();
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, color::border());
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, color::text());
    widgets.noninteractive.corner_radius = radius;

    widgets.inactive.bg_fill = color::control_bg();
    widgets.inactive.weak_bg_fill = color::control_bg();
    widgets.inactive.bg_stroke = Stroke::new(1.0, color::control_border());
    widgets.inactive.fg_stroke = Stroke::new(1.0, color::text());
    widgets.inactive.corner_radius = radius;
    widgets.inactive.expansion = 0.0;

    widgets.hovered.bg_fill = color::control_hover();
    widgets.hovered.weak_bg_fill = color::control_hover();
    widgets.hovered.bg_stroke = Stroke::new(1.0, color::border_strong());
    widgets.hovered.fg_stroke = Stroke::new(1.0, color::text());
    widgets.hovered.corner_radius = radius;
    widgets.hovered.expansion = 0.0;

    widgets.active.bg_fill = color::control_pressed();
    widgets.active.weak_bg_fill = color::control_pressed();
    widgets.active.bg_stroke = Stroke::new(1.0, color::border_focus());
    widgets.active.fg_stroke = Stroke::new(1.0, color::text());
    widgets.active.corner_radius = radius;
    widgets.active.expansion = 0.0;

    widgets.open.bg_fill = color::control_hover();
    widgets.open.weak_bg_fill = color::control_hover();
    widgets.open.bg_stroke = Stroke::new(1.0, color::border_strong());
    widgets.open.fg_stroke = Stroke::new(1.0, color::text());
    widgets.open.corner_radius = radius;
    widgets.open.expansion = 0.0;
}

/// egui's default fonts, preceded by Segoe UI when available, with Phosphor
/// icons and a Korean-capable fallback for game strings.
fn fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);

    let system_font = |file: &str| {
        let windows = std::env::var_os("WINDIR").map(std::path::PathBuf::from)?;
        std::fs::read(windows.join("Fonts").join(file)).ok()
    };

    if let Some(bytes) = system_font("segoeui.ttf") {
        fonts.font_data.insert("segoe".into(), Arc::new(FontData::from_owned(bytes)));
        fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "segoe".into());
    }

    // The bold family is the bold face followed by everything the regular family has.
    let mut bold_chain = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    if let Some(bytes) = system_font("seguisb.ttf").or_else(|| system_font("segoeuib.ttf")) {
        fonts.font_data.insert("segoe-bold".into(), Arc::new(FontData::from_owned(bytes)));
        bold_chain.insert(0, "segoe-bold".into());
    }
    fonts.families.insert(bold_family(), bold_chain);

    if let Some(bytes) = system_font("malgun.ttf") {
        fonts.font_data.insert("malgun".into(), Arc::new(FontData::from_owned(bytes)));
        for family in fonts.families.values_mut() {
            family.push("malgun".into());
        }
    }

    fonts
}

/// The window icon, decoded from the embedded `.ico`.
pub fn window_icon() -> Option<egui::IconData> {
    let image = image::load_from_memory(include_bytes!("../../../assets/Logo.ico")).ok()?.into_rgba8();
    let (width, height) = image.dimensions();
    Some(egui::IconData { rgba: image.into_raw(), width, height })
}

#[cfg(test)]
mod tests {
    use eframe::egui::Color32;

    use super::color;

    #[test]
    fn game_colours_stay_readable_on_the_light_palette() {
        let pale = [Color32::from_rgb(0xFF, 0xFF, 0x00), Color32::from_rgb(0xF8, 0x71, 0x6B), Color32::WHITE];
        color::set_light(true);
        let background = color::panel_alt();
        for colour in pale {
            let shown = color::readable(colour);
            assert!(color::contrast(shown, background) >= 4.5, "{colour:?} -> {shown:?}");
        }
        // A colour already dark enough is left alone.
        let dark_red = Color32::from_rgb(0x80, 0x00, 0x00);
        assert_eq!(color::readable(dark_red), dark_red);
        // Text and accents of the light palette pass the same bar.
        for text in [color::text(), color::text_muted(), color::accent_bright(), color::danger(), color::success()] {
            assert!(color::contrast(text, color::panel_bg()) >= 4.5, "{text:?}");
        }
        color::set_light(false);
        assert_eq!(color::readable(Color32::WHITE), Color32::WHITE, "dark theme keeps game colours");
        assert_eq!(color::text(), Color32::from_rgb(0xE8, 0xEB, 0xF0), "dark palette unchanged");
    }
}
