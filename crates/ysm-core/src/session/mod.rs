//! The edit session: everything the user has loaded and staged.
//!
//! All mutations of skill trees and random groups go through
//! [`EditSession`], which keeps an undo history and can turn the staged state
//! into a [`CommitRequest`]. The session never talks to the database; it
//! reports which skills and groups it still needs (`take_missing_*`) and the
//! caller feeds the results back (`ingest_*`).

mod tree;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::commit::{CommitRequest, SkillUpdate, TreeCommit};
use crate::i18n::trf;
use crate::model::{
    EntryField, EvolutionChain, STAGE_COUNT, SkillDisplay, SkillTreeEntry, StatScalingSnapshot, TREE_SLOTS,
};
use crate::random_group::RandomSkillGroup;
use crate::scaling_edit::{LevelBonusEdit, ScalingEditResult, StatEdit};

pub use tree::{EffectiveRow, EntryRow, StageState, TreeChange, TreeChangeKind, TreeRef, TreeState};

/// How many undo steps are kept.
const UNDO_LIMIT: usize = 200;

/// Everything loaded from the database for one evolution chain.
#[derive(Debug, Clone, PartialEq)]
pub struct ChainBundle {
    pub chain: EvolutionChain,
    /// Tree rows indexed by `[stage - 1][slot - 1]`.
    pub trees: [[Vec<SkillTreeEntry>; TREE_SLOTS]; STAGE_COUNT],
    pub skill_display: HashMap<i32, SkillDisplay>,
    pub random_groups: HashMap<i32, RandomSkillGroup>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupState {
    /// Edited or created in this session; will be written on save.
    Pending,
    /// Unchanged database row.
    Loaded,
    /// Referenced by a tree row but absent from `SummonRandomSkillResource`.
    MissingInDb,
    /// Not fetched yet.
    Loading,
}

#[derive(Debug, Clone)]
struct Snapshot {
    rows: Vec<Vec<EntryRow>>,
    pending_groups: BTreeMap<i32, RandomSkillGroup>,
    manual_groups: HashSet<i32>,
}

#[derive(Debug, Default)]
pub struct EditSession {
    chain: Option<EvolutionChain>,
    stages: Vec<StageState>,

    skill_display: HashMap<i32, SkillDisplay>,
    requested_skills: HashSet<i32>,

    db_groups: HashMap<i32, RandomSkillGroup>,
    db_missing_groups: HashSet<i32>,
    requested_groups: HashSet<i32>,
    /// Group IDs typed into a row by hand that still need a database lookup.
    manual_groups: HashSet<i32>,

    pending_groups: BTreeMap<i32, RandomSkillGroup>,
    pending_skills: BTreeMap<i32, SkillUpdate>,
    pending_stats: BTreeMap<i32, StatEdit>,
    pending_bonuses: BTreeMap<i32, LevelBonusEdit>,

    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    next_uid: u64,
}

impl EditSession {
    pub fn new() -> Self {
        Self::default()
    }

    // ---- loading -----------------------------------------------------------

    /// Replaces the whole session with a freshly loaded chain.
    pub fn load(&mut self, bundle: ChainBundle) {
        let mut next_uid = self.next_uid;
        let ChainBundle { chain, trees, skill_display, random_groups } = bundle;

        let stages: Vec<StageState> = trees
            .into_iter()
            .enumerate()
            .map(|(stage_index, stage_trees)| {
                let stage = stage_index as u8 + 1;
                let summon = chain.stage(stage);
                let summon_id = summon.map_or(0, |s| s.summon_id);
                let trees = stage_trees
                    .into_iter()
                    .enumerate()
                    .map(|(slot_index, original)| {
                        let tree_id = summon.map_or(0, |s| s.skill_tree_ids[slot_index]).max(0);
                        let rows = original
                            .iter()
                            .map(|entry| {
                                next_uid += 1;
                                EntryRow { uid: next_uid, entry: *entry, original: Some(*entry) }
                            })
                            .collect();
                        TreeState {
                            tree: TreeRef::new(stage, slot_index as u8 + 1),
                            summon_id,
                            tree_id,
                            original,
                            rows,
                        }
                    })
                    .collect();
                StageState { stage, summon_id, trees }
            })
            .collect();

        *self =
            Self { chain: Some(chain), stages, skill_display, db_groups: random_groups, next_uid, ..Self::default() };

        // The loader already asked for everything the chain references, so
        // whatever is still unknown does not exist in the database.
        self.requested_skills = self.referenced_skill_ids();
        let referenced_groups = self.referenced_group_ids();
        self.db_missing_groups =
            referenced_groups.iter().copied().filter(|id| !self.db_groups.contains_key(id)).collect();
        self.requested_groups = referenced_groups;
    }

    /// Drops the loaded chain and every staged change.
    pub fn clear(&mut self) {
        *self = Self { next_uid: self.next_uid, ..Self::default() };
    }

    pub fn is_loaded(&self) -> bool {
        self.chain.is_some()
    }

    pub fn chain(&self) -> Option<&EvolutionChain> {
        self.chain.as_ref()
    }

    pub fn stages(&self) -> &[StageState] {
        &self.stages
    }

    pub fn tree(&self, tree: TreeRef) -> Option<&TreeState> {
        self.stages.iter().flat_map(|s| &s.trees).find(|t| t.tree == tree)
    }

    fn tree_mut(&mut self, tree: TreeRef) -> Option<&mut TreeState> {
        self.stages.iter_mut().flat_map(|s| &mut s.trees).find(|t| t.tree == tree)
    }

    fn trees(&self) -> impl Iterator<Item = &TreeState> {
        self.stages.iter().flat_map(|s| &s.trees)
    }

    /// Other slots of the loaded chain that point at the same tree ID.
    ///
    /// They are edited independently, so saving conflicting edits to both
    /// would make the later one win.
    pub fn trees_sharing_id(&self, tree: TreeRef) -> Vec<TreeRef> {
        let Some(this) = self.tree(tree).filter(|t| t.tree_id > 0) else {
            return Vec::new();
        };
        self.trees().filter(|t| t.tree != tree && t.tree_id == this.tree_id).map(|t| t.tree).collect()
    }

    // ---- skill display -----------------------------------------------------

    pub fn skill_display(&self, skill_id: i32) -> Option<&SkillDisplay> {
        self.skill_display.get(&skill_id)
    }

    pub fn skill_name(&self, skill_id: i32) -> String {
        self.skill_display.get(&skill_id).map_or_else(|| trf("Skill {}", &[&skill_id]), |d| d.skill_name.clone())
    }

    /// Skill IDs that need a display lookup and have not been asked for yet.
    pub fn take_missing_skill_ids(&mut self) -> Vec<i32> {
        let mut missing: Vec<i32> = self
            .referenced_skill_ids()
            .into_iter()
            .filter(|id| !self.skill_display.contains_key(id) && !self.requested_skills.contains(id))
            .collect();
        missing.sort_unstable();
        self.requested_skills.extend(missing.iter().copied());
        missing
    }

    pub fn ingest_skill_display(&mut self, found: impl IntoIterator<Item = SkillDisplay>) {
        for display in found {
            self.requested_skills.insert(display.skill_id);
            self.skill_display.insert(display.skill_id, display);
        }
    }

    fn referenced_skill_ids(&self) -> HashSet<i32> {
        let row_skills = self.trees().flat_map(|t| &t.rows).map(|r| r.entry.skill_id);
        let group_skills = self
            .referenced_group_ids()
            .into_iter()
            .chain(self.pending_groups.keys().copied())
            .filter_map(|id| self.overlay_group(id))
            .flat_map(|g| g.skill_ids().iter().copied())
            .collect::<Vec<_>>();
        row_skills.chain(group_skills).filter(|id| *id > 0).collect()
    }

    // ---- random groups -----------------------------------------------------

    /// The group as it would be after saving: staged version first, then the database one.
    pub fn overlay_group(&self, group_id: i32) -> Option<&RandomSkillGroup> {
        if group_id <= 0 {
            return None;
        }
        self.pending_groups.get(&group_id).or_else(|| self.db_groups.get(&group_id))
    }

    /// The group as stored in the database, ignoring staged edits.
    pub fn db_group(&self, group_id: i32) -> Option<&RandomSkillGroup> {
        self.db_groups.get(&group_id)
    }

    pub fn group_state(&self, group_id: i32) -> GroupState {
        if self.pending_groups.contains_key(&group_id) {
            GroupState::Pending
        } else if self.db_groups.contains_key(&group_id) {
            GroupState::Loaded
        } else if self.db_missing_groups.contains(&group_id) {
            GroupState::MissingInDb
        } else {
            GroupState::Loading
        }
    }

    /// Group IDs that need a database lookup and have not been asked for yet.
    pub fn take_missing_group_ids(&mut self) -> Vec<i32> {
        let mut missing: Vec<i32> = self
            .referenced_group_ids()
            .into_iter()
            .filter(|id| {
                !self.db_groups.contains_key(id)
                    && !self.pending_groups.contains_key(id)
                    && !self.db_missing_groups.contains(id)
                    && !self.requested_groups.contains(id)
            })
            .collect();
        missing.sort_unstable();
        self.requested_groups.extend(missing.iter().copied());
        missing
    }

    /// Feeds back the result of a group lookup for `requested` IDs.
    ///
    /// A group ID the user typed by hand that does not exist yet becomes an
    /// empty staged group, so saving creates it.
    pub fn ingest_random_groups(&mut self, requested: &[i32], found: impl IntoIterator<Item = RandomSkillGroup>) {
        for group in found {
            self.manual_groups.remove(&group.group_id);
            self.db_groups.insert(group.group_id, group);
        }
        for id in requested.iter().copied().filter(|id| *id > 0) {
            if self.db_groups.contains_key(&id) {
                continue;
            }
            if self.manual_groups.remove(&id) {
                self.pending_groups.entry(id).or_insert_with(|| RandomSkillGroup::empty(id));
            } else if !self.pending_groups.contains_key(&id) {
                self.db_missing_groups.insert(id);
            }
        }
    }

    /// Every group ID in use by the session; new groups must avoid these.
    pub fn reserved_group_ids(&self) -> HashSet<i32> {
        self.referenced_group_ids()
            .into_iter()
            .chain(self.pending_groups.keys().copied())
            .chain(self.db_groups.keys().copied())
            .collect()
    }

    fn referenced_group_ids(&self) -> HashSet<i32> {
        self.trees().flat_map(|t| &t.rows).map(|r| r.entry.skill_group_id).filter(|id| *id > 0).collect()
    }

    // ---- tree edits --------------------------------------------------------

    /// Appends a row to a tree; `skill_id` may be `0` for an empty row.
    pub fn add_row(&mut self, tree: TreeRef, skill_id: i32) -> Option<u64> {
        let state = self.tree(tree).filter(|t| t.is_editable())?;
        let next_lv = state.rows.iter().map(|r| r.entry.lv).max().map_or(1, |lv| lv + 1);
        let mut entry = SkillTreeEntry::new_row(state.tree_id, next_lv);
        entry.skill_id = skill_id.max(0);

        self.checkpoint();
        let uid = self.allocate_uid();
        self.tree_mut(tree)?.rows.push(EntryRow { uid, entry, original: None });
        Some(uid)
    }

    /// Inserts a copy of a row right below it.
    pub fn duplicate_row(&mut self, tree: TreeRef, uid: u64) -> Option<u64> {
        let state = self.tree(tree).filter(|t| t.is_editable())?;
        let index = state.rows.iter().position(|r| r.uid == uid)?;
        let entry = state.rows[index].entry;

        self.checkpoint();
        let new_uid = self.allocate_uid();
        self.tree_mut(tree)?.rows.insert(index + 1, EntryRow { uid: new_uid, entry, original: None });
        Some(new_uid)
    }

    pub fn delete_row(&mut self, tree: TreeRef, uid: u64) -> bool {
        if self.tree(tree).and_then(|t| t.row(uid)).is_none() {
            return false;
        }
        self.checkpoint();
        if let Some(state) = self.tree_mut(tree) {
            state.rows.retain(|r| r.uid != uid);
        }
        true
    }

    /// Removes every row of a stage merged into the effective row `(skill_id, lv)`.
    pub fn delete_effective(&mut self, stage: u8, skill_id: i32, lv: i32) -> usize {
        let matches = |r: &EntryRow| r.entry.skill_id == skill_id && r.entry.lv == lv;
        let count = self.stage_rows(stage).filter(|r| matches(r)).count();
        if count == 0 {
            return 0;
        }
        self.checkpoint();
        for state in self.stage_trees_mut(stage) {
            state.rows.retain(|r| !matches(r));
        }
        count
    }

    /// Sets one field of a row. Returns whether anything changed.
    pub fn set_field(&mut self, tree: TreeRef, uid: u64, field: EntryField, value: i32) -> bool {
        let Some(row) = self.tree(tree).and_then(|t| t.row(uid)) else {
            return false;
        };
        if row.entry.get(field) == value {
            return false;
        }

        self.checkpoint();
        if field == EntryField::SkillGroupId && value > 0 && self.overlay_group(value).is_none() {
            self.manual_groups.insert(value);
            self.db_missing_groups.remove(&value);
            self.requested_groups.remove(&value);
        }
        if let Some(row) = self.tree_mut(tree).and_then(|t| t.row_mut(uid)) {
            row.entry.set(field, value);
        }
        true
    }

    /// Applies min/max skill level to every row merged into an effective row.
    pub fn set_effective_levels(&mut self, stage: u8, skill_id: i32, lv: i32, min: i32, max: i32) -> bool {
        let matches = |r: &EntryRow| r.entry.skill_id == skill_id && r.entry.lv == lv;
        let differs = |r: &EntryRow| r.entry.min_skill_lv != min || r.entry.max_skill_lv != max;
        if !self.stage_rows(stage).any(|r| matches(r) && differs(r)) {
            return false;
        }
        self.checkpoint();
        for row in self.stage_trees_mut(stage).flat_map(|t| &mut t.rows).filter(|r| matches(r)) {
            row.entry.min_skill_lv = min;
            row.entry.max_skill_lv = max;
        }
        true
    }

    /// Stages a new version of a random group.
    pub fn stage_random_group(&mut self, group: RandomSkillGroup) {
        if group.group_id <= 0 || self.pending_groups.get(&group.group_id) == Some(&group) {
            return;
        }
        self.checkpoint();
        self.manual_groups.remove(&group.group_id);
        self.pending_groups.insert(group.group_id, group);
    }

    /// Stages a group and points a row at it, as one undo step.
    pub fn attach_group(&mut self, tree: TreeRef, uid: u64, group: RandomSkillGroup) -> bool {
        if group.group_id <= 0 || self.tree(tree).and_then(|t| t.row(uid)).is_none() {
            return false;
        }
        self.checkpoint();
        let group_id = group.group_id;
        self.manual_groups.remove(&group_id);
        self.pending_groups.insert(group_id, group);
        if let Some(row) = self.tree_mut(tree).and_then(|t| t.row_mut(uid)) {
            row.entry.skill_group_id = group_id;
        }
        true
    }

    pub fn detach_group(&mut self, tree: TreeRef, uid: u64) -> bool {
        self.set_field(tree, uid, EntryField::SkillGroupId, 0)
    }

    fn stage_rows(&self, stage: u8) -> impl Iterator<Item = &EntryRow> {
        self.stages.iter().filter(move |s| s.stage == stage).flat_map(|s| &s.trees).flat_map(|t| &t.rows)
    }

    fn stage_trees_mut(&mut self, stage: u8) -> impl Iterator<Item = &mut TreeState> {
        self.stages.iter_mut().filter(move |s| s.stage == stage).flat_map(|s| &mut s.trees)
    }

    fn allocate_uid(&mut self) -> u64 {
        self.next_uid += 1;
        self.next_uid
    }

    // ---- undo / redo -------------------------------------------------------

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            rows: self.trees().map(|t| t.rows.clone()).collect(),
            pending_groups: self.pending_groups.clone(),
            manual_groups: self.manual_groups.clone(),
        }
    }

    fn restore(&mut self, snapshot: Snapshot) {
        let Snapshot { rows, pending_groups, manual_groups } = snapshot;
        for (state, rows) in self.stages.iter_mut().flat_map(|s| &mut s.trees).zip(rows) {
            state.rows = rows;
        }
        self.pending_groups = pending_groups;
        self.manual_groups = manual_groups;
    }

    fn checkpoint(&mut self) {
        self.undo.push(self.snapshot());
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Reverts the last tree or random group edit.
    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.undo.pop() else {
            return false;
        };
        self.redo.push(self.snapshot());
        self.restore(previous);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(self.snapshot());
        self.restore(next);
        true
    }

    // ---- skills and stats --------------------------------------------------

    /// Stages (or, when it carries no changes, unstages) a skill field update.
    pub fn stage_skill_update(&mut self, update: SkillUpdate) {
        if update.changes.is_empty() {
            self.pending_skills.remove(&update.skill_id);
        } else {
            self.pending_skills.insert(update.skill_id, update);
        }
    }

    pub fn pending_skill(&self, skill_id: i32) -> Option<&SkillUpdate> {
        self.pending_skills.get(&skill_id)
    }

    /// A database scaling snapshot with this session's staged edits applied.
    pub fn pending_scaling(&self, db: &StatScalingSnapshot) -> StatScalingSnapshot {
        let mut staged = db.clone();
        if let Some(edit) = self.pending_stats.get(&db.summon_id) {
            staged.base_stats.values = edit.commit.stats.values;
        }
        for stage in &mut staged.stage_bonuses {
            if let Some(edit) = self.pending_bonuses.get(&stage.lookup_id) {
                stage.values = edit.upsert.values;
            }
        }
        staged
    }

    /// Replaces the staged stat edits of one summon with the result of the stat editor.
    pub fn apply_scaling_edit(&mut self, result: ScalingEditResult) {
        match result.stat {
            Some(edit) => self.pending_stats.insert(result.summon_id, edit),
            None => self.pending_stats.remove(&result.summon_id),
        };
        for lookup_id in &result.lookup_ids {
            self.pending_bonuses.remove(lookup_id);
        }
        for edit in result.bonuses {
            self.pending_bonuses.insert(edit.upsert.bonus_id, edit);
        }
    }

    pub fn pending_skills(&self) -> impl Iterator<Item = &SkillUpdate> {
        self.pending_skills.values()
    }

    pub fn pending_stats(&self) -> impl Iterator<Item = &StatEdit> {
        self.pending_stats.values()
    }

    pub fn pending_bonuses(&self) -> impl Iterator<Item = &LevelBonusEdit> {
        self.pending_bonuses.values()
    }

    // ---- staged state ------------------------------------------------------

    /// Throws away every staged change and restores trees to their database state.
    pub fn discard_all(&mut self) {
        for state in self.stages.iter_mut().flat_map(|s| &mut s.trees) {
            let mut next_uid = self.next_uid;
            state.rows = state
                .original
                .iter()
                .map(|entry| {
                    next_uid += 1;
                    EntryRow { uid: next_uid, entry: *entry, original: Some(*entry) }
                })
                .collect();
            self.next_uid = next_uid;
        }
        self.pending_groups.clear();
        self.pending_skills.clear();
        self.pending_stats.clear();
        self.pending_bonuses.clear();
        self.manual_groups.clear();
        self.undo.clear();
        self.redo.clear();
    }

    pub fn has_changes(&self) -> bool {
        !self.build_commit_request().is_empty()
    }

    /// Number of staged changes, as shown on the Save button.
    pub fn change_count(&self) -> usize {
        let request = self.build_commit_request();
        self.tree_changes().len()
            + request.skills.iter().map(|s| s.changes.len()).sum::<usize>()
            + request.stats.len()
            + request.level_bonuses.len()
            + request.random_groups.len()
    }

    /// Skills of a stage merged across its trees, ordered by skill ID then level.
    pub fn effective_rows(&self, stage: u8) -> Vec<EffectiveRow> {
        let mut merged: BTreeMap<(i32, i32), (EffectiveRow, BTreeSet<i32>)> = BTreeMap::new();
        for row in self.stage_rows(stage).filter(|r| r.entry.skill_id > 0) {
            let entry = &row.entry;
            let (effective, groups) = merged.entry((entry.skill_id, entry.lv)).or_insert_with(|| {
                (
                    EffectiveRow {
                        skill_id: entry.skill_id,
                        lv: entry.lv,
                        min_skill_lv: entry.min_skill_lv,
                        max_skill_lv: entry.max_skill_lv,
                        is_dirty: false,
                        group_ids: Vec::new(),
                        linked: 0,
                    },
                    BTreeSet::new(),
                )
            });
            effective.min_skill_lv = effective.min_skill_lv.min(entry.min_skill_lv);
            effective.max_skill_lv = effective.max_skill_lv.max(entry.max_skill_lv);
            effective.is_dirty |= row.is_dirty();
            effective.linked += 1;
            if entry.skill_group_id > 0 {
                groups.insert(entry.skill_group_id);
            }
        }
        merged
            .into_values()
            .map(|(mut effective, groups)| {
                effective.group_ids = groups.into_iter().collect();
                effective
            })
            .collect()
    }

    /// Every editable tree with its current rows, changed or not.
    pub fn editable_tree_commits(&self) -> Vec<TreeCommit> {
        self.trees().filter(|t| t.is_editable()).map(tree_commit).collect()
    }

    /// The staged state as a unit of work. Trees are included only when their rows differ.
    pub fn build_commit_request(&self) -> CommitRequest {
        let referenced_groups = self.referenced_group_ids();
        CommitRequest {
            trees: self.trees().filter(|t| t.is_changed()).map(tree_commit).collect(),
            skills: self.pending_skills.values().cloned().collect(),
            stats: self.pending_stats.values().map(|e| e.commit.clone()).collect(),
            level_bonuses: self.pending_bonuses.values().map(|e| e.upsert.clone()).collect(),
            // An empty group nobody points at any more is a leftover placeholder.
            random_groups: self
                .pending_groups
                .values()
                .filter(|g| {
                    !g.skill_ids().is_empty()
                        || referenced_groups.contains(&g.group_id)
                        || self.db_groups.contains_key(&g.group_id)
                })
                .cloned()
                .collect(),
        }
    }

    /// Row-level differences of every changed tree, matched by row identity.
    pub fn tree_changes(&self) -> Vec<TreeChange> {
        let mut changes = Vec::new();
        for state in self.trees().filter(|t| t.is_changed()) {
            let change =
                |kind, before, after| TreeChange { kind, tree: state.tree, tree_id: state.tree_id, before, after };
            let mut unmatched = state.original.clone();

            for row in &state.rows {
                let after = SkillTreeEntry { skill_tree_id: state.tree_id, ..row.entry };
                match row.original {
                    None => changes.push(change(TreeChangeKind::Insert, None, Some(after))),
                    Some(original) => {
                        if let Some(index) = unmatched.iter().position(|e| *e == original) {
                            unmatched.swap_remove(index);
                        }
                        if row.is_dirty() {
                            changes.push(change(TreeChangeKind::Update, Some(original), Some(after)));
                        }
                    }
                }
            }

            unmatched.sort_by_key(SkillTreeEntry::sort_key);
            for removed in unmatched {
                changes.push(change(TreeChangeKind::Delete, Some(removed), None));
            }
        }
        changes
    }
}

fn tree_commit(state: &TreeState) -> TreeCommit {
    TreeCommit {
        summon_id: state.summon_id,
        slot: state.tree.slot,
        original_tree_id: state.tree_id,
        entries: state.current_sorted(),
        clone_if_shared: false,
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::model::SummonListItem;

    pub fn summon(id: i32, evolve_target: i32, tree_ids: [i32; 5]) -> SummonListItem {
        SummonListItem {
            summon_id: id,
            summon_name: format!("Summon {id}"),
            card_name: None,
            card_id: 500,
            evolve_target,
            form: 1,
            is_riding_only: false,
            face_file_name: None,
            illust_file_name: None,
            feature_text: None,
            stat_id: id,
            skill_tree_ids: tree_ids,
        }
    }

    pub fn entry(tree_id: i32, skill_id: i32, lv: i32) -> SkillTreeEntry {
        SkillTreeEntry { skill_id, ..SkillTreeEntry::new_row(tree_id, lv) }
    }

    /// Chain 1 → 2 → 3. Stage 1 slot 1 is tree 10 (skills 100, 101); stage 1
    /// slot 2 is tree 11 (skill 100 again); stage 3 slot 1 is tree 30 with a
    /// row in random group 7.
    pub fn bundle() -> ChainBundle {
        let chain = EvolutionChain {
            stage1: Some(summon(1, 2, [10, 11, 0, 0, 0])),
            stage2: Some(summon(2, 3, [20, 0, 0, 0, 0])),
            stage3: summon(3, 0, [30, 0, 0, 0, 0]),
        };
        let mut trees: [[Vec<SkillTreeEntry>; TREE_SLOTS]; STAGE_COUNT] = Default::default();
        trees[0][0] = vec![entry(10, 100, 1), entry(10, 101, 5)];
        trees[0][1] = vec![SkillTreeEntry { min_skill_lv: 2, max_skill_lv: 9, ..entry(11, 100, 1) }];
        trees[1][0] = vec![entry(20, 200, 1)];
        trees[2][0] = vec![SkillTreeEntry { skill_group_id: 7, ..entry(30, 300, 1) }];

        let display = |id: i32| SkillDisplay {
            skill_id: id,
            skill_name: format!("Name {id}"),
            icon_file_name: None,
            tooltip_markup: None,
        };
        ChainBundle {
            chain,
            trees,
            skill_display: [100, 101, 200, 300, 900].into_iter().map(|id| (id, display(id))).collect(),
            random_groups: HashMap::from([(7, RandomSkillGroup::new(7, [900]))]),
        }
    }

    pub fn loaded() -> EditSession {
        let mut session = EditSession::new();
        session.load(bundle());
        session
    }

    pub fn first_uid(session: &EditSession, tree: TreeRef) -> u64 {
        session.tree(tree).unwrap().rows[0].uid
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    const T11: TreeRef = TreeRef { stage: 1, slot: 1 };
    const T12: TreeRef = TreeRef { stage: 1, slot: 2 };
    const T13: TreeRef = TreeRef { stage: 1, slot: 3 };
    const T31: TreeRef = TreeRef { stage: 3, slot: 1 };

    #[test]
    fn freshly_loaded_session_has_no_changes() {
        let session = loaded();
        assert!(session.is_loaded());
        assert!(!session.has_changes());
        assert_eq!(session.change_count(), 0);
        assert!(session.build_commit_request().is_empty());
        assert_eq!(session.stages().len(), 3);
        assert!(session.stages().iter().all(|s| s.trees.len() == 5));
    }

    #[test]
    fn editing_a_field_marks_the_row_dirty_and_reverting_clears_it() {
        let mut session = loaded();
        let uid = first_uid(&session, T11);

        assert!(session.set_field(T11, uid, EntryField::MaxSkillLv, 10));
        assert!(session.tree(T11).unwrap().row(uid).unwrap().is_dirty());
        assert!(session.has_changes());

        assert!(session.set_field(T11, uid, EntryField::MaxSkillLv, 1));
        assert!(!session.tree(T11).unwrap().row(uid).unwrap().is_dirty());
        assert!(!session.has_changes());
    }

    #[test]
    fn setting_the_same_value_is_not_an_edit() {
        let mut session = loaded();
        let uid = first_uid(&session, T11);
        assert!(!session.set_field(T11, uid, EntryField::Lv, 1));
        assert!(!session.can_undo());
    }

    #[test]
    fn add_row_uses_the_next_level_and_is_dirty() {
        let mut session = loaded();
        let uid = session.add_row(T11, 555).unwrap();
        let row = session.tree(T11).unwrap().row(uid).unwrap();

        assert_eq!(row.entry.lv, 6);
        assert_eq!(row.entry.skill_id, 555);
        assert_eq!(row.entry.skill_tree_id, 10);
        assert!(row.is_new());
        assert!(row.is_dirty());
    }

    #[test]
    fn rows_cannot_be_added_to_an_empty_slot() {
        let mut session = loaded();
        assert_eq!(session.add_row(T13, 1), None);
        assert!(!session.can_undo());
    }

    #[test]
    fn duplicate_inserts_a_new_row_below_the_source() {
        let mut session = loaded();
        let uid = first_uid(&session, T11);
        let copy = session.duplicate_row(T11, uid).unwrap();
        let rows = &session.tree(T11).unwrap().rows;

        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1].uid, copy);
        assert_eq!(rows[1].entry, rows[0].entry);
        assert!(rows[1].is_new());
    }

    #[test]
    fn deleting_and_undoing_restores_the_row() {
        let mut session = loaded();
        let uid = first_uid(&session, T11);

        assert!(session.delete_row(T11, uid));
        assert_eq!(session.tree(T11).unwrap().rows.len(), 1);
        assert!(session.undo());
        assert_eq!(session.tree(T11).unwrap().rows.len(), 2);
        assert!(!session.has_changes());
        assert!(session.redo());
        assert_eq!(session.tree(T11).unwrap().rows.len(), 1);
    }

    #[test]
    fn a_new_edit_clears_the_redo_history() {
        let mut session = loaded();
        let uid = first_uid(&session, T11);
        session.set_field(T11, uid, EntryField::JobLv, 9);
        session.undo();
        assert!(session.can_redo());
        session.set_field(T11, uid, EntryField::JobLv, 4);
        assert!(!session.can_redo());
    }

    #[test]
    fn effective_rows_merge_by_skill_and_level() {
        let session = loaded();
        let rows = session.effective_rows(1);

        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].skill_id, rows[0].lv, rows[0].linked), (100, 1, 2));
        assert_eq!((rows[0].min_skill_lv, rows[0].max_skill_lv), (1, 9));
        assert_eq!((rows[1].skill_id, rows[1].lv, rows[1].linked), (101, 5, 1));
        assert!(!rows[0].is_dirty);
    }

    #[test]
    fn effective_rows_report_random_groups_and_skip_empty_skills() {
        let mut session = loaded();
        session.add_row(T31, 0);
        let rows = session.effective_rows(3);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].group_ids, [7]);
    }

    #[test]
    fn effective_level_edit_propagates_to_all_linked_rows() {
        let mut session = loaded();
        assert!(session.set_effective_levels(1, 100, 1, 3, 12));

        let a = session.tree(T11).unwrap().rows[0].entry;
        let b = session.tree(T12).unwrap().rows[0].entry;
        assert_eq!((a.min_skill_lv, a.max_skill_lv), (3, 12));
        assert_eq!((b.min_skill_lv, b.max_skill_lv), (3, 12));
        assert!(session.effective_rows(1)[0].is_dirty);
        assert!(!session.set_effective_levels(1, 100, 1, 3, 12));
    }

    #[test]
    fn deleting_an_effective_row_removes_it_from_every_tree() {
        let mut session = loaded();
        assert_eq!(session.delete_effective(1, 100, 1), 2);
        assert_eq!(session.tree(T11).unwrap().rows.len(), 1);
        assert!(session.tree(T12).unwrap().rows.is_empty());
        assert_eq!(session.delete_effective(1, 100, 1), 0);
    }

    #[test]
    fn commit_request_contains_only_changed_trees_in_database_order() {
        let mut session = loaded();
        session.add_row(T11, 50);
        let uid = session.tree(T11).unwrap().rows.last().unwrap().uid;
        session.set_field(T11, uid, EntryField::Lv, 0);

        let request = session.build_commit_request();
        assert_eq!(request.trees.len(), 1);
        let tree = &request.trees[0];
        assert_eq!((tree.summon_id, tree.slot, tree.original_tree_id), (1, 1, 10));
        assert!(!tree.clone_if_shared);
        assert_eq!(tree.entries.iter().map(|e| (e.lv, e.skill_id)).collect::<Vec<_>>(), [(0, 50), (1, 100), (5, 101)]);
        assert!(tree.entries.iter().all(|e| e.skill_tree_id == 10));
    }

    #[test]
    fn deleting_and_re_adding_an_identical_row_is_not_a_change() {
        let mut session = loaded();
        let uid = session.tree(T11).unwrap().rows[1].uid;
        session.delete_row(T11, uid);
        let added = session.add_row(T11, 101).unwrap();
        session.set_field(T11, added, EntryField::Lv, 5);

        assert!(!session.has_changes());
        assert!(session.tree_changes().is_empty());
    }

    #[test]
    fn tree_changes_are_matched_by_row_identity() {
        let mut session = loaded();
        let first = first_uid(&session, T11);
        let second = session.tree(T11).unwrap().rows[1].uid;
        session.set_field(T11, first, EntryField::JobLv, 40);
        session.delete_row(T11, second);
        session.add_row(T11, 777);

        let changes = session.tree_changes();
        let kinds: Vec<_> = changes.iter().map(|c| c.kind).collect();
        assert_eq!(kinds, [TreeChangeKind::Update, TreeChangeKind::Insert, TreeChangeKind::Delete]);
        assert_eq!(changes[0].before.unwrap().job_lv, 1);
        assert_eq!(changes[0].after.unwrap().job_lv, 40);
        assert_eq!(changes[1].after.unwrap().skill_id, 777);
        assert_eq!(changes[2].before.unwrap().skill_id, 101);
        assert_eq!(session.change_count(), 3);
    }

    #[test]
    fn typing_an_unknown_group_id_creates_a_placeholder_once_the_lookup_misses() {
        let mut session = loaded();
        let uid = first_uid(&session, T11);
        session.set_field(T11, uid, EntryField::SkillGroupId, 77);

        assert_eq!(session.group_state(77), GroupState::Loading);
        assert_eq!(session.take_missing_group_ids(), [77]);
        assert!(session.take_missing_group_ids().is_empty(), "must not ask twice");

        session.ingest_random_groups(&[77], []);
        assert_eq!(session.group_state(77), GroupState::Pending);
        assert_eq!(session.build_commit_request().random_groups, [RandomSkillGroup::empty(77)]);
    }

    #[test]
    fn typing_an_existing_group_id_loads_it_without_staging() {
        let mut session = loaded();
        let uid = first_uid(&session, T11);
        session.set_field(T11, uid, EntryField::SkillGroupId, 8);
        session.take_missing_group_ids();
        session.ingest_random_groups(&[8], [RandomSkillGroup::new(8, [100])]);

        assert_eq!(session.group_state(8), GroupState::Loaded);
        assert!(session.build_commit_request().random_groups.is_empty());
    }

    #[test]
    fn orphaned_placeholder_groups_are_not_saved() {
        let mut session = loaded();
        let uid = first_uid(&session, T11);
        session.set_field(T11, uid, EntryField::SkillGroupId, 77);
        session.ingest_random_groups(&[77], []);
        session.set_field(T11, uid, EntryField::SkillGroupId, 0);

        assert!(session.build_commit_request().random_groups.is_empty());
        assert!(!session.has_changes());
    }

    #[test]
    fn groups_missing_at_load_time_are_reported() {
        let mut bundle = bundle();
        bundle.random_groups.clear();
        let mut session = EditSession::new();
        session.load(bundle);

        assert_eq!(session.group_state(7), GroupState::MissingInDb);
        assert!(session.take_missing_group_ids().is_empty());
    }

    #[test]
    fn attaching_a_group_is_one_undo_step() {
        let mut session = loaded();
        let uid = first_uid(&session, T11);
        assert!(session.attach_group(T11, uid, RandomSkillGroup::new(42, [100, 101])));

        assert_eq!(session.tree(T11).unwrap().row(uid).unwrap().entry.skill_group_id, 42);
        assert_eq!(session.group_state(42), GroupState::Pending);
        assert!(session.reserved_group_ids().contains(&42));

        assert!(session.undo());
        assert_eq!(session.tree(T11).unwrap().row(uid).unwrap().entry.skill_group_id, 0);
        assert_eq!(session.group_state(42), GroupState::Loading);
        assert!(!session.can_undo());
    }

    #[test]
    fn staged_group_overlays_the_database_version() {
        let mut session = loaded();
        session.stage_random_group(RandomSkillGroup::new(7, [900, 100]));

        assert_eq!(session.overlay_group(7).unwrap().skill_ids(), [100, 900]);
        assert_eq!(session.db_group(7).unwrap().skill_ids(), [900]);
        assert_eq!(session.build_commit_request().random_groups.len(), 1);
    }

    #[test]
    fn missing_skill_display_is_requested_once() {
        let mut session = loaded();
        session.add_row(T11, 4242);

        assert_eq!(session.take_missing_skill_ids(), [4242]);
        assert!(session.take_missing_skill_ids().is_empty());
        assert_eq!(session.skill_name(4242), "Skill 4242");

        session.ingest_skill_display([SkillDisplay {
            skill_id: 4242,
            skill_name: "Found".into(),
            icon_file_name: None,
            tooltip_markup: None,
        }]);
        assert_eq!(session.skill_name(4242), "Found");
    }

    #[test]
    fn group_candidates_count_as_referenced_skills() {
        let mut session = loaded();
        session.stage_random_group(RandomSkillGroup::new(7, [900, 901]));
        assert_eq!(session.take_missing_skill_ids(), [901]);
    }

    #[test]
    fn discard_all_restores_the_database_state() {
        let mut session = loaded();
        let uid = first_uid(&session, T11);
        session.set_field(T11, uid, EntryField::Lv, 99);
        session.add_row(T12, 5);
        session.stage_random_group(RandomSkillGroup::new(7, [1]));

        session.discard_all();
        assert!(!session.has_changes());
        assert!(!session.can_undo());
        assert_eq!(session.tree(T11).unwrap().rows.len(), 2);
        assert_eq!(session.group_state(7), GroupState::Loaded);
    }

    #[test]
    fn trees_sharing_an_id_within_the_chain_are_detected() {
        let mut bundle = bundle();
        bundle.chain.stage2.as_mut().unwrap().skill_tree_ids[0] = 10;
        let mut session = EditSession::new();
        session.load(bundle);

        assert_eq!(session.trees_sharing_id(T11), [TreeRef::new(2, 1)]);
        assert!(session.trees_sharing_id(T12).is_empty());
        assert!(session.trees_sharing_id(T13).is_empty());
    }

    #[test]
    fn loading_again_resets_staged_state_but_keeps_uids_unique() {
        let mut session = loaded();
        let old_uid = first_uid(&session, T11);
        session.add_row(T11, 5);
        session.load(bundle());

        assert!(!session.has_changes());
        assert!(!session.can_undo());
        assert!(first_uid(&session, T11) > old_uid);
    }
}
