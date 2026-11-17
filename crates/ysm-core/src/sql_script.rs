//! Renders a [`CommitRequest`] as an executable SQL script, in T-SQL for
//! MSSQL or in plain SQL for PostgreSQL.
//!
//! The script mirrors what a direct database save does, statement for
//! statement, and runs as a single transaction. IDs of cloned trees and stat
//! rows are allocated by the script itself at execution time.

use std::fmt::Write;

use crate::commit::{CommitRequest, StatCommit, TreeCommit, is_sql_identifier, tree_slot_column};
use crate::model::{SkillTreeEntry, Stat};
use crate::random_group::RandomSkillGroup;

const TREE_COLUMNS: &str = "skill_tree_id, skill_id, skill_group_id, min_skill_lv, max_skill_lv, lv, job_lv, \
     need_skill_id_1, need_skill_lv_1, need_skill_id_2, need_skill_lv_2, need_skill_id_3, need_skill_lv_3, \
     cenhance_min, cenhance_max, jp_ratio";

/// The SQL flavour of a script.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SqlDialect {
    /// MSSQL.
    #[default]
    TSql,
    PostgreSql,
}

/// `generated_at` is only printed in the header comment.
pub fn render_script(request: &CommitRequest, generated_at: &str, dialect: SqlDialect) -> String {
    match dialect {
        SqlDialect::TSql => render_tsql(request, generated_at),
        SqlDialect::PostgreSql => postgres::render(request, generated_at),
    }
}

fn render_tsql(request: &CommitRequest, generated_at: &str) -> String {
    let mut sql = String::new();
    let w = &mut sql;

    let _ = writeln!(w, "-- YSM Summon Editor change script");
    let _ = writeln!(w, "-- Generated: {generated_at}");
    let _ = writeln!(w, "-- Runs as one transaction; any error rolls everything back.");
    let _ = writeln!(w);
    let _ = writeln!(w, "SET XACT_ABORT ON;");
    let _ = writeln!(w, "BEGIN TRANSACTION;");

    let mut clone_index = 0;
    for tree in &request.trees {
        let _ = writeln!(w);
        write_tree(w, tree, &mut clone_index);
    }

    let mut stat_clone_index = 0;
    for stat in request.stats.iter().filter(|s| s.original_stat_id > 0 && s.summon_id > 0) {
        let _ = writeln!(w);
        write_stat(w, stat, &mut stat_clone_index);
    }

    for bonus in request.level_bonuses.iter().filter(|b| b.bonus_id > 0) {
        let _ = writeln!(w);
        let _ = writeln!(w, "-- Growth bonus {}", bonus.bonus_id);
        let assignments = Stat::ALL.map(|s| format!("[{}] = {}", s.column(), bonus.values[s.index()])).join(", ");
        let columns = Stat::ALL.map(|s| format!("[{}]", s.column())).join(", ");
        let values = Stat::ALL.map(|s| bonus.values[s.index()].to_string()).join(", ");
        let id = bonus.bonus_id;
        let _ = writeln!(w, "IF EXISTS (SELECT 1 FROM [CreatureLevelBonus] WHERE [id] = {id})");
        let _ = writeln!(w, "    UPDATE [CreatureLevelBonus] SET {assignments} WHERE [id] = {id};");
        let _ = writeln!(w, "ELSE");
        let _ = writeln!(w, "    INSERT INTO [CreatureLevelBonus] ([id], {columns}) VALUES ({id}, {values});");
    }

    for group in request.random_groups.iter().filter(|g| g.group_id > 0) {
        let _ = writeln!(w);
        write_random_group(w, group);
    }

    for skill in &request.skills {
        let assignments: Vec<String> = skill
            .changes
            .iter()
            .filter(|c| is_sql_identifier(&c.name) && !c.name.eq_ignore_ascii_case("id"))
            .map(|c| format!("[{}] = {}", c.name, c.new.sql_literal()))
            .collect();
        if assignments.is_empty() {
            continue;
        }
        let _ = writeln!(w);
        let _ = writeln!(w, "-- Skill {}", skill.skill_id);
        let _ = writeln!(w, "UPDATE SkillResource SET {} WHERE id = {};", assignments.join(", "), skill.skill_id);
    }

    let _ = writeln!(w);
    let _ = writeln!(w, "COMMIT TRANSACTION;");
    sql
}

fn write_tree(w: &mut String, tree: &TreeCommit, clone_index: &mut u32) {
    let target = if tree.clone_if_shared {
        *clone_index += 1;
        let variable = format!("@NewTreeId{clone_index}");
        let _ = writeln!(
            w,
            "-- Summon {} slot {}: clone shared skill tree {} for this summon only",
            tree.summon_id, tree.slot, tree.original_tree_id
        );
        let _ = writeln!(
            w,
            "DECLARE {variable} int = (SELECT ISNULL(MAX(skill_tree_id), 0) + 1 FROM SkillTreeResource WITH (UPDLOCK, HOLDLOCK));"
        );
        if *clone_index > 1 {
            // An earlier clone without rows would otherwise be handed the same ID.
            let previous = format!("@NewTreeId{}", *clone_index - 1);
            let _ = writeln!(w, "IF {variable} <= {previous} SET {variable} = {previous} + 1;");
        }
        match tree_slot_column(tree.slot) {
            Some(column) => {
                let _ = writeln!(w, "UPDATE SummonResource SET [{column}] = {variable} WHERE id = {};", tree.summon_id);
            }
            None => {
                let _ = writeln!(w, "-- ERROR: invalid tree slot {}", tree.slot);
            }
        }
        variable
    } else {
        let _ = writeln!(
            w,
            "-- Summon {} slot {}: replace skill tree {}",
            tree.summon_id, tree.slot, tree.original_tree_id
        );
        let _ = writeln!(w, "DELETE FROM SkillTreeResource WHERE skill_tree_id = {};", tree.original_tree_id);
        tree.original_tree_id.to_string()
    };

    for entry in &tree.entries {
        let _ =
            writeln!(w, "INSERT INTO SkillTreeResource ({TREE_COLUMNS}) VALUES ({target}, {});", entry_values(entry));
    }
}

fn entry_values(e: &SkillTreeEntry) -> String {
    format!(
        "{}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}",
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
        e.jp_ratio
    )
}

fn write_stat(w: &mut String, stat: &StatCommit, clone_index: &mut u32) {
    let columns = Stat::ALL.map(|s| format!("[{}]", s.column())).join(", ");
    let values = Stat::ALL.map(|s| stat.stats.get(s).to_string()).join(", ");

    if stat.clone_if_shared {
        *clone_index += 1;
        let variable = format!("@NewStatId{clone_index}");
        let _ = writeln!(
            w,
            "-- Summon {}: clone shared stat row {} for this summon only",
            stat.summon_id, stat.original_stat_id
        );
        let _ = writeln!(
            w,
            "DECLARE {variable} int = (SELECT ISNULL(MAX([id]), 0) + 1 FROM [StatResource] WITH (UPDLOCK, HOLDLOCK));"
        );
        let _ = writeln!(w, "INSERT INTO [StatResource] ([id], {columns}) VALUES ({variable}, {values});");
        let _ = writeln!(w, "UPDATE [SummonResource] SET [stat_id] = {variable} WHERE [id] = {};", stat.summon_id);
    } else {
        let assignments = Stat::ALL.map(|s| format!("[{}] = {}", s.column(), stat.stats.get(s))).join(", ");
        let _ = writeln!(w, "-- Stat row {}", stat.original_stat_id);
        let _ = writeln!(w, "UPDATE [StatResource] SET {assignments} WHERE [id] = {};", stat.original_stat_id);
    }
}

fn write_random_group(w: &mut String, group: &RandomSkillGroup) {
    let id = group.group_id;
    let slots = group.slots();
    let assignments: Vec<String> =
        slots.iter().enumerate().map(|(i, skill)| format!("[skill_id_{}] = {skill}", i + 1)).collect();
    let columns: Vec<String> = (1..=slots.len()).map(|i| format!("[skill_id_{i}]")).collect();
    let values: Vec<String> = slots.iter().map(i32::to_string).collect();

    let _ = writeln!(w, "-- Random skill group {id}");
    let _ = writeln!(w, "IF EXISTS (SELECT 1 FROM [SummonRandomSkillResource] WHERE [sid] = {id})");
    let _ = writeln!(
        w,
        "    UPDATE [SummonRandomSkillResource] SET [use_code] = {}, {} WHERE [sid] = {id};",
        group.use_code(),
        assignments.join(", ")
    );
    let _ = writeln!(w, "ELSE");
    let _ = writeln!(
        w,
        "    INSERT INTO [SummonRandomSkillResource] ([sid], [use_code], {}) VALUES ({id}, {}, {});",
        columns.join(", "),
        group.use_code(),
        values.join(", ")
    );
}

/// PostgreSQL rendering: plain SQL statements, no procedural block.
///
/// Cloned IDs live in a temporary table dropped at commit; upserts are an
/// `UPDATE` followed by an `INSERT … WHERE NOT EXISTS`, so no unique
/// constraint is needed.
mod postgres {
    use std::fmt::Write;

    use crate::commit::{CommitRequest, StatCommit, TreeCommit, is_sql_identifier, tree_slot_column};
    use crate::model::Stat;
    use crate::random_group::RandomSkillGroup;

    use super::{TREE_COLUMNS, entry_values};

    const IDS: &str = "ysm_new_ids";

    fn new_id(name: &str) -> String {
        format!("(SELECT id FROM {IDS} WHERE name = '{name}')")
    }

    fn stat_columns() -> String {
        Stat::ALL.map(|s| format!("\"{}\"", s.column())).join(", ")
    }

    pub(super) fn render(request: &CommitRequest, generated_at: &str) -> String {
        let mut sql = String::new();
        let w = &mut sql;

        let _ = writeln!(w, "-- YSM Summon Editor change script (PostgreSQL)");
        let _ = writeln!(w, "-- Generated: {generated_at}");
        let _ = writeln!(w, "-- Runs as one transaction; any error rolls everything back.");
        let _ = writeln!(w);
        let _ = writeln!(w, "BEGIN;");

        let clones_tree = request.trees.iter().any(|t| t.clone_if_shared);
        let clones_stat = request.stats.iter().any(|s| s.clone_if_shared && s.original_stat_id > 0 && s.summon_id > 0);
        if clones_tree || clones_stat {
            let _ = writeln!(
                w,
                "CREATE TEMPORARY TABLE {IDS} (name text PRIMARY KEY, id integer NOT NULL) ON COMMIT DROP;"
            );
        }
        if clones_tree {
            let _ = writeln!(w, "LOCK TABLE SkillTreeResource IN SHARE ROW EXCLUSIVE MODE;");
        }
        if clones_stat {
            let _ = writeln!(w, "LOCK TABLE StatResource IN SHARE ROW EXCLUSIVE MODE;");
        }

        let mut clone_index = 0;
        for tree in &request.trees {
            let _ = writeln!(w);
            write_tree(w, tree, &mut clone_index);
        }

        let mut stat_clone_index = 0;
        for stat in request.stats.iter().filter(|s| s.original_stat_id > 0 && s.summon_id > 0) {
            let _ = writeln!(w);
            write_stat(w, stat, &mut stat_clone_index);
        }

        for bonus in request.level_bonuses.iter().filter(|b| b.bonus_id > 0) {
            let _ = writeln!(w);
            let _ = writeln!(w, "-- Growth bonus {}", bonus.bonus_id);
            let id = bonus.bonus_id;
            let assignments = Stat::ALL.map(|s| format!("\"{}\" = {}", s.column(), bonus.values[s.index()])).join(", ");
            let values = Stat::ALL.map(|s| bonus.values[s.index()].to_string()).join(", ");
            let _ = writeln!(w, "UPDATE CreatureLevelBonus SET {assignments} WHERE id = {id};");
            let _ = writeln!(
                w,
                "INSERT INTO CreatureLevelBonus (id, {}) SELECT {id}, {values} \
                 WHERE NOT EXISTS (SELECT 1 FROM CreatureLevelBonus WHERE id = {id});",
                stat_columns()
            );
        }

        for group in request.random_groups.iter().filter(|g| g.group_id > 0) {
            let _ = writeln!(w);
            write_random_group(w, group);
        }

        for skill in &request.skills {
            let assignments: Vec<String> = skill
                .changes
                .iter()
                .filter(|c| is_sql_identifier(&c.name) && !c.name.eq_ignore_ascii_case("id"))
                .map(|c| format!("\"{}\" = {}", c.name, c.new.pg_literal()))
                .collect();
            if assignments.is_empty() {
                continue;
            }
            let _ = writeln!(w);
            let _ = writeln!(w, "-- Skill {}", skill.skill_id);
            let _ = writeln!(w, "UPDATE SkillResource SET {} WHERE id = {};", assignments.join(", "), skill.skill_id);
        }

        let _ = writeln!(w);
        let _ = writeln!(w, "COMMIT;");
        sql
    }

    fn write_tree(w: &mut String, tree: &TreeCommit, clone_index: &mut u32) {
        let target = if tree.clone_if_shared {
            *clone_index += 1;
            let name = format!("tree{clone_index}");
            let _ = writeln!(
                w,
                "-- Summon {} slot {}: clone shared skill tree {} for this summon only",
                tree.summon_id, tree.slot, tree.original_tree_id
            );
            // An earlier clone without rows would otherwise be handed the same ID.
            let floor = if *clone_index > 1 {
                format!("{} + 1", new_id(&format!("tree{}", *clone_index - 1)))
            } else {
                "1".to_owned()
            };
            let _ = writeln!(
                w,
                "INSERT INTO {IDS} (name, id) SELECT '{name}', GREATEST(COALESCE(MAX(skill_tree_id), 0) + 1, {floor}) FROM SkillTreeResource;"
            );
            match tree_slot_column(tree.slot) {
                Some(column) => {
                    let _ = writeln!(
                        w,
                        "UPDATE SummonResource SET {column} = {} WHERE id = {};",
                        new_id(&name),
                        tree.summon_id
                    );
                }
                None => {
                    let _ = writeln!(w, "-- ERROR: invalid tree slot {}", tree.slot);
                }
            }
            new_id(&name)
        } else {
            let _ = writeln!(
                w,
                "-- Summon {} slot {}: replace skill tree {}",
                tree.summon_id, tree.slot, tree.original_tree_id
            );
            let _ = writeln!(w, "DELETE FROM SkillTreeResource WHERE skill_tree_id = {};", tree.original_tree_id);
            tree.original_tree_id.to_string()
        };

        for entry in &tree.entries {
            let _ = writeln!(
                w,
                "INSERT INTO SkillTreeResource ({TREE_COLUMNS}) VALUES ({target}, {});",
                entry_values(entry)
            );
        }
    }

    fn write_stat(w: &mut String, stat: &StatCommit, clone_index: &mut u32) {
        let values = Stat::ALL.map(|s| stat.stats.get(s).to_string()).join(", ");

        if stat.clone_if_shared {
            *clone_index += 1;
            let name = format!("stat{clone_index}");
            let _ = writeln!(
                w,
                "-- Summon {}: clone shared stat row {} for this summon only",
                stat.summon_id, stat.original_stat_id
            );
            let _ = writeln!(
                w,
                "INSERT INTO {IDS} (name, id) SELECT '{name}', COALESCE(MAX(id), 0) + 1 FROM StatResource;"
            );
            let _ =
                writeln!(w, "INSERT INTO StatResource (id, {}) VALUES ({}, {values});", stat_columns(), new_id(&name));
            let _ = writeln!(w, "UPDATE SummonResource SET stat_id = {} WHERE id = {};", new_id(&name), stat.summon_id);
        } else {
            let assignments = Stat::ALL.map(|s| format!("\"{}\" = {}", s.column(), stat.stats.get(s))).join(", ");
            let _ = writeln!(w, "-- Stat row {}", stat.original_stat_id);
            let _ = writeln!(w, "UPDATE StatResource SET {assignments} WHERE id = {};", stat.original_stat_id);
        }
    }

    fn write_random_group(w: &mut String, group: &RandomSkillGroup) {
        let id = group.group_id;
        let slots = group.slots();
        let assignments: Vec<String> =
            slots.iter().enumerate().map(|(i, skill)| format!("skill_id_{} = {skill}", i + 1)).collect();
        let columns: Vec<String> = (1..=slots.len()).map(|i| format!("skill_id_{i}")).collect();
        let values: Vec<String> = slots.iter().map(i32::to_string).collect();

        let _ = writeln!(w, "-- Random skill group {id}");
        let _ = writeln!(
            w,
            "UPDATE SummonRandomSkillResource SET use_code = {}, {} WHERE sid = {id};",
            group.use_code(),
            assignments.join(", ")
        );
        let _ = writeln!(
            w,
            "INSERT INTO SummonRandomSkillResource (sid, use_code, {}) SELECT {id}, {}, {} \
             WHERE NOT EXISTS (SELECT 1 FROM SummonRandomSkillResource WHERE sid = {id});",
            columns.join(", "),
            group.use_code(),
            values.join(", ")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commit::{FieldChange, LevelBonusUpsert, SkillUpdate};
    use crate::model::StatBlock;
    use crate::value::{FieldKind, FieldValue};
    use rust_decimal::Decimal;

    fn tree(clone: bool, entries: Vec<SkillTreeEntry>) -> TreeCommit {
        TreeCommit { summon_id: 3, slot: 2, original_tree_id: 30, entries, clone_if_shared: clone }
    }

    #[test]
    fn script_is_wrapped_in_a_transaction() {
        let sql = render_script(&CommitRequest::default(), "2026-11-17 10:00:00", SqlDialect::TSql);
        assert!(sql.contains("-- Generated: 2026-11-17 10:00:00"));
        assert!(sql.contains("SET XACT_ABORT ON;\nBEGIN TRANSACTION;"));
        assert!(sql.trim_end().ends_with("COMMIT TRANSACTION;"));
    }

    #[test]
    fn in_place_tree_is_deleted_and_reinserted() {
        let mut entry = SkillTreeEntry::new_row(30, 5);
        entry.skill_id = 400;
        entry.need_skill_id[1] = 9;
        let request = CommitRequest { trees: vec![tree(false, vec![entry])], ..Default::default() };
        let sql = render_script(&request, "now", SqlDialect::TSql);

        assert!(sql.contains("DELETE FROM SkillTreeResource WHERE skill_tree_id = 30;"));
        assert!(sql.contains("VALUES (30, 400, 0, 1, 1, 5, 1, 0, 0, 9, 0, 0, 0, 0, 0, 1);"));
        assert!(!sql.contains("DECLARE"));
    }

    #[test]
    fn cloned_tree_allocates_an_id_and_repoints_the_summon_slot() {
        let entry = SkillTreeEntry::new_row(30, 1);
        let request = CommitRequest { trees: vec![tree(true, vec![entry]), tree(true, vec![])], ..Default::default() };
        let sql = render_script(&request, "now", SqlDialect::TSql);

        assert!(sql.contains("DECLARE @NewTreeId1 int = (SELECT ISNULL(MAX(skill_tree_id), 0) + 1"));
        assert!(sql.contains("UPDATE SummonResource SET [skill_tree2_id] = @NewTreeId1 WHERE id = 3;"));
        assert!(sql.contains("VALUES (@NewTreeId1, 0, 0, 1, 1, 1, 1,"));
        assert!(sql.contains("IF @NewTreeId2 <= @NewTreeId1 SET @NewTreeId2 = @NewTreeId1 + 1;"));
        assert!(!sql.contains("DELETE FROM SkillTreeResource"));
    }

    #[test]
    fn stats_bonuses_and_groups_are_executable_statements() {
        let request = CommitRequest {
            stats: vec![
                StatCommit {
                    summon_id: 3,
                    original_stat_id: 90,
                    stats: StatBlock { id: 90, values: [1, 2, 3, 4, 5, 6, 7] },
                    clone_if_shared: false,
                },
                StatCommit {
                    summon_id: 4,
                    original_stat_id: 91,
                    stats: StatBlock { id: 91, values: [9; 7] },
                    clone_if_shared: true,
                },
            ],
            level_bonuses: vec![LevelBonusUpsert { bonus_id: 12, values: [Decimal::new(15, 1); 7] }],
            random_groups: vec![RandomSkillGroup::new(7, [20, 10])],
            ..Default::default()
        };
        let sql = render_script(&request, "now", SqlDialect::TSql);

        assert!(sql.contains(
            "UPDATE [StatResource] SET [str] = 1, [vit] = 2, [dex] = 3, [agi] = 4, [int] = 5, [men] = 6, [luk] = 7 WHERE [id] = 90;"
        ));
        assert!(sql.contains("DECLARE @NewStatId1 int"));
        assert!(sql.contains("UPDATE [SummonResource] SET [stat_id] = @NewStatId1 WHERE [id] = 4;"));
        assert!(sql.contains("IF EXISTS (SELECT 1 FROM [CreatureLevelBonus] WHERE [id] = 12)"));
        assert!(sql.contains("    UPDATE [CreatureLevelBonus] SET [str] = 1.5,"));
        assert!(sql.contains("IF EXISTS (SELECT 1 FROM [SummonRandomSkillResource] WHERE [sid] = 7)"));
        assert!(sql.contains("SET [use_code] = 2, [skill_id_1] = 10, [skill_id_2] = 20, [skill_id_3] = 0,"));
        assert!(sql.lines().all(|line| !line.trim_start().starts_with("-- IF EXISTS")), "nothing is commented out");
    }

    #[test]
    fn skill_updates_use_typed_literals_and_skip_unsafe_columns() {
        let change = |name: &str, new: FieldValue| FieldChange {
            name: name.into(),
            kind: FieldKind::Text,
            old: FieldValue::Null,
            new,
        };
        let request = CommitRequest {
            skills: vec![SkillUpdate {
                skill_id: 55,
                changes: vec![
                    change("var1", FieldValue::F64(1.5)),
                    change("note", FieldValue::Text("it's".into())),
                    change("id", FieldValue::I32(1)),
                    change("x]; DROP TABLE y;--", FieldValue::I32(1)),
                ],
            }],
            ..Default::default()
        };
        let sql = render_script(&request, "now", SqlDialect::TSql);

        assert!(sql.contains("UPDATE SkillResource SET [var1] = 1.5, [note] = N'it''s' WHERE id = 55;"));
        assert!(!sql.contains("DROP TABLE"));
    }

    #[test]
    fn postgres_script_is_plain_sql_in_one_transaction() {
        let mut entry = SkillTreeEntry::new_row(30, 5);
        entry.skill_id = 400;
        let request = CommitRequest {
            trees: vec![tree(false, vec![entry]), tree(true, vec![entry]), tree(true, vec![])],
            stats: vec![StatCommit {
                summon_id: 4,
                original_stat_id: 91,
                stats: StatBlock { id: 91, values: [9; 7] },
                clone_if_shared: true,
            }],
            level_bonuses: vec![LevelBonusUpsert { bonus_id: 12, values: [Decimal::new(15, 1); 7] }],
            random_groups: vec![RandomSkillGroup::new(7, [20, 10])],
            ..Default::default()
        };
        let sql = render_script(&request, "now", SqlDialect::PostgreSql);

        assert!(sql.contains("\nBEGIN;\n"));
        assert!(sql.trim_end().ends_with("COMMIT;"));
        for tsql in ["[", "@", "ISNULL", "DECLARE", "XACT_ABORT", "N'", "IF ", "$$"] {
            assert!(!sql.contains(tsql), "T-SQL '{tsql}' left in:\n{sql}");
        }
        assert!(sql.contains("DELETE FROM SkillTreeResource WHERE skill_tree_id = 30;"));
        assert!(sql.contains("VALUES (30, 400, 0, 1, 1, 5, 1, 0, 0, 0, 0, 0, 0, 0, 0, 1);"));
        assert!(sql.contains(
            "SELECT 'tree2', GREATEST(COALESCE(MAX(skill_tree_id), 0) + 1, (SELECT id FROM ysm_new_ids WHERE name = 'tree1') + 1)"
        ));
        assert!(sql.contains(
            "UPDATE SummonResource SET skill_tree2_id = (SELECT id FROM ysm_new_ids WHERE name = 'tree1') WHERE id = 3;"
        ));
        assert!(sql.contains("VALUES ((SELECT id FROM ysm_new_ids WHERE name = 'tree1'), 400,"));
        assert!(sql.contains(
            "UPDATE SummonResource SET stat_id = (SELECT id FROM ysm_new_ids WHERE name = 'stat1') WHERE id = 4;"
        ));
        assert!(sql.contains("\"int\" = 1.5"));
        assert!(sql.contains("WHERE NOT EXISTS (SELECT 1 FROM CreatureLevelBonus WHERE id = 12);"));
        assert!(sql.contains("UPDATE SummonRandomSkillResource SET use_code = 2, skill_id_1 = 10, skill_id_2 = 20,"));
        assert!(sql.contains("WHERE NOT EXISTS (SELECT 1 FROM SummonRandomSkillResource WHERE sid = 7);"));
    }

    #[test]
    fn postgres_skill_updates_quote_columns_and_use_standard_literals() {
        let change = |name: &str, new: FieldValue| FieldChange {
            name: name.into(),
            kind: FieldKind::Text,
            old: FieldValue::Null,
            new,
        };
        let request = CommitRequest {
            skills: vec![SkillUpdate {
                skill_id: 55,
                changes: vec![
                    change("var1", FieldValue::Decimal(Decimal::new(15, 1))),
                    change("note", FieldValue::Text("it's".into())),
                    change("flag", FieldValue::Bool(true)),
                    change("x]; DROP TABLE y;--", FieldValue::I32(1)),
                ],
            }],
            ..Default::default()
        };
        let sql = render_script(&request, "now", SqlDialect::PostgreSql);

        assert!(
            sql.contains("UPDATE SkillResource SET \"var1\" = 1.5, \"note\" = 'it''s', \"flag\" = true WHERE id = 55;")
        );
        assert!(!sql.contains("DROP TABLE"));
        assert!(!sql.contains("CREATE TEMPORARY TABLE"), "no clone, no ID table");
    }
}
