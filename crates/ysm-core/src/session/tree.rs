//! Editable state of a single skill tree.

use crate::model::SkillTreeEntry;

/// Addresses a tree in the loaded chain: evolution stage and slot, both 1-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TreeRef {
    pub stage: u8,
    pub slot: u8,
}

impl TreeRef {
    pub fn new(stage: u8, slot: u8) -> Self {
        Self { stage, slot }
    }
}

/// A tree row being edited. `uid` is stable for the lifetime of the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryRow {
    pub uid: u64,
    pub entry: SkillTreeEntry,
    /// The database row this one started as; `None` for rows added in the editor.
    pub original: Option<SkillTreeEntry>,
}

impl EntryRow {
    pub fn is_new(&self) -> bool {
        self.original.is_none()
    }

    pub fn is_dirty(&self) -> bool {
        self.original != Some(self.entry)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeState {
    pub tree: TreeRef,
    pub summon_id: i32,
    /// `0` when the summon has no tree in this slot.
    pub tree_id: i32,
    pub original: Vec<SkillTreeEntry>,
    pub rows: Vec<EntryRow>,
}

impl TreeState {
    /// Rows can only be edited when the slot points at a real tree of a real summon.
    pub fn is_editable(&self) -> bool {
        self.tree_id > 0 && self.summon_id > 0
    }

    /// Current rows in database order, stamped with this tree's ID.
    pub fn current_sorted(&self) -> Vec<SkillTreeEntry> {
        let mut entries: Vec<SkillTreeEntry> =
            self.rows.iter().map(|row| SkillTreeEntry { skill_tree_id: self.tree_id, ..row.entry }).collect();
        entries.sort_by_key(SkillTreeEntry::sort_key);
        entries
    }

    /// Whether saving would write anything for this tree.
    pub fn is_changed(&self) -> bool {
        if !self.is_editable() {
            return false;
        }
        let mut original = self.original.clone();
        original.sort_by_key(SkillTreeEntry::sort_key);
        original != self.current_sorted()
    }

    pub fn row(&self, uid: u64) -> Option<&EntryRow> {
        self.rows.iter().find(|row| row.uid == uid)
    }

    pub(super) fn row_mut(&mut self, uid: u64) -> Option<&mut EntryRow> {
        self.rows.iter_mut().find(|row| row.uid == uid)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageState {
    /// 1-based evolution stage.
    pub stage: u8,
    /// `0` when the chain has no summon at this stage.
    pub summon_id: i32,
    pub trees: Vec<TreeState>,
}

/// Skills of one evolution stage merged across its trees by `(skill_id, lv)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveRow {
    pub skill_id: i32,
    pub lv: i32,
    pub min_skill_lv: i32,
    pub max_skill_lv: i32,
    pub is_dirty: bool,
    /// Distinct random groups of the merged rows, ascending by group ID.
    pub groups: Vec<EffectiveGroup>,
    /// Number of tree rows merged into this one.
    pub linked: usize,
}

/// A random group and one source row that links to it in this evolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectiveGroup {
    pub group_id: i32,
    pub tree: TreeRef,
    pub uid: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeChangeKind {
    Insert,
    Update,
    Delete,
}

/// One row-level difference between a tree and its database state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeChange {
    pub kind: TreeChangeKind,
    pub tree: TreeRef,
    pub tree_id: i32,
    pub before: Option<SkillTreeEntry>,
    pub after: Option<SkillTreeEntry>,
}
