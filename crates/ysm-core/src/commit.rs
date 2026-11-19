//! The unit of work sent to the database (or rendered as a SQL script).

use rust_decimal::Decimal;

use crate::model::{STAT_COUNT, SkillTreeEntry, StatBlock};
use crate::random_group::RandomSkillGroup;
use crate::value::{FieldKind, FieldValue};

/// Identifies one skill tree slot of one summon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TreeKey {
    pub summon_id: i32,
    /// 1-based slot (`skill_tree1_id..skill_tree5_id`).
    pub slot: u8,
    pub tree_id: i32,
}

/// Full replacement of one skill tree's rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeCommit {
    pub summon_id: i32,
    pub slot: u8,
    pub original_tree_id: i32,
    pub entries: Vec<SkillTreeEntry>,
    /// Write the rows under a new tree ID and repoint only this summon slot.
    pub clone_if_shared: bool,
}

impl TreeCommit {
    pub fn key(&self) -> TreeKey {
        TreeKey { summon_id: self.summon_id, slot: self.slot, tree_id: self.original_tree_id }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldChange {
    pub name: String,
    pub kind: FieldKind,
    pub old: FieldValue,
    pub new: FieldValue,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SkillUpdate {
    pub skill_id: i32,
    pub changes: Vec<FieldChange>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatCommit {
    pub summon_id: i32,
    pub original_stat_id: i32,
    pub stats: StatBlock,
    /// Insert a new `StatResource` row and repoint only this summon.
    pub clone_if_shared: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelBonusUpsert {
    pub bonus_id: i32,
    pub values: [Decimal; STAT_COUNT],
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CommitRequest {
    pub trees: Vec<TreeCommit>,
    pub skills: Vec<SkillUpdate>,
    pub stats: Vec<StatCommit>,
    pub level_bonuses: Vec<LevelBonusUpsert>,
    pub random_groups: Vec<RandomSkillGroup>,
}

impl CommitRequest {
    pub fn is_empty(&self) -> bool {
        self.trees.is_empty()
            && self.skills.is_empty()
            && self.stats.is_empty()
            && self.level_bonuses.is_empty()
            && self.random_groups.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CommitResult {
    pub updated_trees: usize,
    pub updated_skills: usize,
    pub updated_random_groups: usize,
}

/// Column of `SummonResource` holding the tree ID of a slot.
pub fn tree_slot_column(slot: u8) -> Option<&'static str> {
    match slot {
        1 => Some("skill_tree1_id"),
        2 => Some("skill_tree2_id"),
        3 => Some("skill_tree3_id"),
        4 => Some("skill_tree4_id"),
        5 => Some("skill_tree5_id"),
        _ => None,
    }
}

/// Whether a name is safe to splice into SQL as a quoted identifier.
pub fn is_sql_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub fn is_updatable_skill_column(change: &FieldChange) -> bool {
    is_sql_identifier(&change.name) && !change.name.eq_ignore_ascii_case("id") && change.kind.is_editable()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifier_check_rejects_injection_attempts() {
        assert!(is_sql_identifier("var12"));
        assert!(is_sql_identifier("_x"));
        assert!(!is_sql_identifier(""));
        assert!(!is_sql_identifier("1abc"));
        assert!(!is_sql_identifier("a]; DROP TABLE x;--"));
        assert!(!is_sql_identifier("a b"));
    }

    #[test]
    fn slot_columns_cover_exactly_five_slots() {
        assert_eq!(tree_slot_column(1), Some("skill_tree1_id"));
        assert_eq!(tree_slot_column(5), Some("skill_tree5_id"));
        assert_eq!(tree_slot_column(0), None);
        assert_eq!(tree_slot_column(6), None);
    }
}
