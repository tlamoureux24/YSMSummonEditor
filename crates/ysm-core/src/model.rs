//! Domain records shared by the data layer, the edit session and the UI.

use crate::i18n::tr;
use rust_decimal::Decimal;

/// Every summon has exactly five skill tree slots.
pub const TREE_SLOTS: usize = 5;
/// The editor always works on a fixed three-stage evolution chain.
pub const STAGE_COUNT: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummonListItem {
    pub summon_id: i32,
    pub summon_name: String,
    pub card_name: Option<String>,
    pub card_id: i32,
    pub evolve_target: i32,
    pub form: i32,
    pub is_riding_only: bool,
    pub face_file_name: Option<String>,
    pub illust_file_name: Option<String>,
    pub feature_text: Option<String>,
    pub stat_id: i32,
    /// `skill_tree1_id..skill_tree5_id`; `0` means "no tree in this slot".
    pub skill_tree_ids: [i32; TREE_SLOTS],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummonListFilter {
    pub search_text: String,
    pub include_riding_only: bool,
    pub include_non_form1: bool,
    pub show_all_evolutions: bool,
}

impl Default for SummonListFilter {
    /// Matches what the original editor always queried with.
    fn default() -> Self {
        Self {
            search_text: String::new(),
            include_riding_only: false,
            include_non_form1: true,
            show_all_evolutions: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvolutionChain {
    pub stage1: Option<SummonListItem>,
    pub stage2: Option<SummonListItem>,
    pub stage3: SummonListItem,
}

impl EvolutionChain {
    /// Stage by 1-based index.
    pub fn stage(&self, stage: u8) -> Option<&SummonListItem> {
        match stage {
            1 => self.stage1.as_ref(),
            2 => self.stage2.as_ref(),
            3 => Some(&self.stage3),
            _ => None,
        }
    }

    /// Existing stages in evolution order.
    pub fn ordered(&self) -> Vec<&SummonListItem> {
        (1..=STAGE_COUNT as u8).filter_map(|s| self.stage(s)).collect()
    }

    pub fn summon_ids(&self) -> Vec<i32> {
        self.ordered().iter().map(|s| s.summon_id).collect()
    }
}

/// One row of `SkillTreeResource`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SkillTreeEntry {
    pub skill_tree_id: i32,
    pub skill_id: i32,
    pub skill_group_id: i32,
    pub min_skill_lv: i32,
    pub max_skill_lv: i32,
    pub lv: i32,
    pub job_lv: i32,
    /// Kept when the editor replaces an entire tree in the database.
    pub jp_ratio: Decimal,
    pub need_skill_id: [i32; 3],
    pub need_skill_lv: [i32; 3],
    pub cenhance_min: i32,
    pub cenhance_max: i32,
}

impl SkillTreeEntry {
    /// Defaults used when the user adds a fresh row to a tree.
    pub fn new_row(skill_tree_id: i32, lv: i32) -> Self {
        Self {
            skill_tree_id,
            skill_id: 0,
            skill_group_id: 0,
            min_skill_lv: 1,
            max_skill_lv: 1,
            lv,
            job_lv: 1,
            jp_ratio: Decimal::ONE,
            need_skill_id: [0; 3],
            need_skill_lv: [0; 3],
            cenhance_min: 0,
            cenhance_max: 0,
        }
    }

    /// Database ordering of tree rows: `ORDER BY lv, skill_id`.
    pub fn sort_key(&self) -> (i32, i32) {
        (self.lv, self.skill_id)
    }

    pub fn get(&self, field: EntryField) -> i32 {
        match field {
            EntryField::SkillId => self.skill_id,
            EntryField::SkillGroupId => self.skill_group_id,
            EntryField::MinSkillLv => self.min_skill_lv,
            EntryField::MaxSkillLv => self.max_skill_lv,
            EntryField::Lv => self.lv,
            EntryField::JobLv => self.job_lv,
            EntryField::NeedSkillId(i) => self.need_skill_id[i as usize % 3],
            EntryField::NeedSkillLv(i) => self.need_skill_lv[i as usize % 3],
            EntryField::CEnhanceMin => self.cenhance_min,
            EntryField::CEnhanceMax => self.cenhance_max,
        }
    }

    pub fn set(&mut self, field: EntryField, value: i32) {
        match field {
            EntryField::SkillId => self.skill_id = value,
            EntryField::SkillGroupId => self.skill_group_id = value,
            EntryField::MinSkillLv => self.min_skill_lv = value,
            EntryField::MaxSkillLv => self.max_skill_lv = value,
            EntryField::Lv => self.lv = value,
            EntryField::JobLv => self.job_lv = value,
            EntryField::NeedSkillId(i) => self.need_skill_id[i as usize % 3] = value,
            EntryField::NeedSkillLv(i) => self.need_skill_lv[i as usize % 3] = value,
            EntryField::CEnhanceMin => self.cenhance_min = value,
            EntryField::CEnhanceMax => self.cenhance_max = value,
        }
    }
}

/// Editable column of a skill tree row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntryField {
    SkillId,
    SkillGroupId,
    MinSkillLv,
    MaxSkillLv,
    Lv,
    JobLv,
    /// 0-based prerequisite index (0..3).
    NeedSkillId(u8),
    /// 0-based prerequisite index (0..3).
    NeedSkillLv(u8),
    CEnhanceMin,
    CEnhanceMax,
}

impl EntryField {
    /// All fields in the column order of the tree grid.
    pub const ALL: [EntryField; 14] = [
        EntryField::SkillId,
        EntryField::SkillGroupId,
        EntryField::MinSkillLv,
        EntryField::MaxSkillLv,
        EntryField::Lv,
        EntryField::JobLv,
        EntryField::NeedSkillId(0),
        EntryField::NeedSkillLv(0),
        EntryField::NeedSkillId(1),
        EntryField::NeedSkillLv(1),
        EntryField::NeedSkillId(2),
        EntryField::NeedSkillLv(2),
        EntryField::CEnhanceMin,
        EntryField::CEnhanceMax,
    ];

    /// Database column name.
    pub fn column(self) -> &'static str {
        match self {
            EntryField::SkillId => "skill_id",
            EntryField::SkillGroupId => "skill_group_id",
            EntryField::MinSkillLv => "min_skill_lv",
            EntryField::MaxSkillLv => "max_skill_lv",
            EntryField::Lv => "lv",
            EntryField::JobLv => "job_lv",
            EntryField::NeedSkillId(0) => "need_skill_id_1",
            EntryField::NeedSkillId(1) => "need_skill_id_2",
            EntryField::NeedSkillId(_) => "need_skill_id_3",
            EntryField::NeedSkillLv(0) => "need_skill_lv_1",
            EntryField::NeedSkillLv(1) => "need_skill_lv_2",
            EntryField::NeedSkillLv(_) => "need_skill_lv_3",
            EntryField::CEnhanceMin => "cenhance_min",
            EntryField::CEnhanceMax => "cenhance_max",
        }
    }

    /// Short grid header.
    pub fn header(self) -> &'static str {
        match self {
            EntryField::SkillId => "ID",
            EntryField::SkillGroupId => tr("Grp"),
            EntryField::MinSkillLv => tr("Min"),
            EntryField::MaxSkillLv => tr("Max"),
            EntryField::Lv => tr("Lv"),
            EntryField::JobLv => tr("Job"),
            EntryField::NeedSkillId(0) => "N1",
            EntryField::NeedSkillId(1) => "N2",
            EntryField::NeedSkillId(_) => "N3",
            EntryField::NeedSkillLv(0) => tr("N1 Lv"),
            EntryField::NeedSkillLv(1) => tr("N2 Lv"),
            EntryField::NeedSkillLv(_) => tr("N3 Lv"),
            EntryField::CEnhanceMin => tr("E Min"),
            EntryField::CEnhanceMax => tr("E Max"),
        }
    }

    /// Longer description shown as a header tooltip.
    pub fn description(self) -> &'static str {
        match self {
            EntryField::SkillId => tr("Skill ID (SkillResource.id)"),
            EntryField::SkillGroupId => tr("Random skill group ID (0 = none)"),
            EntryField::MinSkillLv => tr("Minimum skill level"),
            EntryField::MaxSkillLv => tr("Maximum skill level"),
            EntryField::Lv => tr("Required summon level"),
            EntryField::JobLv => tr("Required job level"),
            EntryField::NeedSkillId(_) => tr("Prerequisite skill ID"),
            EntryField::NeedSkillLv(_) => tr("Prerequisite skill level"),
            EntryField::CEnhanceMin => tr("Minimum creature enhance stage"),
            EntryField::CEnhanceMax => tr("Maximum creature enhance stage"),
        }
    }
}

/// Name, icon and tooltip of a skill, as shown in grids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillDisplay {
    pub skill_id: i32,
    pub skill_name: String,
    pub icon_file_name: Option<String>,
    pub tooltip_markup: Option<String>,
}

/// The seven primary stats, in database column order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stat {
    Str,
    Vit,
    Dex,
    Agi,
    Int,
    Men,
    Luk,
}

pub const STAT_COUNT: usize = 7;

impl Stat {
    pub const ALL: [Stat; STAT_COUNT] = [Stat::Str, Stat::Vit, Stat::Dex, Stat::Agi, Stat::Int, Stat::Men, Stat::Luk];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn label(self) -> &'static str {
        ["STR", "VIT", "DEX", "AGI", "INT", "MEN", "LUK"][self.index()]
    }

    /// Database column name.
    pub fn column(self) -> &'static str {
        ["str", "vit", "dex", "agi", "int", "men", "luk"][self.index()]
    }
}

/// One row of `StatResource`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatBlock {
    pub id: i32,
    pub values: [i32; STAT_COUNT],
}

impl StatBlock {
    pub fn zero(id: i32) -> Self {
        Self { id, values: [0; STAT_COUNT] }
    }

    pub fn get(&self, stat: Stat) -> i32 {
        self.values[stat.index()]
    }
}

/// One growth stage of `CreatureLevelBonus` for a summon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelBonusStage {
    /// 1-based growth stage.
    pub stage_index: i32,
    /// `CreatureLevelBonus.id` this stage reads from.
    pub lookup_id: i32,
    pub values: [Decimal; STAT_COUNT],
    pub exists_in_db: bool,
}

impl LevelBonusStage {
    pub fn get(&self, stat: Stat) -> Decimal {
        self.values[stat.index()]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatureEnhance {
    pub enhance_level: i32,
    pub stat_amplify: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatScalingSnapshot {
    pub summon_id: i32,
    pub stat_id: i32,
    pub evolve_type: i32,
    pub growth_depth: i32,
    pub base_stats: StatBlock,
    pub stage_bonuses: Vec<LevelBonusStage>,
    /// Highest summon level the server allows, when the database says it
    /// (the last level of `SummonLevelResource`); `None` keeps the model's own.
    pub max_level: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillUsage {
    pub skill_id: i32,
    pub skill_tree_id: i32,
    pub summon_id: i32,
    pub summon_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeUsage {
    pub skill_tree_id: i32,
    pub summon_id: i32,
    pub summon_name: String,
    pub tree_slot: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatUsage {
    pub stat_id: i32,
    pub summon_id: i32,
    pub summon_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelBonusUsage {
    pub bonus_id: i32,
    pub summon_id: i32,
    pub summon_name: String,
    pub stage_index: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RandomGroupUsage {
    pub group_id: i32,
    pub skill_tree_id: i32,
    pub summon_id: i32,
    pub summon_name: String,
    pub tree_slot: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_field_get_set_round_trips_every_field() {
        let mut entry = SkillTreeEntry::new_row(7, 3);
        for (i, field) in EntryField::ALL.into_iter().enumerate() {
            entry.set(field, 100 + i as i32);
        }
        for (i, field) in EntryField::ALL.into_iter().enumerate() {
            assert_eq!(entry.get(field), 100 + i as i32, "{field:?}");
        }
        assert_eq!(entry.skill_tree_id, 7);
    }

    #[test]
    fn entry_field_columns_are_unique() {
        let mut columns: Vec<_> = EntryField::ALL.iter().map(|f| f.column()).collect();
        columns.sort_unstable();
        columns.dedup();
        assert_eq!(columns.len(), EntryField::ALL.len());
    }
}
