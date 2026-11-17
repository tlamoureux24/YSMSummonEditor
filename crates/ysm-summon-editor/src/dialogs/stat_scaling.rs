//! Base stats, growth bonuses and the live scaling calculator of a summon.
//!
//! Three sections stand side by side. Each has a title, a caption, a header
//! row and one row per stat, all of fixed height, so STR, VIT … LUK run in
//! straight lines across the whole dialog.

use std::collections::BTreeMap;

use eframe::egui::{self, Align, Color32, Layout, RichText, Sense, Stroke, Ui, Vec2};
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
use ysm_core::model::{CreatureEnhance, STAT_COUNT, Stat, StatScalingSnapshot};
use ysm_core::scaling::{self, MAX_ENHANCE_STAGE, MAX_LEVEL, MIN_LEVEL, ScalingInput, ScalingResult};
use ysm_core::scaling_edit::{ScalingDraft, round_bonus};

use super::DialogContext;
use crate::action::Action;
use crate::icons;
use crate::theme::{self, color, space};
use crate::widgets::{self, modal};

/// One colour per stat, used for the stat names and the growth chart.
const STAT_COLORS: [Color32; STAT_COUNT] = [
    Color32::from_rgb(0xF8, 0x71, 0x6B),
    Color32::from_rgb(0xF0, 0x9A, 0x4F),
    Color32::from_rgb(0xE3, 0xC1, 0x4F),
    Color32::from_rgb(0x5F, 0xC9, 0x77),
    Color32::from_rgb(0x56, 0xBF, 0xE8),
    Color32::from_rgb(0xA9, 0x8B, 0xF0),
    Color32::from_rgb(0xE0, 0x8F, 0xC2),
];

/// Height of a section's header row (two lines of caption text).
const HEADER_HEIGHT: f32 = 36.0;
const STAT_COLUMN_WIDTH: f32 = 40.0;
const BASE_FIELD_WIDTH: f32 = 88.0;
const BONUS_FIELD_WIDTH: f32 = 88.0;
const RESULT_COLUMN_WIDTH: f32 = 48.0;
const CHART_HEIGHT: f32 = 136.0;
/// Space either side of the hairline between two sections.
const SECTION_GAP: f32 = space::XL;

pub struct StatScalingDialog {
    summon_name: String,
    draft: ScalingDraft,
    amplify: BTreeMap<i32, Decimal>,
    level: i32,
    enhance_stage: i32,
    /// Final stat values per level for the chart, recomputed when its inputs change.
    curve: Vec<[i32; STAT_COUNT]>,
    curve_inputs: Option<(StatScalingSnapshot, i32)>,
}

impl StatScalingDialog {
    /// `db` is the database state, `staged` the same with edits staged in the session applied.
    pub fn new(
        summon_name: String,
        db: &StatScalingSnapshot,
        staged: &StatScalingSnapshot,
        creature_enhance: &[CreatureEnhance],
    ) -> Self {
        Self {
            summon_name,
            draft: ScalingDraft::new(db, staged),
            // Later rows win when an enhance level is listed twice.
            amplify: creature_enhance.iter().map(|e| (e.enhance_level, e.stat_amplify)).collect(),
            level: MIN_LEVEL,
            enhance_stage: 0,
            curve: Vec::new(),
            curve_inputs: None,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, id: egui::Id, env: &mut DialogContext<'_>) -> bool {
        let title = format!("Stat Scaling · {}", self.summon_name);
        let mut apply = false;
        let mut cancelled = false;

        let outcome = modal::show(ctx, id, icons::STATS, color::accent_bright(), &title, 1184.0, |ui| {
            let snapshot = self.draft.snapshot();
            let result = scaling::calculate(&ScalingInput {
                level: self.level,
                enhance_stage: self.enhance_stage,
                enhance_amplify: &self.amplify,
                snapshot: &snapshot,
            });
            if self.curve_inputs.as_ref() != Some(&(snapshot.clone(), self.enhance_stage)) {
                self.curve = scaling::growth_curve(&snapshot, &self.amplify, self.enhance_stage);
                self.curve_inputs = Some((snapshot, self.enhance_stage));
            }

            let stages = self.draft.bonuses.len() as f32;
            let base_width = STAT_COLUMN_WIDTH + space::MD + BASE_FIELD_WIDTH;
            let growth_width = STAT_COLUMN_WIDTH + stages * (space::MD + BONUS_FIELD_WIDTH);
            let dividers = 2.0 * (2.0 * SECTION_GAP + 1.0);
            let calculator_width = (ui.available_width() - base_width - growth_width - dividers).max(480.0);

            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                section(ui, base_width, |ui| self.base_stats(ui));
                section_divider(ui);
                section(ui, growth_width, |ui| self.growth_bonuses(ui));
                section_divider(ui);
                section(ui, calculator_width, |ui| self.calculator(ui, &result));
            });

            let dirty = self.draft.is_dirty();
            modal::footer(ui, |ui| {
                if widgets::button(ui, icons::CLOSE, "Cancel").clicked() {
                    cancelled = true;
                }
                if widgets::primary_button(ui, icons::CHECK, "Apply").clicked() {
                    apply = true;
                }
                ui.add_space(space::MD);
                if widgets::button_enabled(ui, dirty, icons::DISCARD, "Reset to database").clicked() {
                    self.draft.reset();
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    let text = if dirty { "Values differ from the database." } else { "No changes." };
                    ui.label(widgets::caption(text));
                });
            });
        });

        if apply {
            env.actions.push(Action::ApplyScaling(self.draft.build_result()));
            return false;
        }
        let escaped = outcome.dismissed && ctx.input(|input| input.key_pressed(egui::Key::Escape));
        !(cancelled || escaped)
    }

    fn base_stats(&mut self, ui: &mut Ui) {
        section_heading(ui, "Base stats", &format!("StatResource {}", self.draft.stat_id()));
        stat_grid(ui, "base-stats", 2, |ui| {
            header(ui, STAT_COLUMN_WIDTH, "Stat", "", Align::Min);
            header(ui, BASE_FIELD_WIDTH, "Base", "", Align::Min);
            ui.end_row();

            for stat in Stat::ALL {
                stat_name(ui, stat, self.draft.is_base_stat_dirty(stat));
                let value = &mut self.draft.base[stat.index()];
                let field = egui::DragValue::new(value).speed(0.2).range(0..=1_000_000);
                ui.add_sized([BASE_FIELD_WIDTH, theme::CONTROL_HEIGHT], field);
                ui.end_row();
            }
        });
    }

    fn growth_bonuses(&mut self, ui: &mut Ui) {
        section_heading(ui, "Growth bonus per level", "CreatureLevelBonus, one row per growth stage");
        stat_grid(ui, "growth-bonus", 1 + self.draft.bonuses.len(), |ui| {
            header(ui, STAT_COLUMN_WIDTH, "Stat", "", Align::Min);
            for bonus in &self.draft.bonuses {
                let id = if bonus.exists_in_db {
                    format!("ID {}", bonus.lookup_id)
                } else {
                    format!("ID {} · new", bonus.lookup_id)
                };
                header(ui, BONUS_FIELD_WIDTH, &format!("Stage {}", bonus.stage_index), &id, Align::Min).on_hover_text(
                    if bonus.exists_in_db {
                        "Existing CreatureLevelBonus row."
                    } else {
                        "No row in the database yet; it is created when a value is saved."
                    },
                );
            }
            ui.end_row();

            for stat in Stat::ALL {
                let dirty = self.draft.bonuses.iter().any(|bonus| bonus.is_stat_dirty(stat));
                stat_name(ui, stat, dirty);
                for bonus in &mut self.draft.bonuses {
                    let value = &mut bonus.values[stat.index()];
                    let mut number = value.to_f64().unwrap_or(0.0);
                    let field =
                        egui::DragValue::new(&mut number).speed(0.005).fixed_decimals(4).range(-10_000.0..=10_000.0);
                    if ui.add_sized([BONUS_FIELD_WIDTH, theme::CONTROL_HEIGHT], field).changed()
                        && let Some(updated) = Decimal::from_f64(number)
                    {
                        *value = round_bonus(updated);
                    }
                }
                ui.end_row();
            }
        });
    }

    fn calculator(&mut self, ui: &mut Ui, result: &ScalingResult) {
        let percent = (result.stat_amplify * Decimal::from(100)).normalize();
        section_heading(
            ui,
            "Calculator",
            &format!(
                "Level {} · enhance stage {} · stat amplify +{percent}%",
                result.effective_level, result.effective_enhance_stage
            ),
        );

        stat_grid(ui, "scaling-result", 9, |ui| {
            header(ui, STAT_COLUMN_WIDTH, "Stat", "", Align::Min);
            for title in ["Base", "Growth 1", "Growth 2", "Growth 3", "Bonus", "Raw", "Final", "Per level"] {
                header(ui, RESULT_COLUMN_WIDTH, title, "", Align::Max);
            }
            ui.end_row();

            for row in &result.rows {
                stat_name(ui, row.stat, false);
                number(ui, row.base.to_string(), color::text(), false);
                for stage in row.stage {
                    number(ui, stage.to_string(), dimmed(stage), false);
                }
                number(ui, row.bonus.to_string(), dimmed(row.bonus), false);
                number(ui, row.final_raw.to_string(), color::text(), false);
                number(ui, row.final_value.to_string(), color::text(), true);
                number(ui, format!("+{}", row.delta), color::success(), false);
                ui.end_row();
            }
        });

        ui.add_space(space::SM);
        ui.horizontal(|ui| {
            ui.set_height(theme::CONTROL_HEIGHT);
            ui.spacing_mut().slider_width = 152.0;
            ui.label("Level");
            ui.add(egui::Slider::new(&mut self.level, MIN_LEVEL..=MAX_LEVEL));
            ui.add_space(space::LG);
            ui.label("Enhance");
            ui.add(egui::Slider::new(&mut self.enhance_stage, 0..=MAX_ENHANCE_STAGE));
        });
        self.chart(ui);
    }

    /// Final stat values across all levels, with a marker at the selected level.
    fn chart(&mut self, ui: &mut Ui) {
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), CHART_HEIGHT), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect(
            rect,
            theme::RADIUS,
            color::input_bg(),
            Stroke::new(1.0, color::border()),
            egui::StrokeKind::Inside,
        );

        let plot = rect.shrink(space::MD);
        let max_value = self.curve.iter().flatten().copied().max().unwrap_or(1).max(1) as f32;
        let x_of =
            |level: i32| plot.left() + plot.width() * (level - MIN_LEVEL) as f32 / (MAX_LEVEL - MIN_LEVEL) as f32;
        let y_of = |value: i32| plot.bottom() - plot.height() * value as f32 / max_value;

        // Where the second and third growth stages begin.
        for boundary in [50, 100] {
            painter.vline(x_of(boundary), plot.y_range(), Stroke::new(1.0, color::border()));
        }
        for stat in Stat::ALL {
            let points: Vec<egui::Pos2> = self
                .curve
                .iter()
                .enumerate()
                .map(|(index, values)| egui::pos2(x_of(MIN_LEVEL + index as i32), y_of(values[stat.index()])))
                .collect();
            painter.line(points, Stroke::new(1.5, color::readable(STAT_COLORS[stat.index()])));
        }
        painter.vline(x_of(self.level), plot.y_range(), Stroke::new(1.0, color::text()));
        painter.text(
            plot.left_top(),
            egui::Align2::LEFT_TOP,
            format!("Final stats by level · max {max_value:.0}"),
            theme::caption(),
            color::text_muted(),
        );

        // Clicking or dragging on the chart picks the level.
        if let Some(pointer) = response.interact_pointer_pos() {
            let fraction = ((pointer.x - plot.left()) / plot.width()).clamp(0.0, 1.0);
            self.level = MIN_LEVEL + (fraction * (MAX_LEVEL - MIN_LEVEL) as f32).round() as i32;
        }
        response.on_hover_text("Click or drag to choose the level");
    }
}

/// A section of fixed width whose content runs top to bottom.
fn section(ui: &mut Ui, width: f32, add_contents: impl FnOnce(&mut Ui)) {
    ui.vertical(|ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing = Vec2::new(space::SM, space::XS);
        add_contents(ui);
    });
}

/// A vertical hairline between two sections, with equal space on both sides.
fn section_divider(ui: &mut Ui) {
    let rows = STAT_COUNT as f32 * (theme::CONTROL_HEIGHT + space::XS);
    let height = 44.0 + HEADER_HEIGHT + rows;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(2.0 * SECTION_GAP + 1.0, height), Sense::hover());
    ui.painter().vline(rect.center().x, rect.y_range(), Stroke::new(1.0, color::border()));
}

/// Title and caption of a section; the same height in every section.
fn section_heading(ui: &mut Ui, title: &str, caption: &str) {
    widgets::section_title(ui, title);
    ui.add(egui::Label::new(widgets::caption(caption)).truncate());
    ui.add_space(space::XS);
}

/// A grid whose rows have the standard control height.
fn stat_grid(ui: &mut Ui, id: &str, columns: usize, add_rows: impl FnOnce(&mut Ui)) {
    egui::Grid::new(id)
        .num_columns(columns)
        .spacing([space::MD, space::XS])
        .min_row_height(theme::CONTROL_HEIGHT)
        .show(ui, add_rows);
}

/// A header cell of fixed size: a title and an optional second line.
fn header(ui: &mut Ui, width: f32, title: &str, detail: &str, align: Align) -> egui::Response {
    ui.allocate_ui_with_layout(Vec2::new(width, HEADER_HEIGHT), Layout::bottom_up(align), |ui| {
        ui.set_min_size(Vec2::new(width, HEADER_HEIGHT));
        ui.spacing_mut().item_spacing.y = 0.0;
        // Bottom-up: the detail line is added first so it ends up below the title.
        if !detail.is_empty() {
            ui.add(egui::Label::new(widgets::caption(detail)).extend().selectable(false));
        }
        let title = RichText::new(title).size(theme::CAPTION_SIZE).color(color::text_muted());
        ui.add(egui::Label::new(title).extend().selectable(false));
    })
    .response
}

fn stat_name(ui: &mut Ui, stat: Stat, dirty: bool) {
    let tint = if dirty { color::dirty_mark() } else { color::readable(STAT_COLORS[stat.index()]) };
    let text = RichText::new(stat.label()).font(theme::bold(theme::BODY_SIZE)).color(tint);
    ui.allocate_ui_with_layout(
        Vec2::new(STAT_COLUMN_WIDTH, theme::CONTROL_HEIGHT),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.set_min_width(STAT_COLUMN_WIDTH);
            ui.add(egui::Label::new(text).selectable(false));
        },
    );
}

/// A right-aligned number in a result column.
fn number(ui: &mut Ui, text: String, tint: Color32, emphasized: bool) {
    let text = RichText::new(text).color(tint);
    let text = if emphasized { text.font(theme::bold(theme::BODY_SIZE)) } else { text };
    ui.allocate_ui_with_layout(
        Vec2::new(RESULT_COLUMN_WIDTH, theme::CONTROL_HEIGHT),
        Layout::right_to_left(Align::Center),
        |ui| {
            ui.set_min_width(RESULT_COLUMN_WIDTH);
            ui.add(egui::Label::new(text).selectable(false));
        },
    );
}

fn dimmed(value: i32) -> Color32 {
    if value == 0 { color::text_faint() } else { color::text() }
}
