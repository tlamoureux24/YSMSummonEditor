//! Transactional write of a [`CommitRequest`].
//!
//! Statement order follows the original editor: skill trees, stat rows,
//! growth bonuses, random groups, then skill fields.

use std::borrow::Cow;

use rust_decimal::Decimal;
use tiberius::Query;
use ysm_core::commit::{CommitRequest, CommitResult, FieldChange, is_sql_identifier, tree_slot_column};
use ysm_core::model::{STAT_COUNT, SkillTreeEntry, StatBlock};
use ysm_core::value::{FieldKind, FieldValue};

use super::cells::Cells;
use super::{SqlClient, SqlResult};

pub(super) async fn commit(client: &mut SqlClient, request: &CommitRequest) -> SqlResult<CommitResult> {
    client.begin_transaction().await?;
    match apply(client, request).await {
        Ok(result) => {
            client.commit_transaction().await?;
            Ok(result)
        }
        Err(error) => {
            // The original failure is what the user needs to see; a rollback
            // failure on a broken connection adds nothing.
            let _ = client.rollback_transaction().await;
            Err(error)
        }
    }
}

async fn apply(client: &mut SqlClient, request: &CommitRequest) -> SqlResult<CommitResult> {
    let mut result = CommitResult::default();

    let mut last_cloned_tree_id = 0;
    for tree in &request.trees {
        let target_tree_id = if tree.clone_if_shared {
            let Some(column) = tree_slot_column(tree.slot) else {
                continue;
            };
            let next = scalar_i32(
                client,
                "SELECT ISNULL(MAX(skill_tree_id), 0) + 1 FROM SkillTreeResource WITH (UPDLOCK, HOLDLOCK);",
            )
            .await?;
            // A previous clone without rows leaves MAX unchanged; never hand out the same ID twice.
            let new_id = next.max(last_cloned_tree_id + 1);
            last_cloned_tree_id = new_id;

            let sql = format!("UPDATE SummonResource SET [{column}] = @P1 WHERE id = @P2;");
            client.execute(sql, &[&new_id, &tree.summon_id]).await?;
            new_id
        } else {
            client
                .execute("DELETE FROM SkillTreeResource WHERE skill_tree_id = @P1;", &[&tree.original_tree_id])
                .await?;
            tree.original_tree_id
        };

        for entry in &tree.entries {
            insert_tree_entry(client, target_tree_id, entry).await?;
        }
        result.updated_trees += 1;
    }

    for stat in request.stats.iter().filter(|s| s.original_stat_id > 0 && s.summon_id > 0) {
        if stat.clone_if_shared {
            let new_id =
                scalar_i32(client, "SELECT ISNULL(MAX([id]), 0) + 1 FROM [StatResource] WITH (UPDLOCK, HOLDLOCK);")
                    .await?;
            let mut query = Query::new(
                "INSERT INTO [StatResource]
                 ([id], [str], [vit], [dex], [agi], [int], [men], [luk])
                 VALUES
                 (@P1, @P2, @P3, @P4, @P5, @P6, @P7, @P8);",
            );
            query.bind(new_id);
            bind_stats(&mut query, &stat.stats);
            query.execute(client).await?;

            client
                .execute("UPDATE [SummonResource] SET [stat_id] = @P1 WHERE [id] = @P2;", &[&new_id, &stat.summon_id])
                .await?;
        } else {
            let mut query = Query::new(
                "UPDATE [StatResource]
                 SET [str] = @P2,
                     [vit] = @P3,
                     [dex] = @P4,
                     [agi] = @P5,
                     [int] = @P6,
                     [men] = @P7,
                     [luk] = @P8
                 WHERE [id] = @P1;",
            );
            query.bind(stat.original_stat_id);
            bind_stats(&mut query, &stat.stats);
            query.execute(client).await?;
        }
    }

    for bonus in request.level_bonuses.iter().filter(|b| b.bonus_id > 0) {
        let mut query = Query::new(
            "IF EXISTS (SELECT 1 FROM [CreatureLevelBonus] WHERE [id] = @P1)
             BEGIN
                 UPDATE [CreatureLevelBonus]
                 SET [str] = @P2,
                     [vit] = @P3,
                     [dex] = @P4,
                     [agi] = @P5,
                     [int] = @P6,
                     [men] = @P7,
                     [luk] = @P8
                 WHERE [id] = @P1;
             END
             ELSE
             BEGIN
                 INSERT INTO [CreatureLevelBonus]
                 ([id], [str], [vit], [dex], [agi], [int], [men], [luk])
                 VALUES
                 (@P1, @P2, @P3, @P4, @P5, @P6, @P7, @P8);
             END",
        );
        query.bind(bonus.bonus_id);
        bind_decimals(&mut query, &bonus.values);
        query.execute(client).await?;
    }

    for group in request.random_groups.iter().filter(|g| g.group_id > 0) {
        // @P1 group ID, @P2 use_code, @P3..@P17 skill_id_1..skill_id_15
        let slots = group.slots();
        let assignments: Vec<String> = (1..=slots.len()).map(|i| format!("[skill_id_{i}] = @P{}", i + 2)).collect();
        let columns: Vec<String> = (1..=slots.len()).map(|i| format!("[skill_id_{i}]")).collect();
        let placeholders: Vec<String> = (1..=slots.len()).map(|i| format!("@P{}", i + 2)).collect();
        let sql = format!(
            "IF EXISTS (SELECT 1 FROM [SummonRandomSkillResource] WHERE [sid] = @P1)
             BEGIN
                 UPDATE [SummonRandomSkillResource]
                 SET [use_code] = @P2, {}
                 WHERE [sid] = @P1;
             END
             ELSE
             BEGIN
                 INSERT INTO [SummonRandomSkillResource]
                 ([sid], [use_code], {})
                 VALUES
                 (@P1, @P2, {});
             END",
            assignments.join(", "),
            columns.join(", "),
            placeholders.join(", ")
        );
        let mut query = Query::new(sql);
        query.bind(group.group_id);
        query.bind(group.use_code());
        for slot in slots {
            query.bind(slot);
        }
        query.execute(client).await?;
        result.updated_random_groups += 1;
    }

    for skill in &request.skills {
        let updatable: Vec<&FieldChange> = skill.changes.iter().filter(|c| is_updatable_skill_column(c)).collect();
        if updatable.is_empty() {
            continue;
        }

        // Column names are spliced into the statement, hence the strict identifier check above.
        let assignments: Vec<String> =
            updatable.iter().enumerate().map(|(i, change)| format!("[{}] = @P{}", change.name, i + 1)).collect();
        let sql = format!("UPDATE SkillResource SET {} WHERE id = @P{};", assignments.join(", "), updatable.len() + 1);
        let mut query = Query::new(sql);
        for change in updatable {
            bind_field(&mut query, change);
        }
        query.bind(skill.skill_id);
        query.execute(client).await?;
        result.updated_skills += 1;
    }

    Ok(result)
}

pub(super) fn is_updatable_skill_column(change: &FieldChange) -> bool {
    is_sql_identifier(&change.name) && !change.name.eq_ignore_ascii_case("id") && change.kind.is_editable()
}

async fn scalar_i32(client: &mut SqlClient, sql: &'static str) -> SqlResult<i32> {
    let row = client.simple_query(sql).await?.into_row().await?;
    Ok(row.map(|row| Cells::new(row).i32()).unwrap_or(1))
}

async fn insert_tree_entry(client: &mut SqlClient, tree_id: i32, e: &SkillTreeEntry) -> SqlResult<()> {
    const SQL: &str = "
        INSERT INTO SkillTreeResource
        (skill_tree_id, skill_id, skill_group_id, min_skill_lv, max_skill_lv, lv, job_lv,
         need_skill_id_1, need_skill_lv_1, need_skill_id_2, need_skill_lv_2,
         need_skill_id_3, need_skill_lv_3, cenhance_min, cenhance_max, jp_ratio)
        VALUES
        (@P1, @P2, @P3, @P4, @P5, @P6, @P7,
         @P8, @P9, @P10, @P11,
         @P12, @P13, @P14, @P15, @P16);";

    let mut query = Query::new(SQL);
    for value in [
        tree_id,
        e.skill_id,
        e.skill_group_id,
        e.min_skill_lv,
        e.max_skill_lv,
        e.lv,
        e.job_lv,
        e.need_skill_id[0],
        e.need_skill_lv[0],
        e.need_skill_id[1],
        e.need_skill_lv[1],
        e.need_skill_id[2],
        e.need_skill_lv[2],
        e.cenhance_min,
        e.cenhance_max,
    ] {
        query.bind(value);
    }
    query.bind(e.jp_ratio);
    query.execute(client).await?;
    Ok(())
}

fn bind_stats(query: &mut Query<'_>, stats: &StatBlock) {
    for value in stats.values {
        query.bind(value);
    }
}

fn bind_decimals(query: &mut Query<'_>, values: &[Decimal; STAT_COUNT]) {
    for value in values {
        query.bind(*value);
    }
}

/// Binds the new value of a skill field with the SQL type of its column.
fn bind_field(query: &mut Query<'_>, change: &FieldChange) {
    match &change.new {
        FieldValue::Bool(v) => query.bind(*v),
        FieldValue::U8(v) => query.bind(*v),
        FieldValue::I16(v) => query.bind(*v),
        FieldValue::I32(v) => query.bind(*v),
        FieldValue::I64(v) => query.bind(*v),
        FieldValue::F32(v) => query.bind(*v),
        FieldValue::F64(v) => query.bind(*v),
        FieldValue::Decimal(v) => query.bind(*v),
        FieldValue::Text(v) | FieldValue::Unsupported(v) => query.bind(Cow::<str>::Owned(v.clone())),
        FieldValue::Null => match change.kind {
            FieldKind::Bool => query.bind(Option::<bool>::None),
            FieldKind::U8 => query.bind(Option::<u8>::None),
            FieldKind::I16 => query.bind(Option::<i16>::None),
            FieldKind::I32 => query.bind(Option::<i32>::None),
            FieldKind::I64 => query.bind(Option::<i64>::None),
            FieldKind::F32 => query.bind(Option::<f32>::None),
            FieldKind::F64 => query.bind(Option::<f64>::None),
            FieldKind::Decimal => query.bind(Option::<Decimal>::None),
            FieldKind::Text | FieldKind::Unsupported => query.bind(Option::<Cow<'_, str>>::None),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(name: &str, kind: FieldKind) -> FieldChange {
        FieldChange { name: name.into(), kind, old: FieldValue::Null, new: FieldValue::Null }
    }

    #[test]
    fn only_safe_editable_columns_are_written() {
        assert!(is_updatable_skill_column(&change("var1", FieldKind::Decimal)));
        assert!(is_updatable_skill_column(&change("icon_file_name", FieldKind::Text)));
        assert!(!is_updatable_skill_column(&change("ID", FieldKind::I32)));
        assert!(!is_updatable_skill_column(&change("blob", FieldKind::Unsupported)));
        assert!(!is_updatable_skill_column(&change("a]=1;--", FieldKind::I32)));
        assert!(!is_updatable_skill_column(&change("", FieldKind::I32)));
    }
}
