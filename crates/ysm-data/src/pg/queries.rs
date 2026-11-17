//! Read queries: the MSSQL statements of `mssql/queries.rs` translated to
//! PostgreSQL. Table names are unquoted (PostgreSQL folds them to lower
//! case), searches ignore case like the usual MSSQL collations do, and every
//! list ends on enough columns to make its order fixed.

use std::collections::HashMap;

use rust_decimal::Decimal;
use tokio_postgres::{Client, Row};
use ysm_core::evolution::growth_depth;
use ysm_core::model::{
    CreatureEnhance, LevelBonusStage, LevelBonusUsage, RandomGroupUsage, STAT_COUNT, SkillDisplay, SkillTreeEntry,
    SkillUsage, StatBlock, StatScalingSnapshot, StatUsage, SummonListFilter, SummonListItem, TreeUsage,
};
use ysm_core::random_group::RandomSkillGroup;
use ysm_core::value::{SkillField, SkillRecord};

use super::PgResult;
use super::cells::{Cells, cell, kind_of};

const SUMMON_PROJECTION: &str = "
    SELECT
        s.id,
        COALESCE(sn.value, CONCAT('Summon ', s.id)),
        cn.value,
        s.card_id,
        s.evolve_target,
        s.form,
        s.is_riding_only,
        s.face_file_name,
        s.illust_file_name,
        sf.value,
        s.stat_id,
        s.skill_tree1_id,
        s.skill_tree2_id,
        s.skill_tree3_id,
        s.skill_tree4_id,
        s.skill_tree5_id
    FROM SummonResource s
    LEFT JOIN StringResource sn ON s.name_id = sn.code
    LEFT JOIN StringResource sf ON s.text_feature_id = sf.code
    LEFT JOIN ItemResource ir ON s.card_id = ir.id
    LEFT JOIN StringResource cn ON ir.name_id = cn.code
";

/// Renders IDs as a literal list for `IN (...)`. Safe: the values are integers.
fn in_list(ids: &[i32]) -> String {
    ids.iter().map(i32::to_string).collect::<Vec<_>>().join(", ")
}

fn summon(row: &Row) -> SummonListItem {
    let mut c = Cells::new(row);
    SummonListItem {
        summon_id: c.i32(),
        summon_name: c.string(),
        card_name: c.opt_string(),
        card_id: c.i32(),
        evolve_target: c.i32(),
        form: c.i32(),
        is_riding_only: c.bool(),
        face_file_name: c.opt_string(),
        illust_file_name: c.opt_string(),
        feature_text: c.opt_string(),
        stat_id: c.i32(),
        skill_tree_ids: std::array::from_fn(|_| c.i32()),
    }
}

pub(super) async fn server_version(client: &Client) -> PgResult<String> {
    let row = client.query_one("SHOW server_version", &[]).await?;
    let version: String = row.get(0);
    Ok(format!("PostgreSQL {}", version.split_whitespace().next().unwrap_or_default()))
}

pub(super) async fn summon_by_id(client: &Client, summon_id: i32) -> PgResult<Option<SummonListItem>> {
    let sql = format!("{SUMMON_PROJECTION} WHERE s.id = $1");
    Ok(client.query_opt(&sql, &[&summon_id]).await?.as_ref().map(summon))
}

pub(super) async fn summons_by_card_id(client: &Client, card_id: i32) -> PgResult<Vec<SummonListItem>> {
    let sql = format!("{SUMMON_PROJECTION} WHERE s.card_id = $1 ORDER BY s.id");
    Ok(client.query(&sql, &[&card_id]).await?.iter().map(summon).collect())
}

pub(super) async fn search_summons(client: &Client, filter: &SummonListFilter) -> PgResult<Vec<SummonListItem>> {
    // $1 IncludeNonForm1, $2 IncludeRidingOnly, $3 ShowAllEvolutions,
    // $4 SearchText, $5 ExactSummonId, $6 SearchLike
    let sql = format!(
        "{SUMMON_PROJECTION}
        WHERE ($1::bool OR s.form = 1)
          AND ($2::bool OR s.is_riding_only = 0)
          AND ($3::bool OR s.evolve_target = 0)
          AND (
            $4::text = ''
            OR ($5::int4 IS NOT NULL AND s.id = $5)
            OR s.id::text LIKE $6::text
            OR COALESCE(sn.value, '') ILIKE $6
            OR COALESCE(cn.value, '') ILIKE $6
          )
        ORDER BY
            CASE WHEN $5 IS NOT NULL AND s.id = $5 THEN 0 ELSE 1 END,
            s.id"
    );

    let search = filter.search_text.trim();
    let exact_id: Option<i32> = search.parse().ok();
    let like = format!("%{search}%");
    let rows = client
        .query(
            &sql,
            &[
                &filter.include_non_form1,
                &filter.include_riding_only,
                &filter.show_all_evolutions,
                &search,
                &exact_id,
                &like.as_str(),
            ],
        )
        .await?;
    Ok(rows.iter().map(summon).collect())
}

pub(super) async fn skill_tree_entries(client: &Client, skill_tree_id: i32) -> PgResult<Vec<SkillTreeEntry>> {
    // The table has no key: rows equal on `lv, skill_id` are ordered by the
    // rest of their columns so a reload always lists them the same way.
    const SQL: &str = "
        SELECT
            skill_tree_id, skill_id, skill_group_id, min_skill_lv, max_skill_lv, lv, job_lv,
            need_skill_id_1, need_skill_lv_1, need_skill_id_2, need_skill_lv_2,
            need_skill_id_3, need_skill_lv_3, cenhance_min, cenhance_max, jp_ratio
        FROM SkillTreeResource
        WHERE skill_tree_id = $1
        ORDER BY lv, skill_id, min_skill_lv, max_skill_lv, cenhance_min, cenhance_max, job_lv, skill_group_id,
                 need_skill_id_1, need_skill_lv_1, need_skill_id_2, need_skill_lv_2,
                 need_skill_id_3, need_skill_lv_3, jp_ratio";

    Ok(client
        .query(SQL, &[&skill_tree_id])
        .await?
        .iter()
        .map(|row| {
            let mut c = Cells::new(row);
            let mut entry = SkillTreeEntry {
                skill_tree_id: c.i32(),
                skill_id: c.i32(),
                skill_group_id: c.i32(),
                min_skill_lv: c.i32(),
                max_skill_lv: c.i32(),
                lv: c.i32(),
                job_lv: c.i32(),
                need_skill_id: [0; 3],
                need_skill_lv: [0; 3],
                cenhance_min: 0,
                cenhance_max: 0,
                jp_ratio: Decimal::ONE,
            };
            for i in 0..3 {
                entry.need_skill_id[i] = c.i32();
                entry.need_skill_lv[i] = c.i32();
            }
            entry.cenhance_min = c.i32();
            entry.cenhance_max = c.i32();
            entry.jp_ratio = c.decimal();
            entry
        })
        .collect())
}

/// Returns `None` when the summon does not exist.
pub(super) async fn stat_scaling(client: &Client, summon_id: i32) -> PgResult<Option<StatScalingSnapshot>> {
    const SUMMON_SQL: &str = "SELECT s.id, s.stat_id, s.form FROM SummonResource s WHERE s.id = $1";
    const STAT_SQL: &str = "
        SELECT sr.id, sr.str, sr.vit, sr.dex, sr.agi, sr.\"int\", sr.men, sr.luk
        FROM StatResource sr
        WHERE sr.id = $1";

    let Some(row) = client.query_opt(SUMMON_SQL, &[&summon_id]).await? else {
        return Ok(None);
    };
    let mut c = Cells::new(&row);
    let (summon_id, stat_id, evolve_type) = (c.i32(), c.i32(), c.i32());
    let depth = growth_depth(evolve_type);
    let lookup_ids: Vec<i32> = (1..=depth).map(|stage_index| summon_id - (depth - stage_index)).collect();

    let base_stats = match client.query_opt(STAT_SQL, &[&stat_id]).await? {
        Some(row) => {
            let mut c = Cells::new(&row);
            StatBlock { id: c.i32(), values: std::array::from_fn(|_| c.i32()) }
        }
        None => StatBlock::zero(stat_id),
    };

    let bonus_sql = format!(
        "
        SELECT b.id, b.str, b.vit, b.dex, b.agi, b.\"int\", b.men, b.luk
        FROM CreatureLevelBonus b
        WHERE b.id IN ({})
        ORDER BY b.id",
        in_list(&lookup_ids)
    );
    let mut bonuses: HashMap<i32, [Decimal; STAT_COUNT]> = HashMap::new();
    for row in client.query(&bonus_sql, &[]).await? {
        let mut c = Cells::new(&row);
        bonuses.insert(c.i32(), std::array::from_fn(|_| c.decimal()));
    }

    let stage_bonuses = (1..=depth)
        .zip(lookup_ids)
        .map(|(stage_index, lookup_id)| {
            let stored = bonuses.get(&lookup_id);
            LevelBonusStage {
                stage_index,
                lookup_id,
                values: stored.copied().unwrap_or([Decimal::ZERO; STAT_COUNT]),
                exists_in_db: stored.is_some(),
            }
        })
        .collect();

    Ok(Some(StatScalingSnapshot { summon_id, stat_id, evolve_type, growth_depth: depth, base_stats, stage_bonuses }))
}

pub(super) async fn creature_enhance(client: &Client) -> PgResult<Vec<CreatureEnhance>> {
    const SQL: &str = "SELECT ce.enhance_level, ce.stat_amplify FROM CreatureEnhance ce ORDER BY ce.enhance_level";
    Ok(client
        .query(SQL, &[])
        .await?
        .iter()
        .map(|row| {
            let mut c = Cells::new(row);
            CreatureEnhance { enhance_level: c.i32(), stat_amplify: c.decimal() }
        })
        .collect())
}

/// Returns `None` when the skill does not exist.
pub(super) async fn skill_record(client: &Client, skill_id: i32) -> PgResult<Option<SkillRecord>> {
    const SQL: &str = "SELECT * FROM SkillResource WHERE id = $1";

    let Some(row) = client.query_opt(SQL, &[&skill_id]).await? else {
        return Ok(None);
    };
    let fields = row
        .columns()
        .iter()
        .enumerate()
        .map(|(index, column)| {
            let kind = kind_of(column.type_());
            SkillField { name: column.name().to_owned(), kind, value: cell(&row, index).field(kind) }
        })
        .collect();
    Ok(Some(SkillRecord { skill_id, fields }))
}

const SKILL_DISPLAY_COLUMNS: &str = "
    s.id,
    COALESCE(name_sr.value, CONCAT('Skill ', s.id)),
    NULLIF(BTRIM(s.icon_file_name), ''),
    tip_sr.value
    FROM SkillResource s
    LEFT JOIN StringResource name_sr ON s.text_id = name_sr.code
    LEFT JOIN StringResource tip_sr ON s.tooltip_id = tip_sr.code";

fn skill_display_row(row: &Row) -> SkillDisplay {
    let mut c = Cells::new(row);
    SkillDisplay {
        skill_id: c.i32(),
        skill_name: c.string(),
        icon_file_name: c.opt_string(),
        tooltip_markup: c.opt_string(),
    }
}

pub(super) async fn skill_display(client: &Client, skill_ids: &[i32]) -> PgResult<HashMap<i32, SkillDisplay>> {
    if skill_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let sql = format!("SELECT {SKILL_DISPLAY_COLUMNS} WHERE s.id IN ({}) ORDER BY s.id", in_list(skill_ids));
    Ok(client.query(&sql, &[]).await?.iter().map(skill_display_row).map(|d| (d.skill_id, d)).collect())
}

pub(super) async fn search_skills(client: &Client, text: &str, limit: usize) -> PgResult<Vec<SkillDisplay>> {
    // $1 SearchText, $2 ExactSkillId, $3 SearchLike; the limit is an integer literal.
    let limit = limit.clamp(1, crate::repository::SKILL_SEARCH_LIMIT);
    let sql = format!(
        "SELECT {SKILL_DISPLAY_COLUMNS}
        WHERE
            $1::text = ''
            OR ($2::int4 IS NOT NULL AND s.id = $2)
            OR s.id::text LIKE $3::text
            OR COALESCE(name_sr.value, '') ILIKE $3
        ORDER BY
            CASE WHEN $2 IS NOT NULL AND s.id = $2 THEN 0 ELSE 1 END,
            s.id
        LIMIT {limit}"
    );

    let search = text.trim();
    let exact_id: Option<i32> = search.parse().ok();
    let like = format!("%{search}%");
    Ok(client.query(&sql, &[&search, &exact_id, &like.as_str()]).await?.iter().map(skill_display_row).collect())
}

pub(super) async fn random_groups(client: &Client, group_ids: &[i32]) -> PgResult<HashMap<i32, RandomSkillGroup>> {
    if group_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let sql = format!(
        "
        SELECT r.sid, r.use_code,
               r.skill_id_1, r.skill_id_2, r.skill_id_3, r.skill_id_4, r.skill_id_5,
               r.skill_id_6, r.skill_id_7, r.skill_id_8, r.skill_id_9, r.skill_id_10,
               r.skill_id_11, r.skill_id_12, r.skill_id_13, r.skill_id_14, r.skill_id_15
        FROM SummonRandomSkillResource r
        WHERE r.sid IN ({})
        ORDER BY r.sid",
        in_list(group_ids)
    );
    Ok(client
        .query(&sql, &[])
        .await?
        .iter()
        .map(|row| {
            let mut c = Cells::new(row);
            let (group_id, use_code) = (c.i32(), c.i32());
            let group = RandomSkillGroup::from_db_row(group_id, use_code, std::array::from_fn(|_| c.i32()));
            (group_id, group)
        })
        .collect())
}

pub(super) async fn next_random_group_id(client: &Client) -> PgResult<i32> {
    const SQL: &str = "
        SELECT COALESCE(MAX(v.id), 0) + 1
        FROM (
            SELECT r.sid AS id FROM SummonRandomSkillResource r
            UNION ALL
            SELECT s.skill_group_id AS id FROM SkillTreeResource s WHERE s.skill_group_id > 0
        ) v";
    let row = client.query_one(SQL, &[]).await?;
    Ok(Cells::new(&row).i32().max(1))
}

pub(super) async fn skill_usage(client: &Client, skill_ids: &[i32]) -> PgResult<Vec<SkillUsage>> {
    if skill_ids.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "
        SELECT
            st.skill_id,
            st.skill_tree_id,
            s.id,
            COALESCE(sr.value, CONCAT('Summon ', s.id))
        FROM SkillTreeResource st
        INNER JOIN SummonResource s
            ON st.skill_tree_id IN (s.skill_tree1_id, s.skill_tree2_id, s.skill_tree3_id, s.skill_tree4_id, s.skill_tree5_id)
        LEFT JOIN StringResource sr ON s.name_id = sr.code
        WHERE st.skill_id IN ({})
        ORDER BY 1, 3, 2",
        in_list(skill_ids)
    );
    Ok(client
        .query(&sql, &[])
        .await?
        .iter()
        .map(|row| {
            let mut c = Cells::new(row);
            SkillUsage { skill_id: c.i32(), skill_tree_id: c.i32(), summon_id: c.i32(), summon_name: c.string() }
        })
        .collect())
}

pub(super) async fn tree_usage(client: &Client, skill_tree_ids: &[i32]) -> PgResult<Vec<TreeUsage>> {
    if skill_tree_ids.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "
        SELECT s.id,
               COALESCE(sr.value, CONCAT('Summon ', s.id)),
               x.skill_tree_id,
               x.tree_slot
        FROM SummonResource s
        CROSS JOIN LATERAL (VALUES
            (s.skill_tree1_id, 1),
            (s.skill_tree2_id, 2),
            (s.skill_tree3_id, 3),
            (s.skill_tree4_id, 4),
            (s.skill_tree5_id, 5)
        ) x(skill_tree_id, tree_slot)
        LEFT JOIN StringResource sr ON s.name_id = sr.code
        WHERE x.skill_tree_id IN ({})
        ORDER BY x.skill_tree_id, s.id, x.tree_slot",
        in_list(skill_tree_ids)
    );
    Ok(client
        .query(&sql, &[])
        .await?
        .iter()
        .map(|row| {
            let mut c = Cells::new(row);
            let (summon_id, summon_name) = (c.i32(), c.string());
            TreeUsage { skill_tree_id: c.i32(), summon_id, summon_name, tree_slot: c.i32() }
        })
        .collect())
}

pub(super) async fn stat_usage(client: &Client, stat_ids: &[i32]) -> PgResult<Vec<StatUsage>> {
    if stat_ids.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "
        SELECT s.stat_id, s.id, COALESCE(sr.value, CONCAT('Summon ', s.id))
        FROM SummonResource s
        LEFT JOIN StringResource sr ON s.name_id = sr.code
        WHERE s.stat_id IN ({})
        ORDER BY s.stat_id, s.id",
        in_list(stat_ids)
    );
    Ok(client
        .query(&sql, &[])
        .await?
        .iter()
        .map(|row| {
            let mut c = Cells::new(row);
            StatUsage { stat_id: c.i32(), summon_id: c.i32(), summon_name: c.string() }
        })
        .collect())
}

pub(super) async fn level_bonus_usage(client: &Client, bonus_ids: &[i32]) -> PgResult<Vec<LevelBonusUsage>> {
    if bonus_ids.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "
        SELECT b.bonus_id, s.id, COALESCE(sr.value, CONCAT('Summon ', s.id)), b.stage_index
        FROM SummonResource s
        CROSS JOIN LATERAL (
            SELECT CASE WHEN s.form <= 1 THEN 1 WHEN s.form = 2 THEN 2 ELSE 3 END AS growth_depth
        ) depth_map
        CROSS JOIN LATERAL (VALUES
            (1, s.id - (depth_map.growth_depth - 1)),
            (2, s.id - (depth_map.growth_depth - 2)),
            (3, s.id - (depth_map.growth_depth - 3))
        ) b(stage_index, bonus_id)
        LEFT JOIN StringResource sr ON s.name_id = sr.code
        WHERE b.stage_index <= depth_map.growth_depth
          AND b.bonus_id IN ({})
        ORDER BY b.bonus_id, s.id, b.stage_index",
        in_list(bonus_ids)
    );
    Ok(client
        .query(&sql, &[])
        .await?
        .iter()
        .map(|row| {
            let mut c = Cells::new(row);
            LevelBonusUsage { bonus_id: c.i32(), summon_id: c.i32(), summon_name: c.string(), stage_index: c.i32() }
        })
        .collect())
}

pub(super) async fn random_group_usage(client: &Client, group_ids: &[i32]) -> PgResult<Vec<RandomGroupUsage>> {
    if group_ids.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "
        SELECT
            s.skill_group_id,
            s.skill_tree_id,
            sm.id,
            COALESCE(sr.value, CONCAT('Summon ', sm.id)),
            tree_slot.slot_index
        FROM SkillTreeResource s
        INNER JOIN SummonResource sm
            ON s.skill_tree_id IN (sm.skill_tree1_id, sm.skill_tree2_id, sm.skill_tree3_id, sm.skill_tree4_id, sm.skill_tree5_id)
        CROSS JOIN LATERAL (VALUES
            (1, sm.skill_tree1_id),
            (2, sm.skill_tree2_id),
            (3, sm.skill_tree3_id),
            (4, sm.skill_tree4_id),
            (5, sm.skill_tree5_id)
        ) tree_slot(slot_index, tree_id)
        LEFT JOIN StringResource sr ON sm.name_id = sr.code
        WHERE s.skill_group_id IN ({})
          AND tree_slot.tree_id = s.skill_tree_id
        ORDER BY 1, 3, 5, 2",
        in_list(group_ids)
    );
    Ok(client
        .query(&sql, &[])
        .await?
        .iter()
        .map(|row| {
            let mut c = Cells::new(row);
            RandomGroupUsage {
                group_id: c.i32(),
                skill_tree_id: c.i32(),
                summon_id: c.i32(),
                summon_name: c.string(),
                tree_slot: c.i32(),
            }
        })
        .collect())
}
