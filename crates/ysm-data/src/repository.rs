use std::collections::HashMap;

use ysm_core::commit::{CommitRequest, CommitResult};
use ysm_core::i18n::Language;
use ysm_core::model::{
    CreatureEnhance, LevelBonusUsage, RandomGroupUsage, SkillDisplay, SkillTreeEntry, SkillUsage, StatScalingSnapshot,
    StatUsage, SummonListFilter, SummonListItem, TreeUsage,
};
use ysm_core::random_group::RandomSkillGroup;
use ysm_core::value::SkillRecord;

use crate::Result;

/// Upper bound on skill search results.
pub const SKILL_SEARCH_LIMIT: usize = 300;

/// Read and write access to summon data.
///
/// Methods block; callers run them off the UI thread. Lookups by a list of
/// IDs return an empty result for an empty list without touching the database.
pub trait SummonRepository: Send {
    /// Language of the game texts read from now on (names, tooltips). A
    /// repository with a single text table ignores it.
    fn set_text_language(&mut self, _language: Language) {}

    /// Verifies the connection and returns a short description of the server.
    fn test_connection(&mut self) -> Result<String>;

    fn summon_by_id(&mut self, summon_id: i32) -> Result<Option<SummonListItem>>;
    fn summons_by_card_id(&mut self, card_id: i32) -> Result<Vec<SummonListItem>>;
    fn search_summons(&mut self, filter: &SummonListFilter) -> Result<Vec<SummonListItem>>;

    /// Rows of one skill tree, ordered by level then skill ID.
    fn skill_tree_entries(&mut self, skill_tree_id: i32) -> Result<Vec<SkillTreeEntry>>;

    fn stat_scaling(&mut self, summon_id: i32) -> Result<StatScalingSnapshot>;
    fn creature_enhance(&mut self) -> Result<Vec<CreatureEnhance>>;

    /// The full `SkillResource` row with column types.
    fn skill_record(&mut self, skill_id: i32) -> Result<SkillRecord>;
    fn skill_display(&mut self, skill_ids: &[i32]) -> Result<HashMap<i32, SkillDisplay>>;
    /// Skills whose ID or name contains `text`; an exact ID match comes first.
    fn search_skills(&mut self, text: &str, limit: usize) -> Result<Vec<SkillDisplay>>;

    fn random_groups(&mut self, group_ids: &[i32]) -> Result<HashMap<i32, RandomSkillGroup>>;
    /// One past the highest group ID used by any group or tree row.
    fn next_random_group_id(&mut self) -> Result<i32>;

    fn skill_usage(&mut self, skill_ids: &[i32]) -> Result<Vec<SkillUsage>>;
    fn tree_usage(&mut self, skill_tree_ids: &[i32]) -> Result<Vec<TreeUsage>>;
    fn stat_usage(&mut self, stat_ids: &[i32]) -> Result<Vec<StatUsage>>;
    fn level_bonus_usage(&mut self, bonus_ids: &[i32]) -> Result<Vec<LevelBonusUsage>>;
    fn random_group_usage(&mut self, group_ids: &[i32]) -> Result<Vec<RandomGroupUsage>>;

    /// Applies the whole request atomically.
    fn commit(&mut self, request: &CommitRequest) -> Result<CommitResult>;
}
