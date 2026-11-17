//! Rich rendering of the game's tooltip markup.

use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{self, Color32, FontFamily, FontId, RichText, Stroke, Ui};
use ysm_core::model::SkillDisplay;
use ysm_core::random_group::RandomSkillGroup;
use ysm_core::session::{EditSession, GroupState};
use ysm_core::tooltip;

use crate::theme::{self, color};
use ysm_core::i18n::{tr, trf};

const TOOLTIP_WIDTH: f32 = 460.0;
const BODY_SIZE: f32 = 13.0;

/// Lays out game markup with its colours, weights and sizes.
pub fn layout(markup: &str, wrap_width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap_width;

    for segment in tooltip::parse(markup) {
        let text_color = segment
            .color_hex
            .as_deref()
            .and_then(tooltip::parse_hex_color)
            .map_or(color::text(), |[r, g, b, a]| color::readable(Color32::from_rgba_unmultiplied(r, g, b, a)));
        let size = segment.font_size.map_or(BODY_SIZE, |size| size.clamp(10.0, 26.0));
        let family = if segment.bold { theme::bold_family() } else { FontFamily::Proportional };
        let format = TextFormat {
            font_id: FontId::new(size, family),
            color: text_color,
            italics: segment.italic,
            underline: if segment.underline { Stroke::new(1.0, text_color) } else { Stroke::NONE },
            ..Default::default()
        };
        job.append(&segment.text, 0.0, format);
    }
    job
}

/// Tooltip contents for a skill: name, ID and the game tooltip when there is one.
pub fn skill(ui: &mut Ui, skill_id: i32, display: Option<&SkillDisplay>) {
    ui.set_max_width(TOOLTIP_WIDTH);
    let name = display.map_or_else(|| trf("Skill {}", &[&skill_id]), |d| d.skill_name.clone());
    ui.horizontal(|ui| {
        ui.label(RichText::new(name).font(theme::bold(theme::BODY_SIZE)));
        ui.label(RichText::new(format!("#{skill_id}")).color(color::text_muted()));
    });

    match display.and_then(|d| d.tooltip_markup.as_deref()).filter(|markup| !markup.trim().is_empty()) {
        Some(markup) => {
            ui.separator();
            ui.label(layout(markup, TOOLTIP_WIDTH));
        }
        None if display.is_none() => {
            ui.label(RichText::new(tr("This skill does not exist in SkillResource.")).color(color::warning()));
        }
        None => {}
    }
}

/// Tooltip contents for a random group badge: state and candidate skills.
pub fn random_group(ui: &mut Ui, session: &EditSession, group_id: i32) {
    ui.set_max_width(TOOLTIP_WIDTH);
    ui.label(RichText::new(trf("Random group {}", &[&group_id])).font(theme::bold(theme::BODY_SIZE)));

    let (state_text, tint) = match session.group_state(group_id) {
        GroupState::Pending => (tr("Staged — will be written on save"), color::dirty_mark()),
        GroupState::Loaded => (tr("Loaded from the database"), color::text_muted()),
        GroupState::MissingInDb => (tr("Missing in SummonRandomSkillResource"), color::warning()),
        GroupState::Loading => (tr("Loading…"), color::text_muted()),
    };
    ui.label(RichText::new(state_text).color(tint));

    if let Some(group) = session.overlay_group(group_id) {
        candidates(ui, session, group);
    }
}

fn candidates(ui: &mut Ui, session: &EditSession, group: &RandomSkillGroup) {
    ui.separator();
    ui.label(RichText::new(trf("UseCode: {}", &[&group.use_code()])).color(color::text_muted()));
    if group.skill_ids().is_empty() {
        ui.label(RichText::new(tr("No candidate skills.")).color(color::text_muted()));
        return;
    }
    egui::Grid::new(("group-candidates", group.group_id)).num_columns(2).spacing([10.0, 2.0]).show(ui, |ui| {
        for skill_id in group.skill_ids() {
            ui.label(RichText::new(skill_id.to_string()).color(color::text_muted()));
            ui.label(session.skill_name(*skill_id));
            ui.end_row();
        }
    });
}
