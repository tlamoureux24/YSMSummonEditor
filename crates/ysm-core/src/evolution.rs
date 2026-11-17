//! Evolution chain resolution.

use crate::model::{EvolutionChain, SummonListItem};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChainError {
    #[error("{}", crate::i18n::tr("selected summon is not a final evolution stage (evolve_target != 0)"))]
    NotFinalStage,
    #[error("{}", crate::i18n::tr("evolution chain card_id mismatch detected"))]
    CardMismatch,
}

/// Builds the fixed three-stage chain ending at `stage3`.
///
/// `candidates` are the summons sharing the final stage's card; earlier
/// stages are the ones whose `evolve_target` points at the next stage.
pub fn resolve_chain(stage3: &SummonListItem, candidates: &[SummonListItem]) -> Result<EvolutionChain, ChainError> {
    if stage3.evolve_target != 0 {
        return Err(ChainError::NotFinalStage);
    }

    let evolves_into = |target: i32| candidates.iter().find(|s| s.evolve_target == target).cloned();
    let stage2 = evolves_into(stage3.summon_id);
    let stage1 = stage2.as_ref().and_then(|s| evolves_into(s.summon_id));

    let same_card = |s: &Option<SummonListItem>| s.as_ref().is_none_or(|s| s.card_id == stage3.card_id);
    if !same_card(&stage1) || !same_card(&stage2) {
        return Err(ChainError::CardMismatch);
    }

    Ok(EvolutionChain { stage1, stage2, stage3: stage3.clone() })
}

/// Number of growth stages a summon of the given form has accumulated.
pub fn growth_depth(form: i32) -> i32 {
    match form {
        i32::MIN..=1 => 1,
        2 => 2,
        _ => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summon(id: i32, card_id: i32, evolve_target: i32) -> SummonListItem {
        SummonListItem {
            summon_id: id,
            summon_name: format!("Summon {id}"),
            card_name: None,
            card_id,
            evolve_target,
            form: 1,
            is_riding_only: false,
            face_file_name: None,
            illust_file_name: None,
            feature_text: None,
            stat_id: id,
            skill_tree_ids: [0; 5],
        }
    }

    #[test]
    fn resolves_all_three_stages() {
        let all = [summon(1, 9, 2), summon(2, 9, 3), summon(3, 9, 0)];
        let chain = resolve_chain(&all[2], &all).unwrap();

        assert_eq!(chain.summon_ids(), [1, 2, 3]);
        assert_eq!(chain.stage(1).unwrap().summon_id, 1);
    }

    #[test]
    fn single_stage_chain_has_only_the_final_summon() {
        let all = [summon(3, 9, 0)];
        let chain = resolve_chain(&all[0], &all).unwrap();

        assert!(chain.stage1.is_none());
        assert!(chain.stage2.is_none());
        assert_eq!(chain.summon_ids(), [3]);
    }

    #[test]
    fn rejects_a_non_final_summon() {
        let all = [summon(2, 9, 3), summon(3, 9, 0)];
        assert_eq!(resolve_chain(&all[0], &all), Err(ChainError::NotFinalStage));
    }

    #[test]
    fn rejects_a_chain_spanning_two_cards() {
        let all = [summon(2, 8, 3), summon(3, 9, 0)];
        assert_eq!(resolve_chain(&all[1], &all), Err(ChainError::CardMismatch));
    }

    #[test]
    fn growth_depth_follows_form() {
        assert_eq!(growth_depth(0), 1);
        assert_eq!(growth_depth(1), 1);
        assert_eq!(growth_depth(2), 2);
        assert_eq!(growth_depth(3), 3);
        assert_eq!(growth_depth(7), 3);
    }
}
