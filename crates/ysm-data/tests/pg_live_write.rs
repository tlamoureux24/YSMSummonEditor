//! Write test of the PostgreSQL repository against a live, disposable copy
//! of a game database. It CHANGES the data: never point it at a database
//! you want to keep.
//!
//! ```text
//! YSM_PG_WRITE_TEST_CONNECTION="Host=...;Database=<copy>;Username=..." \
//! YSM_PG_WRITE_TEST_SCRIPT=/tmp/changes.sql \
//!     cargo test -p ysm-data --test pg_live_write -- --ignored --nocapture
//! ```
//!
//! The password may come from the pgpass file. When `YSM_PG_WRITE_TEST_SCRIPT`
//! is set, the PostgreSQL `.sql` export of every successful save is appended
//! to that file: replayed with `psql` on a second copy of the same database,
//! it must leave the same rows as the direct saves.

use std::io::Write;

use rust_decimal::Decimal;
use ysm_core::commit::{CommitRequest, FieldChange, LevelBonusUpsert, SkillUpdate, StatCommit, TreeCommit};
use ysm_core::model::{SkillTreeEntry, StatBlock, SummonListFilter, SummonListItem};
use ysm_core::random_group::RandomSkillGroup;
use ysm_core::sql_script::{SqlDialect, render_script};
use ysm_core::value::{FieldKind, FieldValue};
use ysm_data::env_local::parse_connection_string_for;
use ysm_data::{DataError, DatabaseEngine, PgRepository, SummonRepository, loader};

fn save(repo: &mut PgRepository, request: &CommitRequest) {
    repo.commit(request).expect("save");
    if let Ok(path) = std::env::var("YSM_PG_WRITE_TEST_SCRIPT") {
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
        file.write_all(render_script(request, "pg_live_write", SqlDialect::PostgreSql).as_bytes()).unwrap();
    }
}

fn reload(repo: &mut PgRepository, summon_id: i32) -> SummonListItem {
    repo.summon_by_id(summon_id).unwrap().expect("summon still exists")
}

#[test]
#[ignore = "changes a live PostgreSQL database"]
fn every_kind_of_save_round_trips() {
    let connection = std::env::var("YSM_PG_WRITE_TEST_CONNECTION").expect("set YSM_PG_WRITE_TEST_CONNECTION");
    let settings = parse_connection_string_for(DatabaseEngine::PostgreSql, &connection).expect("connection string");
    let mut repo = PgRepository::new(settings).unwrap();
    println!("server: {}", repo.test_connection().unwrap());

    // A final summon with three stages, each with a first tree.
    let finals = repo.search_summons(&SummonListFilter { include_non_form1: true, ..Default::default() }).unwrap();
    let bundle = finals
        .iter()
        .filter(|s| s.form >= 3)
        .find_map(|s| {
            let bundle = loader::load_chain(&mut repo, s).ok()?;
            let complete = bundle.chain.stage1.is_some()
                && bundle.chain.stage2.is_some()
                && bundle.trees.iter().all(|stage| !stage[0].is_empty());
            complete.then_some(bundle)
        })
        .expect("a three-stage summon with trees");
    let [s1, s2, s3] = [1, 2, 3].map(|n| bundle.chain.stage(n).unwrap().clone());
    println!("chain: {} {} {}", s1.summon_id, s2.summon_id, s3.summon_id);

    // 1. Tree rewritten in place: one row changed, one added; jp_ratio kept.
    let tree_id = s1.skill_tree_ids[0];
    let mut rows = bundle.trees[0][0].clone();
    rows[0].job_lv += 1;
    let mut added = SkillTreeEntry::new_row(tree_id, rows.last().unwrap().lv + 1);
    added.skill_id = rows[0].skill_id;
    rows.push(added);
    let ratios_before: Vec<Decimal> = bundle.trees[0][0].iter().map(|e| e.jp_ratio).collect();
    let tree = |summon: &SummonListItem, entries: Vec<SkillTreeEntry>, clone| TreeCommit {
        summon_id: summon.summon_id,
        slot: 1,
        original_tree_id: summon.skill_tree_ids[0],
        entries,
        clone_if_shared: clone,
    };
    save(&mut repo, &CommitRequest { trees: vec![tree(&s1, rows.clone(), false)], ..Default::default() });
    let stored = repo.skill_tree_entries(tree_id).unwrap();
    assert_eq!(stored.len(), rows.len());
    assert!(stored.iter().any(|e| e.job_lv == rows[0].job_lv && e.skill_id == rows[0].skill_id));
    let mut ratios_after: Vec<Decimal> = stored.iter().map(|e| e.jp_ratio).collect();
    ratios_after.sort();
    let mut expected = ratios_before.clone();
    expected.push(Decimal::ONE);
    expected.sort();
    assert_eq!(ratios_after, expected, "jp_ratio kept, 1 for the new row");

    // 2. Tree cloned for stage 2 only: new ID past the highest, original untouched.
    let original_tree = s2.skill_tree_ids[0];
    let before_original = repo.skill_tree_entries(original_tree).unwrap();
    save(&mut repo, &CommitRequest { trees: vec![tree(&s2, bundle.trees[1][0].clone(), true)], ..Default::default() });
    let cloned_id = reload(&mut repo, s2.summon_id).skill_tree_ids[0];
    assert_ne!(cloned_id, original_tree);
    assert_eq!(repo.skill_tree_entries(original_tree).unwrap(), before_original);
    let clone_rows = repo.skill_tree_entries(cloned_id).unwrap();
    assert_eq!(clone_rows.len(), before_original.len());
    assert!(
        clone_rows
            .iter()
            .zip(&before_original)
            .all(|(c, o)| SkillTreeEntry { skill_tree_id: o.skill_tree_id, ..*c } == *o)
    );

    // 3. Stats: stage 1 updated in place, stage 3 cloned.
    let base1 = repo.stat_scaling(s1.summon_id).unwrap().base_stats;
    let raised = StatBlock { id: base1.id, values: base1.values.map(|v| v + 1) };
    let base3 = repo.stat_scaling(s3.summon_id).unwrap().base_stats;
    let cloned_stats = StatBlock { id: base3.id, values: base3.values.map(|v| v + 2) };
    save(
        &mut repo,
        &CommitRequest {
            stats: vec![
                StatCommit {
                    summon_id: s1.summon_id,
                    original_stat_id: base1.id,
                    stats: raised,
                    clone_if_shared: false,
                },
                StatCommit {
                    summon_id: s3.summon_id,
                    original_stat_id: base3.id,
                    stats: cloned_stats,
                    clone_if_shared: true,
                },
            ],
            ..Default::default()
        },
    );
    assert_eq!(repo.stat_scaling(s1.summon_id).unwrap().base_stats.values, raised.values);
    let after3 = repo.stat_scaling(s3.summon_id).unwrap().base_stats;
    assert_ne!(after3.id, base3.id);
    assert_eq!(after3.values, cloned_stats.values);

    // 4. Growth bonuses: one updated, one inserted for a summon that has none.
    let snapshot = repo.stat_scaling(s3.summon_id).unwrap();
    let existing = snapshot.stage_bonuses.iter().find(|b| b.exists_in_db).expect("a stored bonus");
    let half = Decimal::new(5, 1);
    let updated_values = existing.values.map(|v| v + half);
    let missing = finals
        .iter()
        .filter_map(|s| repo.stat_scaling(s.summon_id).ok())
        .flat_map(|snap| snap.stage_bonuses)
        .find(|b| !b.exists_in_db);
    let mut bonuses = vec![LevelBonusUpsert { bonus_id: existing.lookup_id, values: updated_values }];
    if let Some(missing) = &missing {
        bonuses.push(LevelBonusUpsert { bonus_id: missing.lookup_id, values: [Decimal::new(125, 2); 7] });
    }
    save(&mut repo, &CommitRequest { level_bonuses: bonuses, ..Default::default() });
    let stored = repo.stat_scaling(s3.summon_id).unwrap();
    let stored = stored.stage_bonuses.iter().find(|b| b.lookup_id == existing.lookup_id).unwrap();
    assert_eq!(stored.values, updated_values);
    match &missing {
        Some(missing) => println!("bonus inserted for {}", missing.lookup_id),
        None => println!("every summon already has its bonuses: insert not exercised"),
    }

    // 5. Random group created, then edited.
    let group_id = repo.next_random_group_id().unwrap();
    let skills: Vec<i32> = rows.iter().map(|e| e.skill_id).filter(|id| *id > 0).take(3).collect();
    save(
        &mut repo,
        &CommitRequest { random_groups: vec![RandomSkillGroup::new(group_id, [skills[0]])], ..Default::default() },
    );
    save(
        &mut repo,
        &CommitRequest { random_groups: vec![RandomSkillGroup::new(group_id, skills.clone())], ..Default::default() },
    );
    let group = repo.random_groups(&[group_id]).unwrap().remove(&group_id).expect("group stored");
    assert_eq!(group.use_code(), RandomSkillGroup::new(group_id, skills.clone()).use_code());
    assert_eq!(repo.next_random_group_id().unwrap(), group_id + 1);

    // 6. Skill fields of every editable type the table has.
    let skill_id = rows[0].skill_id;
    let record = repo.skill_record(skill_id).unwrap();
    let mut changes = Vec::new();
    let mut seen = Vec::new();
    for field in &record.fields {
        if field.name.eq_ignore_ascii_case("id") || seen.contains(&field.kind) {
            continue;
        }
        let new = match (&field.kind, &field.value) {
            (FieldKind::I16, FieldValue::I16(v)) => FieldValue::I16(v.wrapping_add(1)),
            (FieldKind::I32, FieldValue::I32(v)) => FieldValue::I32(v.wrapping_add(1)),
            (FieldKind::I64, FieldValue::I64(v)) => FieldValue::I64(v.wrapping_add(1)),
            (FieldKind::Decimal, FieldValue::Decimal(v)) => FieldValue::Decimal(*v + Decimal::ONE),
            (FieldKind::F32, FieldValue::F32(v)) => FieldValue::F32(v + 1.0),
            (FieldKind::F64, FieldValue::F64(v)) => FieldValue::F64(v + 1.0),
            (FieldKind::Bool, FieldValue::Bool(v)) => FieldValue::Bool(!v),
            (FieldKind::Text, FieldValue::Text(v)) if !v.trim().is_empty() => FieldValue::Text(v.clone()),
            _ => continue,
        };
        seen.push(field.kind);
        changes.push(FieldChange { name: field.name.clone(), kind: field.kind, old: field.value.clone(), new });
    }
    println!(
        "skill {skill_id}: {:?}",
        changes.iter().map(|c| format!("{} {}", c.name, c.kind.label())).collect::<Vec<_>>()
    );
    save(
        &mut repo,
        &CommitRequest { skills: vec![SkillUpdate { skill_id, changes: changes.clone() }], ..Default::default() },
    );
    let after = repo.skill_record(skill_id).unwrap();
    for change in &changes {
        assert_eq!(after.field(&change.name).unwrap().value, change.new, "{}", change.name);
    }

    // 7. A failing statement rolls the whole save back.
    let before = repo.skill_tree_entries(tree_id).unwrap();
    let mut broken_rows = before.clone();
    broken_rows[0].job_lv += 10;
    let broken = CommitRequest {
        trees: vec![tree(&s1, broken_rows, false)],
        skills: vec![SkillUpdate {
            skill_id,
            changes: vec![FieldChange {
                name: "no_such_column".into(),
                kind: FieldKind::I32,
                old: FieldValue::I32(0),
                new: FieldValue::I32(1),
            }],
        }],
        ..Default::default()
    };
    let error = repo.commit(&broken).unwrap_err();
    assert!(matches!(error, DataError::Query(_)), "{error}");
    assert_eq!(repo.skill_tree_entries(tree_id).unwrap(), before, "nothing of the failed save remains");
    println!("rollback: {error}");
}
