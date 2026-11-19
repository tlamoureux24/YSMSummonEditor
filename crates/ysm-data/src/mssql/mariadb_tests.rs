use mysql::prelude::Queryable;
use rust_decimal::Decimal;
use ysm_core::commit::{FieldChange, LevelBonusUpsert, SkillUpdate, StatCommit, TreeCommit};
use ysm_core::model::{SkillTreeEntry, StatBlock};
use ysm_core::random_group::RandomSkillGroup;
use ysm_core::value::{FieldKind, FieldValue};

use super::*;
use crate::SettingsStore;

#[test]
#[ignore = "needs a live MariaDB database; writes are limited to connection-local temporary tables"]
fn mariadb_commit_preserves_tree_values_and_rolls_back_on_error() {
    let settings = SettingsStore::at_default_location().load().unwrap().database;
    assert_eq!(settings.engine, DatabaseEngine::MariaDb);
    let mut repo = SqlRepository::new(settings).unwrap();
    let ReadClient::MariaDb(client) =
        repo.client.insert(ReadClient::MariaDb(Box::new(connect_mariadb(&repo.settings, repo.timeouts).unwrap())))
    else {
        unreachable!();
    };

    // Every table touched by the batch is shadowed in this connection.
    for table in ["SkillTreeResource", "StatResource", "CreatureLevelBonus", "SummonRandomSkillResource"] {
        let (_, ddl): (String, String) = client.query_first(format!("SHOW CREATE TABLE {table}")).unwrap().unwrap();
        client.query_drop(ddl.replacen("CREATE TABLE ", "CREATE TEMPORARY TABLE ", 1)).unwrap();
    }
    client
        .query_drop(
            "CREATE TEMPORARY TABLE SummonResource (
            id INT, stat_id INT, skill_tree1_id INT, skill_tree2_id INT, skill_tree3_id INT,
            skill_tree4_id INT, skill_tree5_id INT, form INT
        ) ENGINE=InnoDB",
        )
        .unwrap();
    client.query_drop(
        "CREATE TEMPORARY TABLE SkillResource (id INT, icon_file_name VARCHAR(256)) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4",
    ).unwrap();
    client.query_drop("INSERT INTO SummonResource VALUES (1, 20, 10, 11, 0, 0, 0, 1)").unwrap();
    client.query_drop("INSERT INTO SkillResource VALUES (123, 'before')").unwrap();
    client.query_drop("INSERT INTO StatResource VALUES (20, 1, 1, 1, 1, 1, 1, 1)").unwrap();

    let entry = SkillTreeEntry { skill_id: 123, jp_ratio: Decimal::new(25, 2), ..SkillTreeEntry::new_row(10, 2) };
    let tree = TreeCommit { summon_id: 1, slot: 1, original_tree_id: 10, entries: vec![entry], clone_if_shared: true };
    let skill_text = "a\\b';–";
    let request = CommitRequest {
        trees: vec![tree.clone()],
        stats: vec![StatCommit {
            summon_id: 1,
            original_stat_id: 20,
            stats: StatBlock { id: 20, values: [4; 7] },
            clone_if_shared: true,
        }],
        level_bonuses: vec![LevelBonusUpsert { bonus_id: 1, values: [Decimal::ONE; 7] }],
        random_groups: vec![RandomSkillGroup::new(9, [123])],
        skills: vec![SkillUpdate {
            skill_id: 123,
            changes: vec![FieldChange {
                name: "icon_file_name".into(),
                kind: FieldKind::Text,
                old: FieldValue::Text("before".into()),
                new: FieldValue::Text(skill_text.into()),
            }],
        }],
    };
    let commit = repo.commit(&request).unwrap();
    assert_eq!((commit.updated_trees, commit.updated_skills, commit.updated_random_groups), (1, 1, 1));
    assert_eq!(repo.skill_tree_entries(12).unwrap()[0].jp_ratio, Decimal::new(25, 2));
    assert_eq!(repo.stat_scaling(1).unwrap().base_stats.values, [4; 7]);
    assert_eq!(repo.random_groups(&[9]).unwrap()[&9].slots()[0], 123);
    assert_eq!(repo.skill_record(123).unwrap().fields[1].value, FieldValue::Text(skill_text.into()));

    let invalid = CommitRequest {
        trees: vec![TreeCommit { original_tree_id: 12, entries: vec![], clone_if_shared: false, ..tree.clone() }, tree],
        ..Default::default()
    };
    assert!(matches!(repo.commit(&invalid), Err(DataError::Query(_))));
    assert_eq!(repo.skill_tree_entries(12).unwrap().len(), 1);
}
