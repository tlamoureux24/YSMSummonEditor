//! Base stats, growth bonuses and the live scaling calculator of a summon.
//!
//! The editing and calculator grids share the same rows. The scaling chart
//! spans the width below them.

use std::collections::BTreeMap;

use eframe::egui::{self, Align, Color32, Layout, RichText, Sense, Stroke, Ui, Vec2};
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
use rust_decimal::{Decimal, RoundingStrategy};
use ysm_core::model::{CreatureEnhance, STAT_COUNT, Stat, StatScalingSnapshot};
use ysm_core::scaling::{self, GrowthModel, MAX_ENHANCE_STAGE, MIN_LEVEL, ScalingInput, ScalingResult, Stage3Preset};
use ysm_core::scaling_edit::{ScalingDraft, round_bonus};

use super::DialogContext;
use crate::action::Action;
use crate::icons;
use crate::theme::{self, color, space};
use crate::widgets::{self, modal};

mod chart;

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
const BASE_FIELD_WIDTH: f32 = 80.0;
const BONUS_FIELD_WIDTH: f32 = 80.0;
const RESULT_COLUMN_WIDTH: f32 = 48.0;
/// Space either side of the hairline between two sections.
const SECTION_GAP: f32 = space::LG;

#[derive(Clone, Copy)]
struct Stage3Tuning {
    before: [Decimal; STAT_COUNT],
    applied: [Decimal; STAT_COUNT],
}

pub struct StatScalingDialog {
    summon_name: String,
    draft: ScalingDraft,
    earlier_forms: Vec<StatScalingSnapshot>,
    amplify: BTreeMap<i32, Decimal>,
    model: GrowthModel,
    tuning: Option<Stage3Tuning>,
    level: i32,
    enhance_stage: i32,
    /// Final stat values per level for the chart, recomputed when its inputs change.
    curve: Vec<[i32; STAT_COUNT]>,
    curve_inputs: Option<(StatScalingSnapshot, i32)>,
    chart: chart::StatChart,
}

impl StatScalingDialog {
    /// `db` is the database state, `staged` the same with edits staged in the session applied.
    pub fn new(
        summon_name: String,
        db: &StatScalingSnapshot,
        staged: &StatScalingSnapshot,
        creature_enhance: &[CreatureEnhance],
        model: GrowthModel,
        earlier_forms: Vec<StatScalingSnapshot>,
    ) -> Self {
        Self {
            summon_name,
            draft: ScalingDraft::new(db, staged),
            earlier_forms,
            // Later rows win when an enhance level is listed twice.
            amplify: creature_enhance.iter().map(|e| (e.enhance_level, e.stat_amplify)).collect(),
            model,
            tuning: None,
            level: MIN_LEVEL,
            enhance_stage: 0,
            curve: Vec::new(),
            curve_inputs: None,
            chart: chart::StatChart::new(),
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, id: egui::Id, env: &mut DialogContext<'_>) -> bool {
        let title = format!("Stat Scaling · {}", self.summon_name);
        let mut apply = false;
        let mut cancelled = false;

        let outcome = modal::show(ctx, id, icons::STATS, color::ACCENT_BRIGHT, &title, 1184.0, |ui| {
            // Leave space for the title, dialog padding and the pinned footer.
            let body_height = (ctx.content_rect().height() - 156.0).max(200.0);
            egui::ScrollArea::vertical()
                .id_salt("stat-scaling-body")
                .max_height(body_height)
                .auto_shrink([false, true])
                .show(ui, |ui| self.body(ui, body_height));

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
                    self.tuning = None;
                    ctx.request_repaint();
                }
                self.tune_stage3(ui);
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

    fn body(&mut self, ui: &mut Ui, body_height: f32) {
        let compact = body_height < 600.0;
        let row_height = if compact { 26.0 } else { theme::CONTROL_HEIGHT };
        let stages = self.draft.bonuses.len() as f32;
        let base_width = STAT_COLUMN_WIDTH + space::SM + BASE_FIELD_WIDTH;
        let growth_width = STAT_COLUMN_WIDTH + stages * (space::SM + BONUS_FIELD_WIDTH);
        let dividers = 2.0 * (2.0 * SECTION_GAP + 1.0);
        let calculator_width = (ui.available_width() - base_width - growth_width - dividers)
            .max(STAT_COLUMN_WIDTH + 8.0 * (space::SM + RESULT_COLUMN_WIDTH));
        let old_selection = (self.level, self.enhance_stage);

        let (snapshot, forms) = ui
            .horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                section(ui, base_width, row_height, |ui| self.base_stats(ui));
                section_divider(ui, row_height);
                section(ui, growth_width, row_height, |ui| self.growth_bonuses(ui));
                section_divider(ui, row_height);

                let snapshot = self.draft.snapshot();
                let forms = self.progression_forms(&snapshot);
                let (stage, result) =
                    scaling::aurumis_progression_at(&forms, self.level, &self.amplify, self.enhance_stage).map_or_else(
                        || {
                            (
                                None,
                                self.model.calculate(&ScalingInput {
                                    level: self.level,
                                    enhance_stage: self.enhance_stage,
                                    enhance_amplify: &self.amplify,
                                    snapshot: &snapshot,
                                }),
                            )
                        },
                        |(stage, result)| (Some(stage), result),
                    );
                section(ui, calculator_width, row_height, |ui| self.calculator(ui, stage, &result));
                (snapshot, forms)
            })
            .inner;

        if !compact {
            ui.add_space(space::XS);
        }
        self.chart_controls(ui);
        let stage_markers: Vec<_> = match self.model {
            GrowthModel::Legacy => scaling::growth_stage_start_levels(snapshot.growth_depth)
                .zip(1..)
                .map(|(level, stage)| (stage, level))
                .collect(),
            GrowthModel::Aurumis => forms
                .iter()
                .enumerate()
                .map(|(index, form)| {
                    let stage = form.growth_depth;
                    let start = if index == 0 { MIN_LEVEL } else { scaling::aurumis_evolution_level(stage) };
                    (stage, start)
                })
                .collect(),
        };
        if self.curve_inputs.as_ref().is_none_or(|inputs| inputs.0 != snapshot || inputs.1 != self.enhance_stage) {
            self.curve = match self.model {
                GrowthModel::Legacy => self.model.growth_curve(&snapshot, &self.amplify, self.enhance_stage),
                GrowthModel::Aurumis => scaling::aurumis_progression_curve(&forms, &self.amplify, self.enhance_stage),
            };
            self.curve_inputs = Some((snapshot, self.enhance_stage));
        }
        let legend_space = theme::CONTROL_HEIGHT + 3.0 * space::SM;
        let height = (body_height - ui.min_size().y - legend_space).clamp(144.0, 296.0);
        self.chart.show(
            ui,
            height,
            &self.curve,
            &mut self.level,
            self.model.max_level(),
            &stage_markers,
            self.model == GrowthModel::Aurumis,
        );
        if old_selection != (self.level, self.enhance_stage) {
            ui.ctx().request_repaint();
        }
    }

    fn progression_forms(&self, snapshot: &StatScalingSnapshot) -> Vec<StatScalingSnapshot> {
        if self.model != GrowthModel::Aurumis {
            return Vec::new();
        }
        let mut forms = self.earlier_forms.clone();
        for form in &mut forms {
            if let Some(edited) = snapshot.stage_bonuses.iter().find(|bonus| bonus.lookup_id == form.summon_id)
                && let Some(active) = form.stage_bonuses.iter_mut().find(|bonus| bonus.lookup_id == form.summon_id)
            {
                active.values = edited.values;
            }
        }
        forms.push(snapshot.clone());
        forms
    }

    fn tune_stage3(&mut self, ui: &mut Ui) {
        let mut snapshot = self.draft.snapshot();
        let Some(stage) = snapshot.stage_bonuses.iter_mut().find(|stage| stage.stage_index == 3) else {
            return;
        };
        let current = stage.values;
        let previous = self.tuning.filter(|tuning| tuning.applied == current);
        let before = previous.map_or(current, |tuning| tuning.before);
        stage.values = before;
        let presets = Stage3Preset::ALL.map(|preset| scaling::suggest_stage3(&snapshot, self.model, preset));
        let [Some(light), Some(balanced), Some(high)] = presets else {
            widgets::button_enabled(ui, false, icons::STATS, "Tune Stage 3…")
                .on_disabled_hover_text("There are no unconfigured Stage 3 stats with enough data to tune.");
            return;
        };

        let button = egui::Button::new(widgets::icon_text(icons::STATS, "Tune Stage 3…"));
        let (response, inner) = egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
            ui.set_min_width(360.0);
            widgets::section_title(ui, "Stage 3 growth");
            let [start, end] = scaling::TUNING_LEVELS;
            ui.label(widgets::caption(format!("Points gained from level {start} to {end}")));
            egui::Grid::new("stage-3-presets").num_columns(5).spacing([space::LG, space::XS]).striped(true).show(
                ui,
                |ui| {
                    for title in ["Stat", "Before", "Gentle", "Balanced", "Strong"] {
                        ui.label(widgets::caption(title));
                    }
                    ui.end_row();
                    for stat in Stat::ALL {
                        let index = stat.index();
                        ui.label(RichText::new(stat.label()).color(STAT_COLORS[index]));
                        for values in [before, light, balanced, high] {
                            ui.label(self.model.stage3_gain(values[index]).to_string());
                        }
                        ui.end_row();
                    }
                },
            );
            ui.add_space(space::XS);
            ui.label(widgets::caption("Only zero rates are filled. Each stat uses one rate across the stage."));
            ui.add_space(space::XS);
            ui.horizontal(|ui| {
                let mut selection = None;
                if ui.button("Gentle").clicked() {
                    selection = Some(light);
                }
                if widgets::primary_button(ui, icons::CHECK, "Balanced").clicked() {
                    selection = Some(balanced);
                }
                if ui.button("Strong").clicked() {
                    selection = Some(high);
                }
                if previous.is_some() && ui.button("Restore previous").clicked() {
                    selection = Some(before);
                }
                if selection.is_some() {
                    ui.close();
                }
                selection
            })
            .inner
        });
        response.on_hover_text("Compare and apply growth suggestions to this draft.");
        if let Some(values) = inner.and_then(|selection| selection.inner)
            && let Some(stage) = self.draft.bonuses.iter_mut().find(|stage| stage.stage_index == 3)
        {
            stage.values = values;
            self.tuning = (values != before).then_some(Stage3Tuning { before, applied: values });
            self.level = scaling::TUNING_LEVELS[1];
            ui.ctx().request_repaint();
        }
    }

    fn base_stats(&mut self, ui: &mut Ui) {
        let title = match self.model {
            GrowthModel::Legacy => "Base stats".to_owned(),
            GrowthModel::Aurumis => format!("Stage {} base stats", self.draft.growth_depth()),
        };
        section_heading(ui, &title, &format!("StatResource {}", self.draft.stat_id()));
        stat_grid(ui, "base-stats", 2, |ui| {
            header(ui, STAT_COLUMN_WIDTH, "Stat", "", Align::Min);
            header(ui, BASE_FIELD_WIDTH, "Base", "", Align::Min);
            ui.end_row();

            for stat in Stat::ALL {
                stat_name(ui, stat, self.draft.is_base_stat_dirty(stat));
                let value = &mut self.draft.base[stat.index()];
                let field = egui::DragValue::new(value).speed(0.2).range(0..=1_000_000);
                ui.add_sized([BASE_FIELD_WIDTH, ui.spacing().interact_size.y], field);
                ui.end_row();
            }
        });
    }

    fn growth_bonuses(&mut self, ui: &mut Ui) {
        let (detail, decimals, speed) = match self.model {
            GrowthModel::Legacy => ("CreatureLevelBonus, one row per growth stage", 4, 0.005),
            GrowthModel::Aurumis => ("Each form uses its own bonus row", 2, 0.01),
        };
        section_heading(ui, "Growth bonus per level", detail);
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
                    let field = egui::DragValue::new(&mut number)
                        .speed(speed)
                        .fixed_decimals(decimals)
                        .range(-10_000.0..=10_000.0);
                    if ui.add_sized([BONUS_FIELD_WIDTH, ui.spacing().interact_size.y], field).changed()
                        && let Some(updated) = Decimal::from_f64(number)
                    {
                        *value = if self.model == GrowthModel::Aurumis {
                            updated.round_dp_with_strategy(decimals as u32, RoundingStrategy::MidpointAwayFromZero)
                        } else {
                            round_bonus(updated)
                        };
                    }
                }
                ui.end_row();
            }
        });
    }

    fn calculator(&self, ui: &mut Ui, stage: Option<i32>, result: &ScalingResult) {
        let percent = (result.stat_amplify * Decimal::from(100)).normalize();
        let stage = stage.map_or_else(String::new, |stage| format!("Stage {stage} · "));
        section_heading(
            ui,
            "Calculator",
            &format!(
                "{stage}Level {} · enhance {} · {percent}% {}",
                result.effective_level,
                result.effective_enhance_stage,
                if self.model == GrowthModel::Aurumis { "to base stats" } else { "to total stats" }
            ),
        );

        stat_grid(ui, "scaling-result", 9, |ui| {
            header(ui, STAT_COLUMN_WIDTH, "Stat", "", Align::Min);
            for title in ["Base", "Growth 1", "Growth 2", "Growth 3", "Bonus", "Raw", "Final", "Per level"] {
                let response = header(ui, RESULT_COLUMN_WIDTH, title, "", Align::Max);
                if self.model == GrowthModel::Aurumis && title == "Per level" {
                    response.on_hover_text("Change since the previous level, including any evolution jump.");
                }
            }
            ui.end_row();

            for row in &result.rows {
                stat_name(ui, row.stat, false);
                number(ui, row.base.to_string(), color::TEXT, false);
                for stage in row.stage {
                    number(ui, stage.to_string(), dimmed(stage), false);
                }
                number(ui, row.bonus.to_string(), dimmed(row.bonus), false);
                number(ui, row.final_raw.to_string(), color::TEXT, false);
                number(ui, row.final_value.to_string(), color::TEXT, true);
                let delta_color = if row.delta < 0 {
                    color::DANGER
                } else if row.delta > 0 {
                    color::SUCCESS
                } else {
                    color::TEXT_FAINT
                };
                let delta = if row.delta == 0 { "0".into() } else { format!("{:+}", row.delta) };
                number(ui, delta, delta_color, false);
                ui.end_row();
            }
        });
    }

    fn chart_controls(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.set_height(theme::CONTROL_HEIGHT);
            widgets::section_title(ui, "Final stats by level");
            ui.add_space(space::SM);
            let hint = if self.model == GrowthModel::Aurumis {
                "Evolves at the earliest levels. Click the plot to inspect."
            } else {
                "Click or drag the plot to set the calculator level."
            };
            ui.label(widgets::caption(hint));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(space::MD);
                ui.spacing_mut().slider_width = 96.0;
                ui.add(egui::Slider::new(&mut self.enhance_stage, 0..=MAX_ENHANCE_STAGE));
                ui.label("Enhance");
                ui.add_space(space::MD);
                ui.spacing_mut().slider_width = 136.0;
                ui.add(egui::Slider::new(&mut self.level, MIN_LEVEL..=self.model.max_level()));
                ui.label("Level");
            });
        });
    }
}

/// A section of fixed width whose content runs top to bottom.
fn section(ui: &mut Ui, width: f32, row_height: f32, add_contents: impl FnOnce(&mut Ui)) {
    ui.vertical(|ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing = Vec2::new(space::SM, space::XS);
        ui.spacing_mut().interact_size.y = row_height;
        if row_height < theme::CONTROL_HEIGHT {
            ui.spacing_mut().button_padding.y = space::XS;
        }
        add_contents(ui);
    });
}

/// A vertical hairline between two sections, with equal space on both sides.
fn section_divider(ui: &mut Ui, row_height: f32) {
    let rows = STAT_COUNT as f32 * (row_height + space::XS);
    let height = 44.0 + HEADER_HEIGHT + rows;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(2.0 * SECTION_GAP + 1.0, height), Sense::hover());
    ui.painter().vline(rect.center().x, rect.y_range(), Stroke::new(1.0, color::BORDER));
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
        .spacing([space::SM, space::XS])
        .min_row_height(ui.spacing().interact_size.y)
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
        let title = RichText::new(title).size(theme::CAPTION_SIZE).color(color::TEXT_MUTED);
        ui.add(egui::Label::new(title).extend().selectable(false));
    })
    .response
}

fn stat_name(ui: &mut Ui, stat: Stat, dirty: bool) {
    let tint = if dirty { color::DIRTY_MARK } else { STAT_COLORS[stat.index()] };
    let text = RichText::new(stat.label()).font(theme::bold(theme::BODY_SIZE)).color(tint);
    ui.allocate_ui_with_layout(
        Vec2::new(STAT_COLUMN_WIDTH, ui.spacing().interact_size.y),
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
        Vec2::new(RESULT_COLUMN_WIDTH, ui.spacing().interact_size.y),
        Layout::right_to_left(Align::Center),
        |ui| {
            ui.set_min_width(RESULT_COLUMN_WIDTH);
            ui.add(egui::Label::new(text).selectable(false));
        },
    );
}

fn dimmed(value: i32) -> Color32 {
    if value == 0 { color::TEXT_FAINT } else { color::TEXT }
}

#[cfg(test)]
mod tests {
    use egui::accesskit::Role;
    use egui_kittest::{
        Harness,
        kittest::{By, Queryable},
    };
    use ysm_core::model::{LevelBonusStage, StatBlock};
    use ysm_core::session::EditSession;

    use super::*;
    use crate::textures::TextureStore;

    #[test]
    fn three_stage_chart_can_hover_and_choose_a_level_at_minimum_window_size() {
        let snapshot = StatScalingSnapshot {
            summon_id: 1,
            stat_id: 1,
            evolve_type: 3,
            growth_depth: 3,
            base_stats: StatBlock { id: 1, values: [10; STAT_COUNT] },
            stage_bonuses: (1..=3)
                .map(|stage_index| LevelBonusStage {
                    stage_index,
                    lookup_id: stage_index,
                    values: [Decimal::ONE; STAT_COUNT],
                    exists_in_db: true,
                })
                .collect(),
        };
        let session = EditSession::new();
        let mut textures = None;
        let mut harness = Harness::builder().with_size([1100.0, 700.0]).build_ui_state(
            |ui, dialog: &mut StatScalingDialog| {
                let Some(textures) = textures.as_mut() else {
                    theme::apply(ui.ctx());
                    textures = Some(TextureStore::new(ui.ctx()));
                    return;
                };
                let mut env = DialogContext { session: &session, textures, actions: &mut Vec::new() };
                dialog.show(ui.ctx(), egui::Id::new("test-stat-scaling"), &mut env);
            },
            StatScalingDialog::new("Demo".into(), &snapshot, &snapshot, &[], GrowthModel::Legacy, Vec::new()),
        );
        harness.run_steps(4);
        let chart = harness.get_by_label("Stat progression plot");
        assert!(chart.rect().width() >= 800.0);
        chart.hover();
        harness.run();
        assert_eq!(harness.state().level, MIN_LEVEL);
        assert!(harness.query(By::new().label_contains("Hover · Lv. 17")).is_some());
        harness.get_by_label("Stat progression plot").click();
        harness.run();
        let level = harness.state().level;
        assert!((175..=176).contains(&level));
        assert!(harness.query(By::new().role(Role::ScrollBar)).is_none());
        let value = harness.state().curve[(level - MIN_LEVEL) as usize][Stat::Luk.index()];
        harness.get_by_label(&format!("LUK {value}")).click();
        harness.run();
        assert!(harness.query(By::new().label("LUK").role(Role::Button)).is_some());
    }
}
