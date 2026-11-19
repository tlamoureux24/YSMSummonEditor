use ysm_core::model::SummonListFilter;
use ysm_core::scaling::aurumis_progression_at;
use ysm_data::{DatabaseEngine, SettingsStore, SqlRepository, SummonRepository, loader};

#[test]
#[ignore = "reads the local MariaDB settings and live database"]
fn loads_live_summons_and_their_resources() {
    let settings = SettingsStore::at_default_location().load().unwrap();
    assert_eq!(settings.database.engine, DatabaseEngine::MariaDb);
    let mut repo = SqlRepository::new(settings.database).unwrap();
    assert!(repo.test_connection().unwrap().contains("MariaDB"));

    let summons = repo
        .search_summons(&SummonListFilter {
            search_text: "6001".into(),
            include_non_form1: true,
            show_all_evolutions: false,
            ..Default::default()
        })
        .unwrap();
    let summon = &summons[0];
    assert_eq!(summon.summon_id, 6003);
    let chain = loader::load_chain(&mut repo, summon).unwrap();
    assert_eq!(chain.chain.summon_ids(), [6001, 6002, 6003]);
    let rows = repo.skill_tree_entries(summon.skill_tree_ids[0]).unwrap();
    let skill_id = rows.iter().find(|row| row.skill_id > 0).unwrap().skill_id;
    let group_id = rows.iter().find(|row| row.skill_group_id > 0).unwrap().skill_group_id;
    assert!(repo.skill_record(skill_id).unwrap().fields.len() > 100);
    assert_eq!(repo.search_skills(&skill_id.to_string(), 1).unwrap()[0].skill_id, skill_id);
    assert!(repo.random_groups(&[group_id]).unwrap().contains_key(&group_id));
    assert!(!repo.random_group_usage(&[group_id]).unwrap().is_empty());
    assert!(!repo.tree_usage(&[summon.skill_tree_ids[0]]).unwrap().is_empty());
    assert!(!repo.level_bonus_usage(&[summon.summon_id]).unwrap().is_empty());
    let forms: Vec<_> = chain.chain.summon_ids().into_iter().map(|id| repo.stat_scaling(id).unwrap()).collect();
    for (level, expected_stage, form) in [(49, 1, &forms[0]), (50, 2, &forms[1]), (100, 3, &forms[2])] {
        let (stage, preview) = aurumis_progression_at(&forms, level, &Default::default(), 0).unwrap();
        assert_eq!(stage, expected_stage);
        assert_eq!(preview.rows[0].base, form.base_stats.values[0]);
    }
    assert!(!repo.creature_enhance().unwrap().is_empty());

    let index = ysm_data::AssetIndex::build(std::path::Path::new(&settings.dump_folder_path));
    assert!(index.resolve(summon.face_file_name.as_deref().unwrap()).is_some());
    assert!(index.resolve(summon.illust_file_name.as_deref().unwrap()).is_some());
}
