//! Working copy of a summon's base stats and growth bonuses.
//!
//! The stat editor mutates a [`ScalingDraft`]; applying it yields a
//! [`ScalingEditResult`] that the edit session stages. Dirtiness is always
//! measured against the database values, so reopening the editor on top of
//! staged edits never loses them.

use rust_decimal::{Decimal, RoundingStrategy};

use crate::commit::{LevelBonusUpsert, StatCommit};
use crate::model::{LevelBonusStage, STAT_COUNT, Stat, StatBlock, StatScalingSnapshot};

/// Growth coefficients are stored with four decimal places.
pub const BONUS_DECIMALS: u32 = 4;

pub fn round_bonus(value: Decimal) -> Decimal {
    value.round_dp_with_strategy(BONUS_DECIMALS, RoundingStrategy::MidpointAwayFromZero)
}

/// A staged `StatResource` edit together with the values it replaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatEdit {
    pub commit: StatCommit,
    pub original: StatBlock,
}

/// A staged `CreatureLevelBonus` edit together with the values it replaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelBonusEdit {
    pub upsert: LevelBonusUpsert,
    pub original: [Decimal; STAT_COUNT],
    pub exists_in_db: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScalingEditResult {
    pub summon_id: i32,
    /// `None` when the base stats match the database.
    pub stat: Option<StatEdit>,
    /// Every bonus row the editor showed; staged edits for these are replaced.
    pub lookup_ids: Vec<i32>,
    pub bonuses: Vec<LevelBonusEdit>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BonusDraft {
    pub stage_index: i32,
    pub lookup_id: i32,
    pub exists_in_db: bool,
    pub original: [Decimal; STAT_COUNT],
    pub values: [Decimal; STAT_COUNT],
}

impl BonusDraft {
    pub fn is_stat_dirty(&self, stat: Stat) -> bool {
        round_bonus(self.values[stat.index()]) != round_bonus(self.original[stat.index()])
    }

    pub fn is_dirty(&self) -> bool {
        Stat::ALL.into_iter().any(|stat| self.is_stat_dirty(stat))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScalingDraft {
    db: StatScalingSnapshot,
    pub base: [i32; STAT_COUNT],
    pub bonuses: Vec<BonusDraft>,
}

impl ScalingDraft {
    /// `db` is the database state; `staged` is the same snapshot with edits
    /// already staged in the session applied (pass `db` again when there are none).
    pub fn new(db: &StatScalingSnapshot, staged: &StatScalingSnapshot) -> Self {
        let mut stages: Vec<&LevelBonusStage> = db.stage_bonuses.iter().collect();
        stages.sort_by_key(|stage| stage.stage_index);

        let bonuses = stages
            .into_iter()
            .map(|stage| BonusDraft {
                stage_index: stage.stage_index,
                lookup_id: stage.lookup_id,
                exists_in_db: stage.exists_in_db,
                original: stage.values,
                values: staged
                    .stage_bonuses
                    .iter()
                    .find(|s| s.lookup_id == stage.lookup_id)
                    .map_or(stage.values, |s| s.values),
            })
            .collect();

        Self { db: db.clone(), base: staged.base_stats.values, bonuses }
    }

    pub fn summon_id(&self) -> i32 {
        self.db.summon_id
    }

    pub fn stat_id(&self) -> i32 {
        self.db.stat_id
    }

    pub fn growth_depth(&self) -> i32 {
        self.db.growth_depth
    }

    pub fn is_base_stat_dirty(&self, stat: Stat) -> bool {
        self.base[stat.index()] != self.db.base_stats.get(stat)
    }

    pub fn is_base_dirty(&self) -> bool {
        Stat::ALL.into_iter().any(|stat| self.is_base_stat_dirty(stat))
    }

    pub fn is_dirty(&self) -> bool {
        self.is_base_dirty() || self.bonuses.iter().any(BonusDraft::is_dirty)
    }

    /// Restores every value to the database state.
    pub fn reset(&mut self) {
        self.base = self.db.base_stats.values;
        for bonus in &mut self.bonuses {
            bonus.values = bonus.original;
        }
    }

    /// The draft as a snapshot, for feeding the live calculator.
    pub fn snapshot(&self) -> StatScalingSnapshot {
        StatScalingSnapshot {
            base_stats: StatBlock { id: self.db.stat_id, values: self.base },
            stage_bonuses: self
                .bonuses
                .iter()
                .map(|bonus| LevelBonusStage {
                    stage_index: bonus.stage_index,
                    lookup_id: bonus.lookup_id,
                    values: bonus.values.map(round_bonus),
                    exists_in_db: bonus.exists_in_db,
                })
                .collect(),
            ..self.db.clone()
        }
    }

    pub fn build_result(&self) -> ScalingEditResult {
        let stat = self.is_base_dirty().then_some(StatEdit {
            commit: StatCommit {
                summon_id: self.db.summon_id,
                original_stat_id: self.db.stat_id,
                stats: StatBlock { id: self.db.stat_id, values: self.base },
                clone_if_shared: false,
            },
            original: self.db.base_stats,
        });

        let bonuses = self
            .bonuses
            .iter()
            .filter(|bonus| bonus.is_dirty())
            .map(|bonus| LevelBonusEdit {
                upsert: LevelBonusUpsert { bonus_id: bonus.lookup_id, values: bonus.values.map(round_bonus) },
                original: bonus.original.map(round_bonus),
                exists_in_db: bonus.exists_in_db,
            })
            .collect();

        ScalingEditResult {
            summon_id: self.db.summon_id,
            stat,
            lookup_ids: self.bonuses.iter().map(|bonus| bonus.lookup_id).collect(),
            bonuses,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::EditSession;

    fn dec(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    fn snapshot() -> StatScalingSnapshot {
        let stage = |stage_index, lookup_id, value: &str, exists_in_db| LevelBonusStage {
            stage_index,
            lookup_id,
            values: [dec(value); STAT_COUNT],
            exists_in_db,
        };
        StatScalingSnapshot {
            max_level: None,
            summon_id: 3,
            stat_id: 90,
            evolve_type: 2,
            growth_depth: 2,
            base_stats: StatBlock { id: 90, values: [10, 11, 12, 13, 14, 15, 16] },
            stage_bonuses: vec![stage(2, 3, "0.5", true), stage(1, 2, "0.3", false)],
        }
    }

    #[test]
    fn untouched_draft_produces_no_edits() {
        let db = snapshot();
        let draft = ScalingDraft::new(&db, &db);
        let result = draft.build_result();

        assert!(!draft.is_dirty());
        assert_eq!(result.stat, None);
        assert!(result.bonuses.is_empty());
        assert_eq!(result.lookup_ids, [2, 3], "bonus rows are ordered by stage");
    }

    #[test]
    fn base_edit_yields_a_stat_commit_with_originals() {
        let db = snapshot();
        let mut draft = ScalingDraft::new(&db, &db);
        draft.base[Stat::Str.index()] = 99;

        let stat = draft.build_result().stat.unwrap();
        assert_eq!(stat.commit.stats.values[0], 99);
        assert_eq!(stat.commit.original_stat_id, 90);
        assert_eq!(stat.commit.summon_id, 3);
        assert!(!stat.commit.clone_if_shared);
        assert_eq!(stat.original.values[0], 10);
        assert!(draft.is_base_stat_dirty(Stat::Str));
        assert!(!draft.is_base_stat_dirty(Stat::Vit));
    }

    #[test]
    fn only_changed_bonus_rows_are_upserted_and_values_are_rounded() {
        let db = snapshot();
        let mut draft = ScalingDraft::new(&db, &db);
        draft.bonuses[0].values[Stat::Luk.index()] = dec("0.123456");

        let result = draft.build_result();
        assert_eq!(result.bonuses.len(), 1);
        let edit = &result.bonuses[0];
        assert_eq!(edit.upsert.bonus_id, 2);
        assert_eq!(edit.upsert.values[Stat::Luk.index()], dec("0.1235"));
        assert_eq!(edit.original[Stat::Luk.index()], dec("0.3"));
        assert!(!edit.exists_in_db);
    }

    #[test]
    fn sub_precision_changes_are_not_edits() {
        let db = snapshot();
        let mut draft = ScalingDraft::new(&db, &db);
        draft.bonuses[0].values[0] = dec("0.30004");
        assert!(!draft.is_dirty());
    }

    #[test]
    fn reopening_on_staged_edits_keeps_them() {
        let db = snapshot();
        let mut session = EditSession::new();

        let mut first = ScalingDraft::new(&db, &session.pending_scaling(&db));
        first.base[0] = 50;
        first.bonuses[1].values[0] = dec("0.9");
        session.apply_scaling_edit(first.build_result());

        // Reopen and apply without touching anything: the staged edits survive.
        let second = ScalingDraft::new(&db, &session.pending_scaling(&db));
        assert_eq!(second.base[0], 50);
        assert!(second.is_dirty());
        session.apply_scaling_edit(second.build_result());

        let request = session.build_commit_request();
        assert_eq!(request.stats.len(), 1);
        assert_eq!(request.stats[0].stats.values[0], 50);
        assert_eq!(request.level_bonuses.len(), 1);
        assert_eq!(request.level_bonuses[0].bonus_id, 3);
    }

    #[test]
    fn resetting_and_applying_unstages_everything() {
        let db = snapshot();
        let mut session = EditSession::new();
        let mut draft = ScalingDraft::new(&db, &db);
        draft.base[0] = 50;
        draft.bonuses[0].values[0] = dec("2");
        session.apply_scaling_edit(draft.build_result());
        assert!(session.has_changes());

        let mut reopened = ScalingDraft::new(&db, &session.pending_scaling(&db));
        reopened.reset();
        session.apply_scaling_edit(reopened.build_result());
        assert!(!session.has_changes());
    }

    #[test]
    fn snapshot_reflects_the_draft() {
        let db = snapshot();
        let mut draft = ScalingDraft::new(&db, &db);
        draft.base[1] = 77;
        let snap = draft.snapshot();

        assert_eq!(snap.base_stats.values[1], 77);
        assert_eq!(snap.growth_depth, 2);
        assert_eq!(snap.stage_bonuses[0].stage_index, 1);
    }
}
