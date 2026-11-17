//! Transactional write of a [`CommitRequest`], statement for statement like
//! `mssql/commit.rs`: skill trees, stat rows, growth bonuses, random groups,
//! then skill fields.

use rust_decimal::Decimal;
use tokio_postgres::types::ToSql;
use tokio_postgres::{Client, Transaction};
use ysm_core::commit::{CommitRequest, CommitResult, FieldChange, is_sql_identifier, tree_slot_column};
use ysm_core::model::{STAT_COUNT, SkillTreeEntry, StatBlock};
use ysm_core::value::{FieldKind, FieldValue};

use super::PgResult;

type Param = Box<dyn ToSql + Sync>;

pub(super) async fn commit(client: &mut Client, request: &CommitRequest) -> PgResult<CommitResult> {
    // Dropping the transaction without `commit` rolls it back.
    let tx = client.transaction().await?;
    let result = apply(&tx, request).await?;
    tx.commit().await?;
    Ok(result)
}

async fn apply(tx: &Transaction<'_>, request: &CommitRequest) -> PgResult<CommitResult> {
    let mut result = CommitResult::default();

    let mut last_cloned_tree_id = 0;
    for tree in &request.trees {
        let target_tree_id = if tree.clone_if_shared {
            let Some(column) = tree_slot_column(tree.slot) else {
                continue;
            };
            // Held until the end of the transaction: no other writer can take the same ID.
            tx.batch_execute("LOCK TABLE SkillTreeResource IN SHARE ROW EXCLUSIVE MODE").await?;
            let next = scalar_i32(tx, "SELECT COALESCE(MAX(skill_tree_id), 0) + 1 FROM SkillTreeResource").await?;
            // A previous clone without rows leaves MAX unchanged; never hand out the same ID twice.
            let new_id = next.max(last_cloned_tree_id + 1);
            last_cloned_tree_id = new_id;

            let sql = format!("UPDATE SummonResource SET {column} = $1 WHERE id = $2");
            tx.execute(&sql, &[&new_id, &tree.summon_id]).await?;
            new_id
        } else {
            tx.execute("DELETE FROM SkillTreeResource WHERE skill_tree_id = $1", &[&tree.original_tree_id]).await?;
            tree.original_tree_id
        };

        for entry in &tree.entries {
            insert_tree_entry(tx, target_tree_id, entry).await?;
        }
        result.updated_trees += 1;
    }

    for stat in request.stats.iter().filter(|s| s.original_stat_id > 0 && s.summon_id > 0) {
        if stat.clone_if_shared {
            tx.batch_execute("LOCK TABLE StatResource IN SHARE ROW EXCLUSIVE MODE").await?;
            let new_id = scalar_i32(tx, "SELECT COALESCE(MAX(id), 0) + 1 FROM StatResource").await?;
            tx.execute(
                "INSERT INTO StatResource (id, str, vit, dex, agi, \"int\", men, luk)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                &with_stats(&new_id, &stat.stats),
            )
            .await?;
            tx.execute("UPDATE SummonResource SET stat_id = $1 WHERE id = $2", &[&new_id, &stat.summon_id]).await?;
        } else {
            tx.execute(
                "UPDATE StatResource
                 SET str = $2, vit = $3, dex = $4, agi = $5, \"int\" = $6, men = $7, luk = $8
                 WHERE id = $1",
                &with_stats(&stat.original_stat_id, &stat.stats),
            )
            .await?;
        }
    }

    for bonus in request.level_bonuses.iter().filter(|b| b.bonus_id > 0) {
        let params = with_decimals(&bonus.bonus_id, &bonus.values);
        let updated = tx
            .execute(
                "UPDATE CreatureLevelBonus
             SET str = $2, vit = $3, dex = $4, agi = $5, \"int\" = $6, men = $7, luk = $8
             WHERE id = $1",
                &params,
            )
            .await?;
        if updated == 0 {
            tx.execute(
                "INSERT INTO CreatureLevelBonus (id, str, vit, dex, agi, \"int\", men, luk)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                &params,
            )
            .await?;
        }
    }

    for group in request.random_groups.iter().filter(|g| g.group_id > 0) {
        // $1 group ID, $2 use_code, $3..$17 skill_id_1..skill_id_15
        let slots = group.slots();
        let assignments: Vec<String> = (1..=slots.len()).map(|i| format!("skill_id_{i} = ${}", i + 2)).collect();
        let columns: Vec<String> = (1..=slots.len()).map(|i| format!("skill_id_{i}")).collect();
        let placeholders: Vec<String> = (1..=slots.len()).map(|i| format!("${}", i + 2)).collect();

        let mut values: Vec<i32> = vec![group.group_id, group.use_code()];
        values.extend(slots.iter().copied());
        let params: Vec<&(dyn ToSql + Sync)> = values.iter().map(|v| v as &(dyn ToSql + Sync)).collect();

        let update =
            format!("UPDATE SummonRandomSkillResource SET use_code = $2, {} WHERE sid = $1", assignments.join(", "));
        if tx.execute(&update, &params).await? == 0 {
            let insert = format!(
                "INSERT INTO SummonRandomSkillResource (sid, use_code, {}) VALUES ($1, $2, {})",
                columns.join(", "),
                placeholders.join(", ")
            );
            tx.execute(&insert, &params).await?;
        }
        result.updated_random_groups += 1;
    }

    for skill in &request.skills {
        let updatable: Vec<&FieldChange> = skill.changes.iter().filter(|c| is_updatable_skill_column(c)).collect();
        if updatable.is_empty() {
            continue;
        }

        // Column names are spliced into the statement, hence the strict identifier check above.
        let assignments: Vec<String> =
            updatable.iter().enumerate().map(|(i, change)| format!("\"{}\" = ${}", change.name, i + 1)).collect();
        let sql = format!("UPDATE SkillResource SET {} WHERE id = ${}", assignments.join(", "), updatable.len() + 1);
        let mut owned: Vec<Param> = updatable.iter().map(|change| field_param(change)).collect();
        owned.push(Box::new(skill.skill_id));
        tx.execute(&sql, &borrow(&owned)).await?;
        result.updated_skills += 1;
    }

    Ok(result)
}

async fn scalar_i32(tx: &Transaction<'_>, sql: &str) -> PgResult<i32> {
    let row = tx.query_one(sql, &[]).await?;
    Ok(super::cells::Cells::new(&row).i32().max(1))
}

fn borrow(owned: &[Param]) -> Vec<&(dyn ToSql + Sync)> {
    owned.iter().map(|p| p.as_ref() as &(dyn ToSql + Sync)).collect()
}

async fn insert_tree_entry(tx: &Transaction<'_>, tree_id: i32, e: &SkillTreeEntry) -> PgResult<()> {
    const SQL: &str = "
        INSERT INTO SkillTreeResource
        (skill_tree_id, skill_id, skill_group_id, min_skill_lv, max_skill_lv, lv, job_lv,
         need_skill_id_1, need_skill_lv_1, need_skill_id_2, need_skill_lv_2,
         need_skill_id_3, need_skill_lv_3, cenhance_min, cenhance_max, jp_ratio)
        VALUES
        ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)";

    let ints = [
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
    ];
    let mut params: Vec<&(dyn ToSql + Sync)> = ints.iter().map(|v| v as &(dyn ToSql + Sync)).collect();
    params.push(&e.jp_ratio);
    tx.execute(SQL, &params).await?;
    Ok(())
}

/// `$1` the row ID, `$2..$8` the seven stats.
fn with_stats<'a>(id: &'a i32, stats: &'a StatBlock) -> Vec<&'a (dyn ToSql + Sync)> {
    let mut params: Vec<&(dyn ToSql + Sync)> = vec![id];
    params.extend(stats.values.iter().map(|v| v as &(dyn ToSql + Sync)));
    params
}

/// `$1` the row ID, `$2..$8` the seven growth values.
fn with_decimals<'a>(id: &'a i32, values: &'a [Decimal; STAT_COUNT]) -> Vec<&'a (dyn ToSql + Sync)> {
    let mut params: Vec<&(dyn ToSql + Sync)> = vec![id];
    params.extend(values.iter().map(|v| v as &(dyn ToSql + Sync)));
    params
}

/// Same rule as the MSSQL writer: a safe identifier, never `id`, an editable type.
fn is_updatable_skill_column(change: &FieldChange) -> bool {
    is_sql_identifier(&change.name) && !change.name.eq_ignore_ascii_case("id") && change.kind.is_editable()
}

/// The new value of a skill field, typed like its column.
fn field_param(change: &FieldChange) -> Param {
    match &change.new {
        FieldValue::Bool(v) => Box::new(*v),
        FieldValue::U8(v) => Box::new(i16::from(*v)),
        FieldValue::I16(v) => Box::new(*v),
        FieldValue::I32(v) => Box::new(*v),
        FieldValue::I64(v) => Box::new(*v),
        FieldValue::F32(v) => Box::new(*v),
        FieldValue::F64(v) => Box::new(*v),
        FieldValue::Decimal(v) => Box::new(*v),
        FieldValue::Text(v) | FieldValue::Unsupported(v) => Box::new(v.clone()),
        FieldValue::Null => match change.kind {
            FieldKind::Bool => Box::new(Option::<bool>::None),
            FieldKind::U8 | FieldKind::I16 => Box::new(Option::<i16>::None),
            FieldKind::I32 => Box::new(Option::<i32>::None),
            FieldKind::I64 => Box::new(Option::<i64>::None),
            FieldKind::F32 => Box::new(Option::<f32>::None),
            FieldKind::F64 => Box::new(Option::<f64>::None),
            FieldKind::Decimal => Box::new(Option::<Decimal>::None),
            FieldKind::Text | FieldKind::Unsupported => Box::new(Option::<String>::None),
        },
    }
}
