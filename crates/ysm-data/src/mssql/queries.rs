//! Read queries. The SQL is carried over verbatim from the original
//! `SqlServerSummonEditorRepository`; only ID lists are inlined as integer
//! literals instead of being expanded into parameters.

use std::collections::HashMap;

use rust_decimal::Decimal;
use tiberius::Row;
use ysm_core::evolution::growth_depth;
use ysm_core::model::{
    CreatureEnhance, LevelBonusStage, LevelBonusUsage, RandomGroupUsage, STAT_COUNT, SkillDisplay, SkillTreeEntry,
    SkillUsage, StatBlock, StatScalingSnapshot, StatUsage, SummonListFilter, SummonListItem, TreeUsage,
};
use ysm_core::random_group::RandomSkillGroup;
use ysm_core::value::{SkillField, SkillRecord};

use super::cells::{Cells, cell_field};
use super::{SqlClient, SqlResult};

const SUMMON_PROJECTION: &str = "
    SELECT
        s.[id] AS SummonId,
        ISNULL(sn.[value], CONCAT('Summon ', s.[id])) AS SummonName,
        cn.[value] AS CardName,
        s.[card_id] AS CardId,
        s.[evolve_target] AS EvolveTarget,
        s.[form] AS Form,
        CAST(s.[is_riding_only] AS bit) AS IsRidingOnly,
        s.[face_file_name] AS FaceFileName,
        s.[illust_file_name] AS IllustFileName,
        sf.[value] AS FeatureText,
        s.[stat_id] AS StatId,
        s.[skill_tree1_id] AS SkillTree1Id,
        s.[skill_tree2_id] AS SkillTree2Id,
        s.[skill_tree3_id] AS SkillTree3Id,
        s.[skill_tree4_id] AS SkillTree4Id,
        s.[skill_tree5_id] AS SkillTree5Id
    FROM [SummonResource] s
    LEFT JOIN [StringResource] sn ON s.[name_id] = sn.[code]
    LEFT JOIN [StringResource] sf ON s.[text_feature_id] = sf.[code]
    LEFT JOIN [ItemResource] ir ON s.[card_id] = ir.[id]
    LEFT JOIN [StringResource] cn ON ir.[name_id] = cn.[code]
";

/// Renders IDs as a literal list for `IN (...)`. Safe: the values are integers.
pub(super) fn in_list(ids: &[i32]) -> String {
    ids.iter().map(i32::to_string).collect::<Vec<_>>().join(", ")
}

fn summon(row: Row) -> SummonListItem {
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

pub(super) async fn server_version(client: &mut SqlClient) -> SqlResult<String> {
    let row = client.simple_query("SELECT @@VERSION;").await?.into_row().await?;
    let version = row.map(|row| Cells::new(row).string()).unwrap_or_default();
    Ok(version.lines().next().unwrap_or("SQL Server").trim().to_owned())
}

pub(super) async fn summon_by_id(client: &mut SqlClient, summon_id: i32) -> SqlResult<Option<SummonListItem>> {
    let sql = format!("{SUMMON_PROJECTION} WHERE s.[id] = @P1;");
    Ok(client.query(sql, &[&summon_id]).await?.into_row().await?.map(summon))
}

pub(super) async fn summons_by_card_id(client: &mut SqlClient, card_id: i32) -> SqlResult<Vec<SummonListItem>> {
    let sql = format!("{SUMMON_PROJECTION} WHERE s.[card_id] = @P1 ORDER BY s.[id] ASC;");
    let rows = client.query(sql, &[&card_id]).await?.into_first_result().await?;
    Ok(rows.into_iter().map(summon).collect())
}

pub(super) async fn search_summons(
    client: &mut SqlClient,
    filter: &SummonListFilter,
) -> SqlResult<Vec<SummonListItem>> {
    // @P1 IncludeNonForm1, @P2 IncludeRidingOnly, @P3 ShowAllEvolutions,
    // @P4 SearchText, @P5 ExactSummonId, @P6 SearchLike
    let sql = format!(
        "{SUMMON_PROJECTION}
        WHERE (@P1 = 1 OR s.[form] = 1)
          AND (@P2 = 1 OR s.[is_riding_only] = 0)
          AND (@P3 = 1 OR s.[evolve_target] = 0)
          AND (
            @P4 = ''
            OR (@P5 IS NOT NULL AND s.[id] = @P5)
            OR CAST(s.[id] AS nvarchar(32)) LIKE @P6
            OR ISNULL(sn.[value], '') LIKE @P6
            OR ISNULL(cn.[value], '') LIKE @P6
          )
        ORDER BY
            CASE WHEN @P5 IS NOT NULL AND s.[id] = @P5 THEN 0 ELSE 1 END,
            s.[id] ASC;"
    );

    let search = filter.search_text.trim();
    let exact_id: Option<i32> = search.parse().ok();
    let like = format!("%{search}%");
    let rows = client
        .query(
            sql,
            &[
                &filter.include_non_form1,
                &filter.include_riding_only,
                &filter.show_all_evolutions,
                &search,
                &exact_id,
                &like.as_str(),
            ],
        )
        .await?
        .into_first_result()
        .await?;
    Ok(rows.into_iter().map(summon).collect())
}

pub(super) async fn skill_tree_entries(client: &mut SqlClient, skill_tree_id: i32) -> SqlResult<Vec<SkillTreeEntry>> {
    const SQL: &str = "
        SELECT
            skill_tree_id AS SkillTreeId,
            skill_id AS SkillId,
            skill_group_id AS SkillGroupId,
            min_skill_lv AS MinSkillLv,
            max_skill_lv AS MaxSkillLv,
            lv AS Lv,
            job_lv AS JobLv,
            need_skill_id_1 AS NeedSkillId1,
            need_skill_lv_1 AS NeedSkillLv1,
            need_skill_id_2 AS NeedSkillId2,
            need_skill_lv_2 AS NeedSkillLv2,
            need_skill_id_3 AS NeedSkillId3,
            need_skill_lv_3 AS NeedSkillLv3,
            cenhance_min AS CEnhanceMin,
            cenhance_max AS CEnhanceMax,
            CAST(jp_ratio AS decimal(18,6)) AS JpRatio
        FROM SkillTreeResource
        WHERE skill_tree_id = @P1
        ORDER BY lv ASC, skill_id ASC;";

    let rows = client.query(SQL, &[&skill_tree_id]).await?.into_first_result().await?;
    Ok(rows
        .into_iter()
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
pub(super) async fn stat_scaling(client: &mut SqlClient, summon_id: i32) -> SqlResult<Option<StatScalingSnapshot>> {
    const SUMMON_SQL: &str = "
        SELECT TOP 1
            s.[id] AS SummonId,
            s.[stat_id] AS StatId,
            s.[form] AS EvolveType
        FROM [SummonResource] s
        WHERE s.[id] = @P1;";

    const STAT_SQL: &str = "
        SELECT TOP 1
            sr.[id] AS Id,
            sr.[str] AS Str,
            sr.[vit] AS Vit,
            sr.[dex] AS Dex,
            sr.[agi] AS Agi,
            sr.[int] AS [Int],
            sr.[men] AS Men,
            sr.[luk] AS Luk
        FROM [StatResource] sr
        WHERE sr.[id] = @P1;";

    let Some(row) = client.query(SUMMON_SQL, &[&summon_id]).await?.into_row().await? else {
        return Ok(None);
    };
    let mut c = Cells::new(row);
    let (summon_id, stat_id, evolve_type) = (c.i32(), c.i32(), c.i32());
    let depth = growth_depth(evolve_type);
    let lookup_ids: Vec<i32> = (1..=depth).map(|stage_index| summon_id - (depth - stage_index)).collect();

    let base_stats = match client.query(STAT_SQL, &[&stat_id]).await?.into_row().await? {
        Some(row) => {
            let mut c = Cells::new(row);
            StatBlock { id: c.i32(), values: std::array::from_fn(|_| c.i32()) }
        }
        None => StatBlock::zero(stat_id),
    };

    let bonus_sql = format!(
        "
        SELECT
            b.[id] AS LookupId,
            CAST(b.[str] AS decimal(18,6)) AS Str,
            CAST(b.[vit] AS decimal(18,6)) AS Vit,
            CAST(b.[dex] AS decimal(18,6)) AS Dex,
            CAST(b.[agi] AS decimal(18,6)) AS Agi,
            CAST(b.[int] AS decimal(18,6)) AS [Int],
            CAST(b.[men] AS decimal(18,6)) AS Men,
            CAST(b.[luk] AS decimal(18,6)) AS Luk
        FROM [CreatureLevelBonus] b
        WHERE b.[id] IN ({});",
        in_list(&lookup_ids)
    );
    let mut bonuses: HashMap<i32, [Decimal; STAT_COUNT]> = HashMap::new();
    for row in client.simple_query(bonus_sql).await?.into_first_result().await? {
        let mut c = Cells::new(row);
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

pub(super) async fn creature_enhance(client: &mut SqlClient) -> SqlResult<Vec<CreatureEnhance>> {
    const SQL: &str = "
        SELECT
            ce.[enhance_level] AS EnhanceLevel,
            CAST(ce.[stat_amplify] AS decimal(18,6)) AS StatAmplify,
            ce.[card_durability] AS CardDurability,
            ce.[slot_amount] AS SlotAmount,
            ce.[jp_addition] AS JpAddition
        FROM [CreatureEnhance] ce
        ORDER BY ce.[enhance_level] ASC;";

    let rows = client.simple_query(SQL).await?.into_first_result().await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let mut c = Cells::new(row);
            CreatureEnhance { enhance_level: c.i32(), stat_amplify: c.decimal() }
        })
        .collect())
}

/// Returns `None` when the skill does not exist.
pub(super) async fn skill_record(client: &mut SqlClient, skill_id: i32) -> SqlResult<Option<SkillRecord>> {
    const SQL: &str = "SELECT TOP 1 * FROM SkillResource WHERE id = @P1;";

    let Some(row) = client.query(SQL, &[&skill_id]).await?.into_row().await? else {
        return Ok(None);
    };
    let names: Vec<String> = row.columns().iter().map(|column| column.name().to_owned()).collect();
    let fields = names
        .into_iter()
        .zip(row)
        .map(|(name, cell)| {
            let (kind, value) = cell_field(&cell);
            SkillField { name, kind, value }
        })
        .collect();
    Ok(Some(SkillRecord { skill_id, fields }))
}

fn skill_display_row(row: Row) -> SkillDisplay {
    let mut c = Cells::new(row);
    SkillDisplay {
        skill_id: c.i32(),
        skill_name: c.string(),
        icon_file_name: c.opt_string(),
        tooltip_markup: c.opt_string(),
    }
}

pub(super) async fn skill_display(client: &mut SqlClient, skill_ids: &[i32]) -> SqlResult<HashMap<i32, SkillDisplay>> {
    if skill_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let sql = format!(
        "
        SELECT
            s.id AS SkillId,
            ISNULL(name_sr.value, CONCAT('Skill ', s.id)) AS SkillName,
            NULLIF(LTRIM(RTRIM(s.icon_file_name)), '') AS IconFileName,
            tip_sr.value AS TooltipMarkup
        FROM SkillResource s
        LEFT JOIN StringResource name_sr ON s.text_id = name_sr.code
        LEFT JOIN StringResource tip_sr ON s.tooltip_id = tip_sr.code
        WHERE s.id IN ({});",
        in_list(skill_ids)
    );
    let rows = client.simple_query(sql).await?.into_first_result().await?;
    Ok(rows.into_iter().map(skill_display_row).map(|d| (d.skill_id, d)).collect())
}

pub(super) async fn search_skills(client: &mut SqlClient, text: &str, limit: usize) -> SqlResult<Vec<SkillDisplay>> {
    // @P1 Limit, @P2 SearchText, @P3 ExactSkillId, @P4 SearchLike
    const SQL: &str = "
        SELECT TOP (@P1)
            s.[id] AS SkillId,
            ISNULL(name_sr.[value], CONCAT('Skill ', s.[id])) AS SkillName,
            NULLIF(LTRIM(RTRIM(s.[icon_file_name])), '') AS IconFileName,
            tip_sr.[value] AS TooltipMarkup
        FROM [SkillResource] s
        LEFT JOIN [StringResource] name_sr ON s.[text_id] = name_sr.[code]
        LEFT JOIN [StringResource] tip_sr ON s.[tooltip_id] = tip_sr.[code]
        WHERE
            @P2 = ''
            OR (@P3 IS NOT NULL AND s.[id] = @P3)
            OR CAST(s.[id] AS nvarchar(32)) LIKE @P4
            OR ISNULL(name_sr.[value], '') LIKE @P4
        ORDER BY
            CASE WHEN @P3 IS NOT NULL AND s.[id] = @P3 THEN 0 ELSE 1 END,
            s.[id] ASC;";

    let search = text.trim();
    let exact_id: Option<i32> = search.parse().ok();
    let like = format!("%{search}%");
    let limit = limit.clamp(1, crate::repository::SKILL_SEARCH_LIMIT) as i32;
    let rows = client.query(SQL, &[&limit, &search, &exact_id, &like.as_str()]).await?.into_first_result().await?;
    Ok(rows.into_iter().map(skill_display_row).collect())
}

pub(super) async fn random_groups(
    client: &mut SqlClient,
    group_ids: &[i32],
) -> SqlResult<HashMap<i32, RandomSkillGroup>> {
    if group_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let sql = format!(
        "
        SELECT
            r.[sid] AS GroupId,
            r.[use_code] AS UseCode,
            r.[skill_id_1] AS SkillId1,
            r.[skill_id_2] AS SkillId2,
            r.[skill_id_3] AS SkillId3,
            r.[skill_id_4] AS SkillId4,
            r.[skill_id_5] AS SkillId5,
            r.[skill_id_6] AS SkillId6,
            r.[skill_id_7] AS SkillId7,
            r.[skill_id_8] AS SkillId8,
            r.[skill_id_9] AS SkillId9,
            r.[skill_id_10] AS SkillId10,
            r.[skill_id_11] AS SkillId11,
            r.[skill_id_12] AS SkillId12,
            r.[skill_id_13] AS SkillId13,
            r.[skill_id_14] AS SkillId14,
            r.[skill_id_15] AS SkillId15
        FROM [SummonRandomSkillResource] r
        WHERE r.[sid] IN ({});",
        in_list(group_ids)
    );
    let rows = client.simple_query(sql).await?.into_first_result().await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let mut c = Cells::new(row);
            let (group_id, use_code) = (c.i32(), c.i32());
            let group = RandomSkillGroup::from_db_row(group_id, use_code, std::array::from_fn(|_| c.i32()));
            (group_id, group)
        })
        .collect())
}

pub(super) async fn next_random_group_id(client: &mut SqlClient) -> SqlResult<i32> {
    const SQL: &str = "
        SELECT ISNULL(MAX(v.[id]), 0) + 1
        FROM (
            SELECT r.[sid] AS [id] FROM [SummonRandomSkillResource] r
            UNION ALL
            SELECT s.[skill_group_id] AS [id]
            FROM [SkillTreeResource] s
            WHERE s.[skill_group_id] > 0
        ) v;";

    let row = client.simple_query(SQL).await?.into_row().await?;
    Ok(row.map(|row| Cells::new(row).i32()).unwrap_or(1).max(1))
}

pub(super) async fn skill_usage(client: &mut SqlClient, skill_ids: &[i32]) -> SqlResult<Vec<SkillUsage>> {
    if skill_ids.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "
        SELECT
            st.skill_id AS SkillId,
            st.skill_tree_id AS SkillTreeId,
            s.id AS SummonId,
            ISNULL(sr.value, CONCAT('Summon ', s.id)) AS SummonName
        FROM SkillTreeResource st
        INNER JOIN SummonResource s
            ON st.skill_tree_id IN (s.skill_tree1_id, s.skill_tree2_id, s.skill_tree3_id, s.skill_tree4_id, s.skill_tree5_id)
        LEFT JOIN StringResource sr ON s.name_id = sr.code
        WHERE st.skill_id IN ({})
        ORDER BY st.skill_id, s.id;",
        in_list(skill_ids)
    );
    let rows = client.simple_query(sql).await?.into_first_result().await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let mut c = Cells::new(row);
            SkillUsage { skill_id: c.i32(), skill_tree_id: c.i32(), summon_id: c.i32(), summon_name: c.string() }
        })
        .collect())
}

pub(super) async fn tree_usage(client: &mut SqlClient, skill_tree_ids: &[i32]) -> SqlResult<Vec<TreeUsage>> {
    if skill_tree_ids.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "
        SELECT s.id AS SummonId,
               ISNULL(sr.value, CONCAT('Summon ', s.id)) AS SummonName,
               x.skill_tree_id AS SkillTreeId,
               x.tree_slot AS TreeSlotIndex
        FROM SummonResource s
        CROSS APPLY (VALUES
            (s.skill_tree1_id, 1),
            (s.skill_tree2_id, 2),
            (s.skill_tree3_id, 3),
            (s.skill_tree4_id, 4),
            (s.skill_tree5_id, 5)
        ) x(skill_tree_id, tree_slot)
        LEFT JOIN StringResource sr ON s.name_id = sr.code
        WHERE x.skill_tree_id IN ({})
        ORDER BY x.skill_tree_id, s.id;",
        in_list(skill_tree_ids)
    );
    let rows = client.simple_query(sql).await?.into_first_result().await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let mut c = Cells::new(row);
            let (summon_id, summon_name) = (c.i32(), c.string());
            TreeUsage { skill_tree_id: c.i32(), summon_id, summon_name, tree_slot: c.i32() }
        })
        .collect())
}

pub(super) async fn stat_usage(client: &mut SqlClient, stat_ids: &[i32]) -> SqlResult<Vec<StatUsage>> {
    if stat_ids.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "
        SELECT
            s.[stat_id] AS StatId,
            s.[id] AS SummonId,
            ISNULL(sr.[value], CONCAT('Summon ', s.[id])) AS SummonName
        FROM [SummonResource] s
        LEFT JOIN [StringResource] sr ON s.[name_id] = sr.[code]
        WHERE s.[stat_id] IN ({})
        ORDER BY s.[stat_id], s.[id];",
        in_list(stat_ids)
    );
    let rows = client.simple_query(sql).await?.into_first_result().await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let mut c = Cells::new(row);
            StatUsage { stat_id: c.i32(), summon_id: c.i32(), summon_name: c.string() }
        })
        .collect())
}

pub(super) async fn level_bonus_usage(client: &mut SqlClient, bonus_ids: &[i32]) -> SqlResult<Vec<LevelBonusUsage>> {
    if bonus_ids.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "
        SELECT
            b.[bonus_id] AS BonusId,
            s.[id] AS SummonId,
            ISNULL(sr.[value], CONCAT('Summon ', s.[id])) AS SummonName,
            b.[stage_index] AS StageIndex
        FROM [SummonResource] s
        CROSS APPLY (
            SELECT CASE
                WHEN s.[form] <= 1 THEN 1
                WHEN s.[form] = 2 THEN 2
                ELSE 3
            END AS growth_depth
        ) depth_map
        CROSS APPLY (VALUES
            (1, s.[id] - (depth_map.[growth_depth] - 1)),
            (2, s.[id] - (depth_map.[growth_depth] - 2)),
            (3, s.[id] - (depth_map.[growth_depth] - 3))
        ) b(stage_index, bonus_id)
        LEFT JOIN [StringResource] sr ON s.[name_id] = sr.[code]
        WHERE b.[stage_index] <= depth_map.[growth_depth]
          AND b.[bonus_id] IN ({})
        ORDER BY b.[bonus_id], s.[id], b.[stage_index];",
        in_list(bonus_ids)
    );
    let rows = client.simple_query(sql).await?.into_first_result().await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let mut c = Cells::new(row);
            LevelBonusUsage { bonus_id: c.i32(), summon_id: c.i32(), summon_name: c.string(), stage_index: c.i32() }
        })
        .collect())
}

pub(super) async fn random_group_usage(client: &mut SqlClient, group_ids: &[i32]) -> SqlResult<Vec<RandomGroupUsage>> {
    if group_ids.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "
        SELECT
            s.[skill_group_id] AS GroupId,
            s.[skill_tree_id] AS SkillTreeId,
            sm.[id] AS SummonId,
            ISNULL(sr.[value], CONCAT('Summon ', sm.[id])) AS SummonName,
            tree_slot.[slot_index] AS TreeSlotIndex
        FROM [SkillTreeResource] s
        INNER JOIN [SummonResource] sm
            ON s.[skill_tree_id] IN (sm.[skill_tree1_id], sm.[skill_tree2_id], sm.[skill_tree3_id], sm.[skill_tree4_id], sm.[skill_tree5_id])
        CROSS APPLY (VALUES
            (1, sm.[skill_tree1_id]),
            (2, sm.[skill_tree2_id]),
            (3, sm.[skill_tree3_id]),
            (4, sm.[skill_tree4_id]),
            (5, sm.[skill_tree5_id])
        ) tree_slot(slot_index, tree_id)
        LEFT JOIN [StringResource] sr ON sm.[name_id] = sr.[code]
        WHERE s.[skill_group_id] IN ({})
          AND tree_slot.[tree_id] = s.[skill_tree_id]
        ORDER BY s.[skill_group_id], sm.[id], tree_slot.[slot_index];",
        in_list(group_ids)
    );
    let rows = client.simple_query(sql).await?.into_first_result().await?;
    Ok(rows
        .into_iter()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_lists_are_rendered_as_plain_integers() {
        assert_eq!(in_list(&[3, -1, 40062]), "3, -1, 40062");
        assert_eq!(in_list(&[]), "");
    }
}
