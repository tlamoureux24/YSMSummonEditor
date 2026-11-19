//! Summon stat scaling calculator.
//!
//! Model the AurumisRZ server's current-form growth and the original editor's
//! cumulative growth calculation.

use std::collections::BTreeMap;

use rust_decimal::prelude::ToPrimitive;
use rust_decimal::{Decimal, RoundingStrategy};

use crate::model::{STAT_COUNT, Stat, StatScalingSnapshot};

pub const MIN_LEVEL: i32 = 1;
pub const MAX_LEVEL: i32 = 350;
pub const MAX_ENHANCE_STAGE: i32 = 5;
pub const TUNING_LEVELS: [i32; 2] = [150, 200];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrowthModel {
    Legacy,
    Aurumis,
}

impl GrowthModel {
    pub fn max_level(self) -> i32 {
        match self {
            Self::Legacy => MAX_LEVEL,
            Self::Aurumis => 200,
        }
    }

    pub fn stage3_gain(self, coefficient: Decimal) -> i32 {
        let [start, end] = TUNING_LEVELS;
        self.bonus_at(coefficient, 3, end).saturating_sub(self.bonus_at(coefficient, 3, start))
    }

    fn bonus_at(self, coefficient: Decimal, stage: i32, level: i32) -> i32 {
        let factor = self.level_factor(stage, level);
        match self {
            Self::Legacy => truncate(coefficient * factor),
            Self::Aurumis => (coefficient.to_f32().unwrap_or(0.0) * factor.to_f32().unwrap_or(0.0)) as i32,
        }
    }

    fn level_factor(self, stage: i32, level: i32) -> Decimal {
        let config = &STAGE_CONFIGS[(stage - 1) as usize];
        match self {
            Self::Legacy => {
                let elapsed = (level - config.offset).clamp(0, config.max_level);
                let normal = elapsed.min(config.threshold);
                let over = (elapsed - config.threshold).max(0);
                Decimal::from(normal + over * 3) * (Decimal::ONE + Decimal::new(i64::from(stage - 1) * 25, 2))
            }
            // GetSummonLevelBonus applies the current form's row and counts
            // the evolution level itself for forms 2 and 3.
            Self::Aurumis => Decimal::from(if stage == 1 { level } else { (level + 1 - config.offset).max(0) }),
        }
    }

    pub fn calculate(self, input: &ScalingInput<'_>) -> ScalingResult {
        calculate_with_model(input, self)
    }

    pub fn growth_curve(
        self,
        snapshot: &StatScalingSnapshot,
        enhance_amplify: &BTreeMap<i32, Decimal>,
        enhance_stage: i32,
    ) -> Vec<[i32; STAT_COUNT]> {
        (MIN_LEVEL..=self.max_level())
            .map(|level| {
                final_values(self.calculate(&ScalingInput { level, enhance_stage, enhance_amplify, snapshot }))
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage3Preset {
    Light,
    Balanced,
    High,
}

impl Stage3Preset {
    pub const ALL: [Self; 3] = [Self::Light, Self::Balanced, Self::High];

    fn rate(self) -> Decimal {
        Decimal::new(
            match self {
                Self::Light => 6,
                Self::Balanced => 10,
                Self::High => 14,
            },
            2,
        )
    }
}

/// Proposes coefficients for unconfigured Stage 3 stats. Earlier forms'
/// strongest positive growth adds weight to their established strengths.
pub fn suggest_stage3(
    snapshot: &StatScalingSnapshot,
    model: GrowthModel,
    preset: Stage3Preset,
) -> Option<[Decimal; STAT_COUNT]> {
    if snapshot.evolve_type != 3 {
        return None;
    }
    let current = snapshot.stage_bonuses.iter().find(|stage| stage.stage_index == 3)?.values;
    let [start, end] = TUNING_LEVELS;
    let factor = model.level_factor(3, end) - model.level_factor(3, start);
    let proposed = std::array::from_fn(|index| {
        if !current[index].is_zero() {
            return current[index];
        }
        let prior_rate = snapshot
            .stage_bonuses
            .iter()
            .filter(|stage| matches!(stage.stage_index, 1 | 2))
            .map(|stage| stage.values[index])
            .max()
            .unwrap_or_default()
            .max(Decimal::ZERO)
            .min(Decimal::from(10));
        let base = Decimal::from(snapshot.base_stats.values[index].max(0));
        let reference = base + prior_rate * Decimal::from(end - start);
        if reference.is_zero() {
            return Decimal::ZERO;
        }
        let target = (reference * preset.rate()).clamp(Decimal::ONE, Decimal::from(50));
        // MariaDB stores CreatureLevelBonus with two decimal places.
        (target / factor).round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero)
    });
    (proposed != current).then_some(proposed)
}

struct StageConfig {
    /// Summon level at which this growth stage starts.
    offset: i32,
    /// Stage levels past this count triple.
    threshold: i32,
    max_level: i32,
}

const STAGE_CONFIGS: [StageConfig; 3] = [
    StageConfig { offset: 0, threshold: 50, max_level: 60 },
    StageConfig { offset: 50, threshold: 50, max_level: 65 },
    StageConfig { offset: 100, threshold: 200, max_level: 200 },
];

/// First level eligible for each growth stage of the summon.
pub fn growth_stage_start_levels(growth_depth: i32) -> impl Iterator<Item = i32> {
    STAGE_CONFIGS.iter().take(growth_depth.clamp(1, STAGE_CONFIGS.len() as i32) as usize).map(|stage| stage.offset + 1)
}

pub fn aurumis_evolution_level(stage: i32) -> i32 {
    STAGE_CONFIGS[(stage.clamp(1, 3) - 1) as usize].offset.max(MIN_LEVEL)
}

/// Evaluates the form available at this level, with evolution at the first eligible level.
/// The forms are ordered from earliest to latest.
pub fn aurumis_progression_at(
    forms: &[StatScalingSnapshot],
    level: i32,
    enhance_amplify: &BTreeMap<i32, Decimal>,
    enhance_stage: i32,
) -> Option<(i32, ScalingResult)> {
    let form_at = |level| {
        forms.iter().rev().find(|form| level >= aurumis_evolution_level(form.growth_depth)).or_else(|| forms.first())
    };
    let form = form_at(level)?;
    let calculate = |snapshot, level| {
        GrowthModel::Aurumis.calculate(&ScalingInput { level, enhance_stage, enhance_amplify, snapshot })
    };
    let mut current = calculate(form, level);
    let effective_level = current.effective_level;
    if effective_level > MIN_LEVEL
        && let Some(previous) = form_at(effective_level - 1)
        && !std::ptr::eq(form, previous)
    {
        for (row, previous) in current.rows.iter_mut().zip(calculate(previous, effective_level - 1).rows) {
            row.delta = row.final_value.saturating_sub(previous.final_value);
        }
    }
    Some((form.growth_depth, current))
}

pub fn aurumis_progression_curve(
    forms: &[StatScalingSnapshot],
    enhance_amplify: &BTreeMap<i32, Decimal>,
    enhance_stage: i32,
) -> Vec<[i32; STAT_COUNT]> {
    (MIN_LEVEL..=GrowthModel::Aurumis.max_level())
        .filter_map(|level| aurumis_progression_at(forms, level, enhance_amplify, enhance_stage))
        .map(|(_, result)| final_values(result))
        .collect()
}

fn final_values(result: ScalingResult) -> [i32; STAT_COUNT] {
    let mut values = [0; STAT_COUNT];
    for row in result.rows {
        values[row.stat.index()] = row.final_value;
    }
    values
}

pub struct ScalingInput<'a> {
    pub level: i32,
    pub enhance_stage: i32,
    /// `CreatureEnhance.stat_amplify` by enhance level.
    pub enhance_amplify: &'a BTreeMap<i32, Decimal>,
    pub snapshot: &'a StatScalingSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScalingRow {
    pub stat: Stat,
    pub base: i32,
    /// Growth bonus contributed by each evolution stage.
    pub stage: [i32; 3],
    pub bonus: i32,
    pub final_raw: i32,
    /// `final_raw` with the enhance amplifier applied.
    pub final_value: i32,
    /// Gain over the previous level.
    pub delta: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScalingResult {
    pub requested_level: i32,
    pub effective_level: i32,
    pub requested_enhance_stage: i32,
    pub effective_enhance_stage: i32,
    pub stat_amplify: Decimal,
    pub rows: Vec<ScalingRow>,
}

pub fn calculate(input: &ScalingInput<'_>) -> ScalingResult {
    GrowthModel::Legacy.calculate(input)
}

fn calculate_with_model(input: &ScalingInput<'_>, model: GrowthModel) -> ScalingResult {
    let snapshot = input.snapshot;
    let level = input.level.clamp(MIN_LEVEL, model.max_level());
    let previous_level = (level - 1).max(0);
    let enhance_stage = input.enhance_stage.clamp(0, MAX_ENHANCE_STAGE);
    let stat_amplify = input.enhance_amplify.get(&enhance_stage).copied().unwrap_or(Decimal::ZERO);
    let multiplier = Decimal::ONE + stat_amplify;
    let growth_depth = snapshot.growth_depth.clamp(1, 3);

    let rows = Stat::ALL
        .into_iter()
        .map(|stat| {
            let mut stage = [0; 3];
            let mut stage_previous = [0; 3];
            for stage_index in 1..=growth_depth {
                if model == GrowthModel::Aurumis && stage_index != growth_depth {
                    continue;
                }
                let slot = (stage_index - 1) as usize;
                let coefficient = snapshot
                    .stage_bonuses
                    .iter()
                    .find(|b| b.stage_index == stage_index)
                    .map(|b| b.get(stat))
                    .unwrap_or(Decimal::ZERO);
                stage[slot] = model.bonus_at(coefficient, stage_index, level);
                stage_previous[slot] = model.bonus_at(coefficient, stage_index, previous_level);
            }

            let base = snapshot.base_stats.get(stat);
            let bonus: i32 = stage.iter().sum();
            let bonus_previous: i32 = stage_previous.iter().sum();
            let final_raw = base + bonus;
            let (final_value, final_previous) = match model {
                GrowthModel::Legacy => (amplify(final_raw, multiplier), amplify(base + bonus_previous, multiplier)),
                GrowthModel::Aurumis => {
                    let factor = 1.0 + stat_amplify.to_f32().unwrap_or(0.0);
                    let enhanced_base = (base as f32 * factor) as i32;
                    (enhanced_base.saturating_add(bonus), enhanced_base.saturating_add(bonus_previous))
                }
            };

            ScalingRow {
                stat,
                base,
                stage,
                bonus,
                final_raw,
                final_value,
                delta: final_value.saturating_sub(final_previous),
            }
        })
        .collect();

    ScalingResult {
        requested_level: input.level,
        effective_level: level,
        requested_enhance_stage: input.enhance_stage,
        effective_enhance_stage: enhance_stage,
        stat_amplify,
        rows,
    }
}

/// Final stat values for every level `1..=350`, used to plot growth curves.
pub fn growth_curve(
    snapshot: &StatScalingSnapshot,
    enhance_amplify: &BTreeMap<i32, Decimal>,
    enhance_stage: i32,
) -> Vec<[i32; STAT_COUNT]> {
    GrowthModel::Legacy.growth_curve(snapshot, enhance_amplify, enhance_stage)
}

fn amplify(value: i32, multiplier: Decimal) -> i32 {
    truncate(Decimal::from(value) * multiplier)
}

fn truncate(value: Decimal) -> i32 {
    value.trunc().to_i32().unwrap_or(if value.is_sign_negative() { i32::MIN } else { i32::MAX })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{LevelBonusStage, StatBlock};

    fn dec(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    fn no_amplify() -> BTreeMap<i32, Decimal> {
        BTreeMap::from([(0, Decimal::ZERO)])
    }

    /// Snapshot where only STR has a base value and growth coefficients.
    fn snapshot(depth: i32, base_str: i32, str_coefficients: &[(i32, &str)]) -> StatScalingSnapshot {
        let mut base = StatBlock::zero(1);
        base.values[0] = base_str;
        StatScalingSnapshot {
            summon_id: 100,
            stat_id: 1,
            evolve_type: depth,
            growth_depth: depth,
            base_stats: base,
            stage_bonuses: str_coefficients
                .iter()
                .map(|(stage_index, coefficient)| {
                    let mut values = [Decimal::ZERO; STAT_COUNT];
                    values[0] = dec(coefficient);
                    LevelBonusStage { stage_index: *stage_index, lookup_id: 100, values, exists_in_db: true }
                })
                .collect(),
        }
    }

    fn str_row(level: i32, enhance: i32, amplify: &BTreeMap<i32, Decimal>, snap: &StatScalingSnapshot) -> ScalingRow {
        let result =
            calculate(&ScalingInput { level, enhance_stage: enhance, enhance_amplify: amplify, snapshot: snap });
        result.rows.into_iter().find(|r| r.stat == Stat::Str).unwrap()
    }

    #[test]
    fn matches_guide_example_for_str() {
        let snap = snapshot(2, 20, &[(1, "0.30"), (2, "0.50")]);
        let row = str_row(80, 0, &no_amplify(), &snap);

        assert_eq!(row.stage[0], 24);
        assert_eq!(row.stage[1], 18);
        assert_eq!(row.bonus, 42);
        assert_eq!(row.final_raw, 62);
        assert_eq!(row.final_value, 62);
    }

    #[test]
    fn truncates_per_stage() {
        let snap = snapshot(1, 0, &[(1, "0.3333")]);
        assert_eq!(str_row(10, 0, &no_amplify(), &snap).bonus, 3);
    }

    #[test]
    fn missing_stage_bonus_counts_as_zero() {
        let snap = snapshot(3, 10, &[(1, "0.2")]);
        let row = str_row(120, 0, &no_amplify(), &snap);

        assert!(row.stage[0] > 0);
        assert_eq!(row.stage[1], 0);
        assert_eq!(row.stage[2], 0);
    }

    #[test]
    fn clamps_level_to_350() {
        let snap = snapshot(1, 0, &[(1, "1")]);
        let result =
            calculate(&ScalingInput { level: 999, enhance_stage: 0, enhance_amplify: &no_amplify(), snapshot: &snap });
        assert_eq!(result.requested_level, 999);
        assert_eq!(result.effective_level, 350);
    }

    #[test]
    fn delta_is_the_gain_over_the_previous_level() {
        let snap = snapshot(1, 0, &[(1, "1")]);
        let level1 = str_row(1, 0, &no_amplify(), &snap);
        let level2 = str_row(2, 0, &no_amplify(), &snap);

        assert_eq!(level1.delta, 1);
        assert_eq!(level2.delta, level2.final_value - level1.final_value);
    }

    #[test]
    fn applies_enhance_amplify_to_the_final_stat() {
        let snap = snapshot(1, 100, &[(1, "0")]);
        let amplify = BTreeMap::from([(0, Decimal::ZERO), (5, dec("0.1234"))]);
        let result =
            calculate(&ScalingInput { level: 1, enhance_stage: 5, enhance_amplify: &amplify, snapshot: &snap });
        let row = result.rows.iter().find(|r| r.stat == Stat::Str).unwrap();

        assert_eq!(row.final_raw, 100);
        assert_eq!(row.final_value, 112);
        assert_eq!(result.effective_enhance_stage, 5);
        assert_eq!(result.stat_amplify, dec("0.1234"));
    }

    #[test]
    fn levels_past_the_threshold_count_triple() {
        // Stage 1: 50 normal levels + 10 over levels × 3 = 80 effective levels.
        let snap = snapshot(1, 0, &[(1, "1")]);
        assert_eq!(str_row(60, 0, &no_amplify(), &snap).bonus, 80);
        // Stage 1 is capped at 60 stage levels.
        assert_eq!(str_row(300, 0, &no_amplify(), &snap).bonus, 80);
    }

    #[test]
    fn growth_curve_covers_every_level_and_never_decreases() {
        let snap = snapshot(3, 5, &[(1, "0.5"), (2, "0.7"), (3, "0.9")]);
        let curve = growth_curve(&snap, &no_amplify(), 0);

        assert_eq!(curve.len(), MAX_LEVEL as usize);
        assert!(curve.windows(2).all(|pair| pair[1][0] >= pair[0][0]));
    }

    #[test]
    fn aurumis_uses_current_form_rate_and_enhances_base_only() {
        let snap = snapshot(3, 100, &[(1, "1"), (2, "2"), (3, "0.50")]);
        let enhance = BTreeMap::from([(5, dec("0.1"))]);
        let at = |level| {
            GrowthModel::Aurumis
                .calculate(&ScalingInput { level, enhance_stage: 5, enhance_amplify: &enhance, snapshot: &snap })
                .rows
                .remove(0)
        };
        assert_eq!(at(99).stage, [0, 0, 0]);
        assert_eq!(at(150).stage, [0, 0, 25]);
        assert_eq!(at(150).final_value, 135);
        assert_eq!(at(200).final_value, 160);
        assert_eq!(at(350), at(200));
    }

    #[test]
    fn aurumis_progression_uses_each_forms_base_and_shows_both_evolution_jumps() {
        let forms =
            [snapshot(1, 25, &[(1, "0.50")]), snapshot(2, 60, &[(2, "0.50")]), snapshot(3, 110, &[(3, "0.30")])];
        let curve = aurumis_progression_curve(&forms, &no_amplify(), 0);
        let strength = |level: usize| curve[level - 1][Stat::Str.index()];
        assert_eq!(strength(49), 49);
        assert_eq!(strength(50), 60);
        assert_eq!(strength(99), 85);
        assert_eq!(strength(100), 110);
        assert_eq!(strength(200), 140);
        let (form, result) = aurumis_progression_at(&forms, 100, &no_amplify(), 0).unwrap();
        assert_eq!(form, 3);
        assert_eq!(result.rows[Stat::Str.index()].delta, 25);
    }

    #[test]
    fn stage3_presets_are_ordered_bounded_and_preserve_configured_coefficients() {
        let mut snap = snapshot(3, 149, &[(1, "0.23"), (2, "0.20"), (3, "0")]);
        snap.stage_bonuses[2].values[Stat::Vit.index()] = dec("0.44");
        snap.base_stats.values[Stat::Dex.index()] = 6000;
        let [light, balanced, high] =
            Stage3Preset::ALL.map(|preset| suggest_stage3(&snap, GrowthModel::Aurumis, preset).unwrap());
        assert!(light[0] < balanced[0] && balanced[0] < high[0]);
        assert_eq!(balanced[Stat::Vit.index()], dec("0.44"));
        assert_eq!(balanced[Stat::Luk.index()], Decimal::ZERO);
        assert_eq!(GrowthModel::Aurumis.stage3_gain(high[Stat::Dex.index()]), 50);
        assert!(GrowthModel::Aurumis.stage3_gain(balanced[0]) >= 14);
        snap.evolve_type = 2;
        assert!(suggest_stage3(&snap, GrowthModel::Aurumis, Stage3Preset::Balanced).is_none());
    }
}
