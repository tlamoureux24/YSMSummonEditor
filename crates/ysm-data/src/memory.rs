//! In-memory repository with the same semantics as the MSSQL one.
//!
//! It backs the `--demo` mode of the application and the tests, so its
//! `commit` deliberately mirrors the database implementation statement for
//! statement (ID allocation for clones, upserts, delete-and-reinsert trees).

use std::collections::{BTreeMap, HashMap};

use rust_decimal::Decimal;
use ysm_core::commit::{CommitRequest, CommitResult, is_sql_identifier};
use ysm_core::evolution::growth_depth;
use ysm_core::model::{
    CreatureEnhance, LevelBonusStage, LevelBonusUsage, RandomGroupUsage, STAT_COUNT, SkillDisplay, SkillTreeEntry,
    SkillUsage, StatBlock, StatScalingSnapshot, StatUsage, SummonListFilter, SummonListItem, TreeUsage,
};
use ysm_core::random_group::RandomSkillGroup;
use ysm_core::value::{BUSINESS_FIELDS, FieldKind, FieldValue, SkillField, SkillRecord};

use crate::{DataError, Result, SummonRepository};

#[derive(Debug, Clone)]
struct MemSkill {
    display: SkillDisplay,
    fields: Vec<SkillField>,
}

#[derive(Debug, Clone, Default)]
pub struct InMemoryRepository {
    summons: BTreeMap<i32, SummonListItem>,
    skills: BTreeMap<i32, MemSkill>,
    trees: BTreeMap<i32, Vec<SkillTreeEntry>>,
    stats: BTreeMap<i32, StatBlock>,
    bonuses: BTreeMap<i32, [Decimal; STAT_COUNT]>,
    groups: BTreeMap<i32, RandomSkillGroup>,
    enhance: Vec<CreatureEnhance>,
}

impl InMemoryRepository {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_summon(&mut self, summon: SummonListItem) {
        self.summons.insert(summon.summon_id, summon);
    }

    /// Adds a skill with a plausible set of `SkillResource` columns.
    pub fn insert_skill(&mut self, skill_id: i32, name: &str, tooltip_markup: Option<&str>) {
        let icon = format!("icon_skill_{skill_id}.png");
        let mut fields = vec![
            field("id", FieldValue::I32(skill_id)),
            field("text_id", FieldValue::I32(100_000 + skill_id)),
            field("tooltip_id", FieldValue::I32(200_000 + skill_id)),
            field("icon_file_name", FieldValue::Text(icon.clone())),
            field("is_passive", FieldValue::Bool(skill_id % 5 == 0)),
            field("elemental", FieldValue::U8((skill_id % 7) as u8)),
            field("cast_range", FieldValue::I16(((skill_id % 9) * 5) as i16)),
            field("cost_mp", FieldValue::I32(10 + skill_id % 40)),
            field("cost_mp_per_skl", FieldValue::Decimal(Decimal::new(i64::from(skill_id % 30), 1))),
            SkillField { name: "script_text".into(), kind: FieldKind::Text, value: FieldValue::Null },
        ];
        for name in BUSINESS_FIELDS {
            let value = match name {
                "effect_type" => FieldValue::I32(100 + skill_id % 12),
                "state_id" => FieldValue::I32(if skill_id % 3 == 0 { 3000 + skill_id % 50 } else { 0 }),
                "state_level_base" | "state_level_per_skl" => FieldValue::I32(skill_id % 4),
                _ if name.starts_with("delay") => FieldValue::F64(f64::from(skill_id % 6) * 0.5),
                _ if name.starts_with("state_second") => FieldValue::F64(f64::from(skill_id % 5) * 10.0),
                _ => FieldValue::Decimal(Decimal::ZERO),
            };
            fields.push(field(name, value));
        }
        let display = SkillDisplay {
            skill_id,
            skill_name: name.to_owned(),
            icon_file_name: Some(icon),
            tooltip_markup: tooltip_markup.map(str::to_owned),
        };
        self.skills.insert(skill_id, MemSkill { display, fields });
    }

    pub fn insert_tree(&mut self, skill_tree_id: i32, entries: Vec<SkillTreeEntry>) {
        let entries = entries.into_iter().map(|e| SkillTreeEntry { skill_tree_id, ..e }).collect();
        self.trees.insert(skill_tree_id, entries);
    }

    pub fn insert_stat(&mut self, stat: StatBlock) {
        self.stats.insert(stat.id, stat);
    }

    pub fn insert_level_bonus(&mut self, bonus_id: i32, values: [Decimal; STAT_COUNT]) {
        self.bonuses.insert(bonus_id, values);
    }

    pub fn insert_random_group(&mut self, group: RandomSkillGroup) {
        self.groups.insert(group.group_id, group);
    }

    pub fn set_creature_enhance(&mut self, enhance: Vec<CreatureEnhance>) {
        self.enhance = enhance;
    }

    /// Rows of a tree as currently stored (test helper).
    pub fn tree(&self, skill_tree_id: i32) -> Option<&[SkillTreeEntry]> {
        self.trees.get(&skill_tree_id).map(Vec::as_slice)
    }

    /// `(summon, 1-based slot)` for every slot pointing at `tree_id`.
    fn slots_using_tree(&self, tree_id: i32) -> impl Iterator<Item = (&SummonListItem, i32)> {
        self.summons.values().flat_map(move |summon| {
            summon
                .skill_tree_ids
                .iter()
                .enumerate()
                .filter(move |(_, id)| **id == tree_id)
                .map(move |(slot, _)| (summon, slot as i32 + 1))
        })
    }
}

fn field(name: &str, value: FieldValue) -> SkillField {
    let kind = match &value {
        FieldValue::Bool(_) => FieldKind::Bool,
        FieldValue::U8(_) => FieldKind::U8,
        FieldValue::I16(_) => FieldKind::I16,
        FieldValue::I32(_) => FieldKind::I32,
        FieldValue::I64(_) => FieldKind::I64,
        FieldValue::F32(_) => FieldKind::F32,
        FieldValue::F64(_) => FieldKind::F64,
        FieldValue::Decimal(_) => FieldKind::Decimal,
        FieldValue::Text(_) | FieldValue::Null => FieldKind::Text,
        FieldValue::Unsupported(_) => FieldKind::Unsupported,
    };
    SkillField { name: name.to_owned(), kind, value }
}

fn contains_ignore_case(haystack: &str, needle_lower: &str) -> bool {
    haystack.to_lowercase().contains(needle_lower)
}

impl SummonRepository for InMemoryRepository {
    fn test_connection(&mut self) -> Result<String> {
        Ok(format!("In-memory demo data ({} summons, {} skills)", self.summons.len(), self.skills.len()))
    }

    fn summon_by_id(&mut self, summon_id: i32) -> Result<Option<SummonListItem>> {
        Ok(self.summons.get(&summon_id).cloned())
    }

    fn summons_by_card_id(&mut self, card_id: i32) -> Result<Vec<SummonListItem>> {
        Ok(self.summons.values().filter(|s| s.card_id == card_id).cloned().collect())
    }

    fn search_summons(&mut self, filter: &SummonListFilter) -> Result<Vec<SummonListItem>> {
        let search = filter.search_text.trim().to_lowercase();
        let exact_id: Option<i32> = search.parse().ok();
        let matches_id_or_name = |summon: &SummonListItem| {
            Some(summon.summon_id) == exact_id
                || summon.summon_id.to_string().contains(&search)
                || contains_ignore_case(&summon.summon_name, &search)
        };
        let mut previous_matches = HashMap::new();
        if !filter.show_all_evolutions && !search.is_empty() {
            for previous in self.summons.values().filter(|s| s.evolve_target != 0 && matches_id_or_name(s)) {
                let Some(next) = self.summons.get(&previous.evolve_target).filter(|s| s.card_id == previous.card_id)
                else {
                    continue;
                };
                let final_id = if next.evolve_target == 0 { next.summon_id } else { next.evolve_target };
                *previous_matches.entry((final_id, previous.card_id)).or_insert(false) |=
                    Some(previous.summon_id) == exact_id;
            }
        }

        let mut found: Vec<SummonListItem> = self
            .summons
            .values()
            .filter(|s| filter.include_non_form1 || s.form == 1)
            .filter(|s| filter.include_riding_only || !s.is_riding_only)
            .filter(|s| filter.show_all_evolutions || s.evolve_target == 0)
            .filter(|s| {
                search.is_empty()
                    || matches_id_or_name(s)
                    || s.card_name.as_deref().is_some_and(|card| contains_ignore_case(card, &search))
                    || previous_matches.contains_key(&(s.summon_id, s.card_id))
            })
            .cloned()
            .collect();
        found.sort_by_key(|s| {
            let exact = Some(s.summon_id) == exact_id || previous_matches.get(&(s.summon_id, s.card_id)) == Some(&true);
            (!exact, s.summon_id)
        });
        Ok(found)
    }

    fn skill_tree_entries(&mut self, skill_tree_id: i32) -> Result<Vec<SkillTreeEntry>> {
        let mut entries = self.trees.get(&skill_tree_id).cloned().unwrap_or_default();
        entries.sort_by_key(SkillTreeEntry::sort_key);
        Ok(entries)
    }

    fn stat_scaling(&mut self, summon_id: i32) -> Result<StatScalingSnapshot> {
        let summon = self.summons.get(&summon_id).ok_or_else(|| DataError::NotFound(format!("Summon {summon_id}")))?;
        let depth = growth_depth(summon.form);
        let stage_bonuses = (1..=depth)
            .map(|stage_index| {
                let lookup_id = summon_id - (depth - stage_index);
                let stored = self.bonuses.get(&lookup_id);
                LevelBonusStage {
                    stage_index,
                    lookup_id,
                    values: stored.copied().unwrap_or([Decimal::ZERO; STAT_COUNT]),
                    exists_in_db: stored.is_some(),
                }
            })
            .collect();

        Ok(StatScalingSnapshot {
            summon_id,
            stat_id: summon.stat_id,
            evolve_type: summon.form,
            growth_depth: depth,
            base_stats: self.stats.get(&summon.stat_id).copied().unwrap_or(StatBlock::zero(summon.stat_id)),
            stage_bonuses,
        })
    }

    fn creature_enhance(&mut self) -> Result<Vec<CreatureEnhance>> {
        Ok(self.enhance.clone())
    }

    fn skill_record(&mut self, skill_id: i32) -> Result<SkillRecord> {
        let skill = self.skills.get(&skill_id).ok_or_else(|| DataError::NotFound(format!("Skill {skill_id}")))?;
        Ok(SkillRecord { skill_id, fields: skill.fields.clone() })
    }

    fn skill_display(&mut self, skill_ids: &[i32]) -> Result<HashMap<i32, SkillDisplay>> {
        Ok(skill_ids
            .iter()
            .filter_map(|id| self.skills.get(id))
            .map(|s| (s.display.skill_id, s.display.clone()))
            .collect())
    }

    fn search_skills(&mut self, text: &str, limit: usize) -> Result<Vec<SkillDisplay>> {
        let search = text.trim().to_lowercase();
        let exact_id: Option<i32> = search.parse().ok();
        let mut found: Vec<SkillDisplay> = self
            .skills
            .values()
            .map(|s| &s.display)
            .filter(|d| {
                search.is_empty()
                    || d.skill_id.to_string().contains(&search)
                    || contains_ignore_case(&d.skill_name, &search)
            })
            .cloned()
            .collect();
        found.sort_by_key(|d| (Some(d.skill_id) != exact_id, d.skill_id));
        found.truncate(limit.clamp(1, crate::repository::SKILL_SEARCH_LIMIT));
        Ok(found)
    }

    fn random_groups(&mut self, group_ids: &[i32]) -> Result<HashMap<i32, RandomSkillGroup>> {
        Ok(group_ids.iter().filter_map(|id| self.groups.get(id)).map(|g| (g.group_id, g.clone())).collect())
    }

    fn next_random_group_id(&mut self) -> Result<i32> {
        let highest_group = self.groups.keys().copied().max().unwrap_or(0);
        let highest_row = self.trees.values().flatten().map(|e| e.skill_group_id).max().unwrap_or(0);
        Ok(highest_group.max(highest_row).max(0) + 1)
    }

    fn skill_usage(&mut self, skill_ids: &[i32]) -> Result<Vec<SkillUsage>> {
        let mut rows = Vec::new();
        for (tree_id, entries) in &self.trees {
            for entry in entries.iter().filter(|e| skill_ids.contains(&e.skill_id)) {
                for (summon, _) in self.slots_using_tree(*tree_id) {
                    rows.push(SkillUsage {
                        skill_id: entry.skill_id,
                        skill_tree_id: *tree_id,
                        summon_id: summon.summon_id,
                        summon_name: summon.summon_name.clone(),
                    });
                }
            }
        }
        rows.sort_by_key(|r| (r.skill_id, r.summon_id));
        Ok(rows)
    }

    fn tree_usage(&mut self, skill_tree_ids: &[i32]) -> Result<Vec<TreeUsage>> {
        let mut rows: Vec<TreeUsage> = skill_tree_ids
            .iter()
            .flat_map(|tree_id| {
                self.slots_using_tree(*tree_id).map(|(summon, slot)| TreeUsage {
                    skill_tree_id: *tree_id,
                    summon_id: summon.summon_id,
                    summon_name: summon.summon_name.clone(),
                    tree_slot: slot,
                })
            })
            .collect();
        rows.sort_by_key(|r| (r.skill_tree_id, r.summon_id));
        Ok(rows)
    }

    fn stat_usage(&mut self, stat_ids: &[i32]) -> Result<Vec<StatUsage>> {
        let mut rows: Vec<StatUsage> = self
            .summons
            .values()
            .filter(|s| stat_ids.contains(&s.stat_id))
            .map(|s| StatUsage { stat_id: s.stat_id, summon_id: s.summon_id, summon_name: s.summon_name.clone() })
            .collect();
        rows.sort_by_key(|r| (r.stat_id, r.summon_id));
        Ok(rows)
    }

    fn level_bonus_usage(&mut self, bonus_ids: &[i32]) -> Result<Vec<LevelBonusUsage>> {
        let mut rows = Vec::new();
        for summon in self.summons.values() {
            let depth = growth_depth(summon.form);
            for stage_index in 1..=depth {
                let bonus_id = summon.summon_id - (depth - stage_index);
                if bonus_ids.contains(&bonus_id) {
                    rows.push(LevelBonusUsage {
                        bonus_id,
                        summon_id: summon.summon_id,
                        summon_name: summon.summon_name.clone(),
                        stage_index,
                    });
                }
            }
        }
        rows.sort_by_key(|r| (r.bonus_id, r.summon_id, r.stage_index));
        Ok(rows)
    }

    fn random_group_usage(&mut self, group_ids: &[i32]) -> Result<Vec<RandomGroupUsage>> {
        let mut rows = Vec::new();
        for (tree_id, entries) in &self.trees {
            for entry in entries.iter().filter(|e| group_ids.contains(&e.skill_group_id)) {
                for (summon, slot) in self.slots_using_tree(*tree_id) {
                    rows.push(RandomGroupUsage {
                        group_id: entry.skill_group_id,
                        skill_tree_id: *tree_id,
                        summon_id: summon.summon_id,
                        summon_name: summon.summon_name.clone(),
                        tree_slot: slot,
                    });
                }
            }
        }
        rows.sort_by_key(|r| (r.group_id, r.summon_id, r.tree_slot));
        Ok(rows)
    }

    fn commit(&mut self, request: &CommitRequest) -> Result<CommitResult> {
        // Work on a copy so a failure leaves the data untouched, like a rolled back transaction.
        let mut next = self.clone();
        let mut result = CommitResult::default();

        for tree in &request.trees {
            let target_id = if tree.clone_if_shared {
                let new_id = next.trees.keys().copied().max().unwrap_or(0).max(0) + 1;
                let slot = usize::from(tree.slot);
                if !(1..=5).contains(&slot) {
                    return Err(DataError::Invalid(format!("Tree slot must be in range 1..5, got {slot}.")));
                }
                if let Some(summon) = next.summons.get_mut(&tree.summon_id) {
                    summon.skill_tree_ids[slot - 1] = new_id;
                }
                new_id
            } else {
                tree.original_tree_id
            };
            next.insert_tree(target_id, tree.entries.clone());
            result.updated_trees += 1;
        }

        for stat in request.stats.iter().filter(|s| s.original_stat_id > 0 && s.summon_id > 0) {
            if stat.clone_if_shared {
                let new_id = next.stats.keys().copied().max().unwrap_or(0).max(0) + 1;
                next.stats.insert(new_id, StatBlock { id: new_id, values: stat.stats.values });
                if let Some(summon) = next.summons.get_mut(&stat.summon_id) {
                    summon.stat_id = new_id;
                }
            } else if let Some(existing) = next.stats.get_mut(&stat.original_stat_id) {
                existing.values = stat.stats.values;
            }
        }

        for bonus in request.level_bonuses.iter().filter(|b| b.bonus_id > 0) {
            next.bonuses.insert(bonus.bonus_id, bonus.values);
        }

        for group in request.random_groups.iter().filter(|g| g.group_id > 0) {
            next.groups.insert(group.group_id, group.clone());
            result.updated_random_groups += 1;
        }

        for update in &request.skills {
            let skill = next
                .skills
                .get_mut(&update.skill_id)
                .ok_or_else(|| DataError::NotFound(format!("Skill {}", update.skill_id)))?;
            let mut written = 0;
            for change in &update.changes {
                if !is_sql_identifier(&change.name) || change.name.eq_ignore_ascii_case("id") {
                    continue;
                }
                if let Some(field) = skill.fields.iter_mut().find(|f| f.name.eq_ignore_ascii_case(&change.name)) {
                    field.value = change.new.clone();
                    written += 1;
                }
            }
            if written > 0 {
                result.updated_skills += 1;
            }
        }

        *self = next;
        Ok(result)
    }
}

// ---- demo data -------------------------------------------------------------

impl InMemoryRepository {
    /// A small, self-consistent data set that exercises every editor feature:
    /// three-stage chains, a tree shared inside a chain, a tree, a stat row
    /// and a random group shared across chains, and a missing bonus row.
    pub fn demo() -> Self {
        let mut repo = Self::new();
        let dec = |mantissa: i64, scale: u32| Decimal::new(mantissa, scale);

        const SKILLS: [(i32, &str, &str); 24] = [
            (40001, "Arcane Ward", "Raises magic defense of the summon and its master."),
            (40002, "Mana Surge", "Raises magic attack for a short time."),
            (40003, "Ember Lance", "Hurls a lance of fire at one enemy."),
            (40004, "Flame Veil", "Wraps the caster in flames that burn attackers."),
            (40005, "Cinder Burst", "Explodes, damaging every enemy around the target."),
            (40006, "Healing Tide", "Restores HP of one ally."),
            (40007, "Mist Shroud", "Raises evasion of nearby allies."),
            (40008, "Riptide", "Pulls an enemy towards the caster."),
            (40009, "Gale Wing", "Raises movement and attack speed."),
            (40010, "Talon Strike", "A quick physical attack with a chance to bleed."),
            (40011, "Skyfall", "Dives from above for heavy damage."),
            (40012, "Bark Skin", "Raises physical defense."),
            (40013, "Root Snare", "Roots one enemy in place."),
            (40014, "Verdant Bloom", "Heals all party members over time."),
            (40015, "Frost Breath", "Slows enemies in a cone."),
            (40016, "Glacial Armor", "Absorbs damage with a shell of ice."),
            (40017, "Stone Fist", "A heavy blow that can stun."),
            (40018, "Bulwark", "Taunts enemies and raises block rate."),
            (40019, "Blessing of Strength", "Raises strength of the master."),
            (40020, "Blessing of Vitality", "Raises vitality of the master."),
            (40021, "Blessing of Agility", "Raises agility of the master."),
            (40022, "Blessing of Wisdom", "Raises wisdom of the master."),
            (40023, "Unity", "Shares a part of the summon's stats with its master."),
            (40024, "Creature Mastery", "Raises the effect of creature equipment."),
        ];
        for (id, name, description) in SKILLS {
            let tooltip = format!(
                "<#FFD166><B>{name}</B></#FFD166><BR>{description}<BR><BR><#8ECAE6>Cooldown: {}s</#8ECAE6>  <I>MP {}</I>",
                5 + id % 20,
                10 + id % 40
            );
            repo.insert_skill(id, name, Some(&tooltip));
        }

        repo.set_creature_enhance(
            (0..=5)
                .map(|level| CreatureEnhance { enhance_level: level, stat_amplify: dec(i64::from(level) * 4, 2) })
                .collect(),
        );

        let row = |skill_id: i32, lv: i32, job_lv: i32, max: i32| SkillTreeEntry {
            skill_id,
            job_lv,
            max_skill_lv: max,
            ..SkillTreeEntry::new_row(0, lv)
        };

        // Tree shared by two families, to trigger the "shared tree" question.
        const SHARED_TREE: i32 = 9000;
        repo.insert_tree(
            SHARED_TREE,
            vec![row(40019, 0, 51, 10), row(40020, 0, 51, 10), row(40021, 0, 51, 10), row(40022, 0, 51, 10)],
        );
        // Random group shared by the same two families.
        const SHARED_GROUP: i32 = 301;
        repo.insert_random_group(RandomSkillGroup::new(SHARED_GROUP, [40019, 40020, 40021, 40022]));
        repo.insert_random_group(RandomSkillGroup::new(302, [40009, 40010]));

        struct Family {
            base_id: i32,
            names: [&'static str; 3],
            feature: &'static str,
            skills: [i32; 3],
            shared: bool,
        }
        let families = [
            Family {
                base_id: 1011,
                names: ["Ember Sprite", "Blaze Sprite", "Inferno Seraph"],
                feature: "A basic creature specialized in fire magic.",
                skills: [40003, 40004, 40005],
                shared: true,
            },
            Family {
                base_id: 1021,
                names: ["Tide Sprite", "Aqua Sprite", "Ocean Seraph"],
                feature: "A basic creature specialized in healing magic.",
                skills: [40006, 40007, 40008],
                shared: true,
            },
            Family {
                base_id: 1201,
                names: ["Fledgling Hawk", "Storm Hawk", "Tempest Roc"],
                feature: "A swift creature that strikes from the sky.",
                skills: [40009, 40010, 40011],
                shared: false,
            },
            Family {
                base_id: 1401,
                names: ["Sapling Treant", "Elder Treant", "Ancient Treant"],
                feature: "A sturdy creature that protects its allies.",
                skills: [40012, 40013, 40014],
                shared: false,
            },
            Family {
                base_id: 1501,
                names: ["Frost Whelp", "Glacier Drake", "Rime Wyrm"],
                feature: "A rare creature commanding ice.",
                skills: [40015, 40016, 40001],
                shared: false,
            },
            Family {
                base_id: 1601,
                names: ["Stone Imp", "Granite Golem", "Obsidian Colossus"],
                feature: "A rare creature with overwhelming defense.",
                skills: [40017, 40018, 40002],
                shared: false,
            },
        ];

        for (family_index, family) in families.iter().enumerate() {
            let card_id = 540_001 + family_index as i32;
            // One tree is reused by all three stages of the family.
            let family_tree = family.base_id * 10;
            repo.insert_tree(family_tree, vec![row(40023, 0, 1, 1), row(40024, 0, 20, 5)]);

            for stage in 0..3 {
                let summon_id = family.base_id + stage;
                let form = stage + 1;
                let own_tree = summon_id * 10 + 1;
                let mut rows: Vec<SkillTreeEntry> = (0..=stage as usize)
                    .flat_map(|i| {
                        let skill = family.skills[i];
                        // The same skill appears once per enhance tier, like in real data.
                        (0..3).map(move |tier| SkillTreeEntry {
                            min_skill_lv: tier + 1,
                            max_skill_lv: tier + 1,
                            cenhance_min: tier,
                            cenhance_max: 5,
                            ..row(skill, 0, 10 * (i as i32 + 1), 1)
                        })
                    })
                    .collect();
                rows.push(row(40001, 0, 1, 10));
                rows.push(row(40002, 0, 1, 10));
                if stage == 2 {
                    let group = if family.shared { SHARED_GROUP } else { 302 };
                    if family.shared || family.base_id == 1201 {
                        rows.push(SkillTreeEntry { skill_group_id: group, ..row(40019, 1, 60, 5) });
                    }
                }
                repo.insert_tree(own_tree, rows);

                let stat_id = if family.shared && stage == 0 { 5000 } else { summon_id };
                repo.insert_summon(SummonListItem {
                    summon_id,
                    summon_name: family.names[stage as usize].to_owned(),
                    card_name: Some(format!("{} Card", family.names[0])),
                    card_id,
                    evolve_target: if stage < 2 { summon_id + 1 } else { 0 },
                    form,
                    is_riding_only: false,
                    face_file_name: Some(format!("face_{summon_id}.png")),
                    illust_file_name: Some(format!("illust_{summon_id}.jpg")),
                    feature_text: Some(family.feature.to_owned()),
                    stat_id,
                    skill_tree_ids: [own_tree, 0, family_tree, if family.shared { SHARED_TREE } else { 0 }, 0],
                });
                repo.insert_stat(StatBlock {
                    id: summon_id,
                    values: std::array::from_fn(|i| 20 + family_index as i32 * 3 + stage * 12 + i as i32 * 2),
                });
                // The last family has no bonus row for its final stage, to show the "missing" state.
                if !(family.base_id == 1601 && stage == 2) {
                    repo.insert_level_bonus(
                        summon_id,
                        std::array::from_fn(|i| {
                            dec(30 + i64::from(stage) * 15 + i as i64 * 5 + family_index as i64, 2)
                        }),
                    );
                }
            }
        }
        // Stat row shared by the first stages of the two "shared" families.
        repo.insert_stat(StatBlock { id: 5000, values: [18, 16, 15, 17, 22, 21, 12] });

        // A riding-only summon and a summon without earlier stages.
        repo.insert_tree(17011, vec![row(40009, 0, 1, 5)]);
        repo.insert_summon(SummonListItem {
            summon_id: 1701,
            summon_name: "Dusk Colt".into(),
            card_name: Some("Dusk Colt Card".into()),
            card_id: 540_101,
            evolve_target: 0,
            form: 1,
            is_riding_only: true,
            face_file_name: Some("face_1701.png".into()),
            illust_file_name: None,
            feature_text: Some("A mount. It cannot fight.".into()),
            stat_id: 1701,
            skill_tree_ids: [17011, 0, 0, 0, 0],
        });
        repo.insert_stat(StatBlock { id: 1701, values: [10; STAT_COUNT] });

        repo.insert_tree(20011, vec![row(40006, 0, 1, 10), row(40014, 0, 30, 5), row(49999, 0, 50, 1)]);
        repo.insert_summon(SummonListItem {
            summon_id: 2001,
            summon_name: "Lone Wisp".into(),
            card_name: None,
            card_id: 540_201,
            evolve_target: 0,
            form: 1,
            is_riding_only: false,
            face_file_name: None,
            illust_file_name: None,
            feature_text: None,
            stat_id: 2001,
            skill_tree_ids: [0, 0, 0, 0, 0],
        });
        // Its only tree sits in slot 1; set after insert to keep the literal above simple.
        if let Some(wisp) = repo.summons.get_mut(&2001) {
            wisp.skill_tree_ids[0] = 20011;
        }
        repo.insert_stat(StatBlock { id: 2001, values: [14, 13, 12, 11, 25, 24, 9] });
        repo.insert_level_bonus(2001, [dec(45, 2); STAT_COUNT]);

        repo
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_only_search_finds_and_ranks_the_chain_from_an_earlier_stage() {
        let mut repo = InMemoryRepository::demo();
        let filter = |search: &str| SummonListFilter {
            show_all_evolutions: false,
            search_text: search.to_owned(),
            ..Default::default()
        };
        let by_id = repo.search_summons(&filter("1011")).unwrap();
        assert_eq!(by_id[0].summon_id, 1013);
        let earlier_name = repo.summon_by_id(1012).unwrap().unwrap().summon_name;
        let by_name = repo.search_summons(&filter(&earlier_name)).unwrap();
        assert!(by_name.iter().any(|summon| summon.summon_id == 1013));
        let all = repo.search_summons(&filter("")).unwrap();
        assert!(all.iter().all(|summon| summon.evolve_target == 0));
        assert!(all.iter().any(|summon| summon.form == 1));
    }
    use ysm_core::commit::{FieldChange, LevelBonusUpsert, SkillUpdate, StatCommit, TreeCommit};

    fn tree_commit(repo: &mut InMemoryRepository, summon_id: i32, slot: u8, clone: bool) -> TreeCommit {
        let summon = repo.summon_by_id(summon_id).unwrap().unwrap();
        let tree_id = summon.skill_tree_ids[usize::from(slot) - 1];
        let mut entries = repo.skill_tree_entries(tree_id).unwrap();
        entries.truncate(1);
        TreeCommit { summon_id, slot, original_tree_id: tree_id, entries, clone_if_shared: clone }
    }

    #[test]
    fn in_place_tree_commit_replaces_the_rows() {
        let mut repo = InMemoryRepository::demo();
        let commit = tree_commit(&mut repo, 1013, 4, false);
        let result = repo.commit(&CommitRequest { trees: vec![commit], ..Default::default() }).unwrap();

        assert_eq!(result.updated_trees, 1);
        assert_eq!(repo.tree(9000).unwrap().len(), 1);
        assert_eq!(repo.summon_by_id(1023).unwrap().unwrap().skill_tree_ids[3], 9000, "other users see the edit");
    }

    #[test]
    fn cloned_tree_commit_repoints_only_the_edited_summon() {
        let mut repo = InMemoryRepository::demo();
        let original_len = repo.tree(9000).unwrap().len();
        let commit = tree_commit(&mut repo, 1013, 4, true);
        repo.commit(&CommitRequest { trees: vec![commit], ..Default::default() }).unwrap();

        let new_id = repo.summon_by_id(1013).unwrap().unwrap().skill_tree_ids[3];
        assert_ne!(new_id, 9000);
        assert_eq!(repo.tree(new_id).unwrap().len(), 1);
        assert!(repo.tree(new_id).unwrap().iter().all(|e| e.skill_tree_id == new_id));
        assert_eq!(repo.tree(9000).unwrap().len(), original_len, "the shared tree is untouched");
        assert_eq!(repo.summon_by_id(1023).unwrap().unwrap().skill_tree_ids[3], 9000);
    }

    #[test]
    fn stat_commit_updates_in_place_or_clones() {
        let mut repo = InMemoryRepository::demo();
        let stats = StatBlock { id: 5000, values: [1, 2, 3, 4, 5, 6, 7] };
        let request = |clone| CommitRequest {
            stats: vec![StatCommit { summon_id: 1011, original_stat_id: 5000, stats, clone_if_shared: clone }],
            ..Default::default()
        };

        repo.commit(&request(false)).unwrap();
        assert_eq!(repo.stat_scaling(1021).unwrap().base_stats.values, stats.values, "shared row changed for both");

        let mut repo = InMemoryRepository::demo();
        repo.commit(&request(true)).unwrap();
        let cloned = repo.stat_scaling(1011).unwrap();
        assert_ne!(cloned.stat_id, 5000);
        assert_eq!(cloned.base_stats.values, stats.values);
        assert_eq!(repo.stat_scaling(1021).unwrap().base_stats.values, [18, 16, 15, 17, 22, 21, 12]);
    }

    #[test]
    fn bonus_and_group_upserts_insert_missing_rows() {
        let mut repo = InMemoryRepository::demo();
        assert!(!repo.stat_scaling(1603).unwrap().stage_bonuses[2].exists_in_db);

        let request = CommitRequest {
            level_bonuses: vec![LevelBonusUpsert { bonus_id: 1603, values: [Decimal::ONE; STAT_COUNT] }],
            random_groups: vec![RandomSkillGroup::new(777, [40001, 40002])],
            ..Default::default()
        };
        let result = repo.commit(&request).unwrap();

        assert_eq!(result.updated_random_groups, 1);
        let stage = &repo.stat_scaling(1603).unwrap().stage_bonuses[2];
        assert!(stage.exists_in_db);
        assert_eq!(stage.values[0], Decimal::ONE);
        assert_eq!(repo.random_groups(&[777]).unwrap()[&777].skill_ids(), [40001, 40002]);
        assert_eq!(repo.next_random_group_id().unwrap(), 778);
    }

    #[test]
    fn skill_update_writes_known_columns_only() {
        let mut repo = InMemoryRepository::demo();
        let change =
            |name: &str, new| FieldChange { name: name.into(), kind: FieldKind::Decimal, old: FieldValue::Null, new };
        let request = CommitRequest {
            skills: vec![SkillUpdate {
                skill_id: 40003,
                changes: vec![
                    change("var1", FieldValue::Decimal(Decimal::new(125, 2))),
                    change("id", FieldValue::I32(1)),
                    change("no_such_column", FieldValue::I32(1)),
                ],
            }],
            ..Default::default()
        };
        let result = repo.commit(&request).unwrap();

        assert_eq!(result.updated_skills, 1);
        let record = repo.skill_record(40003).unwrap();
        assert_eq!(record.field("var1").unwrap().value, FieldValue::Decimal(Decimal::new(125, 2)));
        assert_eq!(record.field("id").unwrap().value, FieldValue::I32(40003));
    }

    #[test]
    fn failed_commit_changes_nothing() {
        let mut repo = InMemoryRepository::demo();
        let request = CommitRequest {
            random_groups: vec![RandomSkillGroup::new(888, [40001])],
            skills: vec![SkillUpdate { skill_id: -1, changes: vec![] }],
            ..Default::default()
        };

        assert!(repo.commit(&request).is_err());
        assert!(repo.random_groups(&[888]).unwrap().is_empty());
    }

    #[test]
    fn usage_queries_report_every_slot() {
        let mut repo = InMemoryRepository::demo();

        let trees = repo.tree_usage(&[9000]).unwrap();
        assert_eq!(trees.len(), 6, "two families, three stages each");
        assert!(trees.iter().all(|u| u.tree_slot == 4));

        let groups = repo.random_group_usage(&[301]).unwrap();
        assert_eq!(groups.iter().map(|u| u.summon_id).collect::<Vec<_>>(), [1013, 1023]);

        let stats = repo.stat_usage(&[5000]).unwrap();
        assert_eq!(stats.iter().map(|u| u.summon_id).collect::<Vec<_>>(), [1011, 1021]);

        let bonuses = repo.level_bonus_usage(&[1011]).unwrap();
        assert_eq!(
            bonuses.iter().map(|u| (u.summon_id, u.stage_index)).collect::<Vec<_>>(),
            [(1011, 1), (1012, 1), (1013, 1)]
        );

        let skills = repo.skill_usage(&[40003]).unwrap();
        assert_eq!(
            skills.iter().map(|u| u.summon_id).collect::<Vec<_>>(),
            [1011, 1011, 1011, 1012, 1012, 1012, 1013, 1013, 1013]
        );
    }

    #[test]
    fn filters_follow_the_sql_semantics() {
        let mut repo = InMemoryRepository::demo();
        let count =
            |repo: &mut InMemoryRepository, filter: SummonListFilter| repo.search_summons(&filter).unwrap().len();

        assert_eq!(count(&mut repo, SummonListFilter::default()), 19, "riding-only summons are hidden by default");
        assert_eq!(count(&mut repo, SummonListFilter { include_riding_only: true, ..Default::default() }), 20);
        assert_eq!(count(&mut repo, SummonListFilter { show_all_evolutions: false, ..Default::default() }), 7);
        assert_eq!(count(&mut repo, SummonListFilter { include_non_form1: false, ..Default::default() }), 7);
    }

    #[test]
    fn skill_search_puts_the_exact_id_first_and_honours_the_limit() {
        let mut repo = InMemoryRepository::demo();
        assert_eq!(repo.search_skills("blessing", 300).unwrap().len(), 4);
        assert_eq!(repo.search_skills("", 5).unwrap().len(), 5);
        assert_eq!(repo.search_skills("40010", 300).unwrap()[0].skill_id, 40010);
    }

    #[test]
    fn scaling_uses_lookup_ids_below_the_summon_id() {
        let mut repo = InMemoryRepository::demo();
        let snapshot = repo.stat_scaling(1013).unwrap();

        assert_eq!(snapshot.growth_depth, 3);
        assert_eq!(snapshot.stage_bonuses.iter().map(|s| s.lookup_id).collect::<Vec<_>>(), [1011, 1012, 1013]);
        assert!(repo.stat_scaling(424_242).is_err());
    }
}
