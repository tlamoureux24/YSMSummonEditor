//! Human-readable list of everything a save is about to change.

use std::collections::HashMap;

use rust_decimal::Decimal;

use crate::commit::{CommitRequest, SkillUpdate};
use crate::i18n::{tr, trf};
use crate::model::{EntryField, SkillTreeEntry, Stat};
use crate::random_group::RandomSkillGroup;
use crate::scaling_edit::{LevelBonusEdit, StatEdit};
use crate::session::{EditSession, TreeChange, TreeChangeKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewKind {
    Insert,
    Update,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewRow {
    pub kind: ReviewKind,
    /// What sort of data changes, e.g. "Skill tree" or "Stat".
    pub category: &'static str,
    pub target: String,
    pub field: String,
    pub before: String,
    pub after: String,
}

/// Builds the review for a session and the final request about to be saved.
///
/// Random groups are taken from `request` rather than the session because
/// the save planner may have cloned them under new IDs.
pub fn build_review_rows(session: &EditSession, request: &CommitRequest) -> Vec<ReviewRow> {
    let db_groups: HashMap<i32, &RandomSkillGroup> =
        request.random_groups.iter().filter_map(|g| session.db_group(g.group_id).map(|db| (g.group_id, db))).collect();

    let mut rows = Vec::new();
    for change in session.tree_changes() {
        tree_rows(&change, &mut rows);
    }
    for update in session.pending_skills() {
        skill_rows(update, &mut rows);
    }
    for edit in session.pending_stats() {
        stat_rows(edit, &mut rows);
    }
    for edit in session.pending_bonuses() {
        bonus_rows(edit, &mut rows);
    }
    for group in &request.random_groups {
        group_rows(group, db_groups.get(&group.group_id).copied(), &mut rows);
    }
    rows
}

fn tree_rows(change: &TreeChange, rows: &mut Vec<ReviewRow>) {
    let location = trf("Evolution {} · Tree {} · #{}", &[&change.tree.stage, &change.tree.slot, &change.tree_id]);
    match (change.kind, &change.before, &change.after) {
        (TreeChangeKind::Insert, _, Some(after)) => rows.push(ReviewRow {
            kind: ReviewKind::Insert,
            category: tr("Skill tree"),
            target: location,
            field: trf("skill {}", &[&after.skill_id]),
            before: String::new(),
            after: summarize_entry(after),
        }),
        (TreeChangeKind::Delete, Some(before), _) => rows.push(ReviewRow {
            kind: ReviewKind::Delete,
            category: tr("Skill tree"),
            target: location,
            field: trf("skill {}", &[&before.skill_id]),
            before: summarize_entry(before),
            after: String::new(),
        }),
        (TreeChangeKind::Update, Some(before), Some(after)) => {
            for field in EntryField::ALL.into_iter().filter(|f| before.get(*f) != after.get(*f)) {
                rows.push(ReviewRow {
                    kind: ReviewKind::Update,
                    category: tr("Skill tree"),
                    target: trf("{} · skill {}", &[&location, &before.skill_id]),
                    field: field.column().to_owned(),
                    before: before.get(field).to_string(),
                    after: after.get(field).to_string(),
                });
            }
        }
        _ => {}
    }
}

fn summarize_entry(entry: &SkillTreeEntry) -> String {
    let mut text =
        trf("lv {} · job {} · skill lv {}–{}", &[&entry.lv, &entry.job_lv, &entry.min_skill_lv, &entry.max_skill_lv]);
    if entry.skill_group_id > 0 {
        text.push_str(&trf(" · group {}", &[&entry.skill_group_id]));
    }
    for i in 0..3 {
        if entry.need_skill_id[i] > 0 {
            text.push_str(&trf(" · needs {} lv {}", &[&entry.need_skill_id[i], &entry.need_skill_lv[i]]));
        }
    }
    if entry.cenhance_min != 0 || entry.cenhance_max != 0 {
        text.push_str(&trf(" · enhance {}–{}", &[&entry.cenhance_min, &entry.cenhance_max]));
    }
    text
}

fn skill_rows(update: &SkillUpdate, rows: &mut Vec<ReviewRow>) {
    for change in &update.changes {
        rows.push(ReviewRow {
            kind: ReviewKind::Update,
            category: tr("Skill"),
            target: trf("Skill {}", &[&update.skill_id]),
            field: change.name.clone(),
            before: display_or_null(&change.old),
            after: display_or_null(&change.new),
        });
    }
}

fn display_or_null(value: &crate::value::FieldValue) -> String {
    if value.is_null() { "NULL".to_owned() } else { value.to_string() }
}

fn stat_rows(edit: &StatEdit, rows: &mut Vec<ReviewRow>) {
    for stat in Stat::ALL {
        let (before, after) = (edit.original.get(stat), edit.commit.stats.get(stat));
        if before != after {
            rows.push(ReviewRow {
                kind: ReviewKind::Update,
                category: tr("Stat"),
                target: trf("Stat {}", &[&edit.commit.original_stat_id]),
                field: stat.column().to_owned(),
                before: before.to_string(),
                after: after.to_string(),
            });
        }
    }
}

fn bonus_rows(edit: &LevelBonusEdit, rows: &mut Vec<ReviewRow>) {
    for stat in Stat::ALL {
        let (before, after) = (edit.original[stat.index()], edit.upsert.values[stat.index()]);
        if before != after {
            rows.push(ReviewRow {
                kind: if edit.exists_in_db { ReviewKind::Update } else { ReviewKind::Insert },
                category: tr("Growth bonus"),
                target: trf("Bonus {}", &[&edit.upsert.bonus_id]),
                field: stat.column().to_owned(),
                before: if edit.exists_in_db { format_bonus(before) } else { String::new() },
                after: format_bonus(after),
            });
        }
    }
}

fn format_bonus(value: Decimal) -> String {
    format!("{value:.4}")
}

fn group_rows(group: &RandomSkillGroup, db: Option<&RandomSkillGroup>, rows: &mut Vec<ReviewRow>) {
    let kind = if db.is_some() { ReviewKind::Update } else { ReviewKind::Insert };
    let join = |g: &RandomSkillGroup| g.skill_ids().iter().map(i32::to_string).collect::<Vec<_>>().join(", ");
    let target = trf("Group {}", &[&group.group_id]);

    rows.push(ReviewRow {
        kind,
        category: tr("Random group"),
        target: target.clone(),
        field: "use_code".to_owned(),
        before: db.map(|g| g.use_code().to_string()).unwrap_or_default(),
        after: group.use_code().to_string(),
    });
    rows.push(ReviewRow {
        kind,
        category: tr("Random group"),
        target,
        field: "skills".to_owned(),
        before: db.map(join).unwrap_or_default(),
        after: join(group),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commit::FieldChange;
    use crate::scaling_edit::ScalingDraft;
    use crate::session::TreeRef;
    use crate::session::test_support::{first_uid, loaded};
    use crate::value::{FieldKind, FieldValue};

    const T11: TreeRef = TreeRef { stage: 1, slot: 1 };

    #[test]
    fn tree_update_lists_each_changed_field() {
        let mut session = loaded();
        let uid = first_uid(&session, T11);
        session.set_field(T11, uid, EntryField::JobLv, 40);
        session.set_field(T11, uid, EntryField::MaxSkillLv, 10);

        let rows = build_review_rows(&session, &session.build_commit_request());
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.kind == ReviewKind::Update && r.category == "Skill tree"));
        assert_eq!(rows[0].target, "Evolution 1 · Tree 1 · #10 · skill 100");
        let fields: Vec<_> = rows.iter().map(|r| (r.field.as_str(), r.before.as_str(), r.after.as_str())).collect();
        assert_eq!(fields, [("max_skill_lv", "1", "10"), ("job_lv", "1", "40")]);
    }

    #[test]
    fn inserts_and_deletes_are_summarized() {
        let mut session = loaded();
        let uid = first_uid(&session, T11);
        session.delete_row(T11, uid);
        session.add_row(T11, 555);

        let rows = build_review_rows(&session, &session.build_commit_request());
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].kind, ReviewKind::Insert);
        assert_eq!(rows[0].field, "skill 555");
        assert!(rows[0].before.is_empty());
        assert!(rows[0].after.starts_with("lv 6"));
        assert_eq!(rows[1].kind, ReviewKind::Delete);
        assert_eq!(rows[1].field, "skill 100");
        assert!(rows[1].after.is_empty());
    }

    #[test]
    fn skill_stat_and_bonus_edits_show_before_and_after() {
        let mut session = loaded();
        session.stage_skill_update(SkillUpdate {
            skill_id: 100,
            changes: vec![FieldChange {
                name: "var1".into(),
                kind: FieldKind::F64,
                old: FieldValue::Null,
                new: FieldValue::F64(2.5),
            }],
        });

        let db = crate::model::StatScalingSnapshot {
            max_level: None,
            summon_id: 3,
            stat_id: 90,
            evolve_type: 1,
            growth_depth: 1,
            base_stats: crate::model::StatBlock { id: 90, values: [1; 7] },
            stage_bonuses: vec![crate::model::LevelBonusStage {
                stage_index: 1,
                lookup_id: 3,
                values: [Decimal::ONE; 7],
                exists_in_db: false,
            }],
        };
        let mut draft = ScalingDraft::new(&db, &db);
        draft.base[Stat::Dex.index()] = 9;
        draft.bonuses[0].values[Stat::Int.index()] = Decimal::new(15, 1);
        session.apply_scaling_edit(draft.build_result());

        let rows = build_review_rows(&session, &session.build_commit_request());
        let as_tuple =
            |r: &ReviewRow| (r.category, r.target.clone(), r.field.clone(), r.before.clone(), r.after.clone());
        assert_eq!(as_tuple(&rows[0]), ("Skill", "Skill 100".into(), "var1".into(), "NULL".into(), "2.5".into()));
        assert_eq!(as_tuple(&rows[1]), ("Stat", "Stat 90".into(), "dex".into(), "1".into(), "9".into()));
        assert_eq!(as_tuple(&rows[2]), ("Growth bonus", "Bonus 3".into(), "int".into(), "".into(), "1.5000".into()));
        assert_eq!(rows[2].kind, ReviewKind::Insert, "the bonus row does not exist yet");
    }

    #[test]
    fn random_group_rows_compare_against_the_database() {
        let mut session = loaded();
        session.stage_random_group(RandomSkillGroup::new(7, [900, 100]));
        session.stage_random_group(RandomSkillGroup::new(50, [1, 2]));

        let rows = build_review_rows(&session, &session.build_commit_request());
        assert_eq!(rows.len(), 4);
        assert_eq!((rows[0].kind, rows[0].before.as_str(), rows[0].after.as_str()), (ReviewKind::Update, "1", "2"));
        assert_eq!((rows[1].before.as_str(), rows[1].after.as_str()), ("900", "100, 900"));
        assert_eq!(rows[2].kind, ReviewKind::Insert);
        assert_eq!(rows[3].target, "Group 50");
        assert_eq!(rows[3].after, "1, 2");
    }
}
