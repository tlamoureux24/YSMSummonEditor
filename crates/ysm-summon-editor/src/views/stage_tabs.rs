//! Evolution tabs. They sit above both lower cards because the selected
//! evolution decides what the skill tree card and the effective skills card show.

use eframe::egui::Ui;
use ysm_core::model::STAGE_COUNT;
use ysm_core::session::TreeState;

use super::ViewContext;
use crate::action::Action;
use crate::widgets::{self, Tab};
use ysm_core::i18n::{tr, trf};

pub fn show(ui: &mut Ui, view: &mut ViewContext<'_>) {
    let state = view.state;
    let session = &state.session;

    let tabs: Vec<Tab> = (1..=STAGE_COUNT as u8)
        .map(|stage| {
            let summon = session.chain().and_then(|chain| chain.stage(stage));
            let dirty =
                session.stages().iter().filter(|s| s.stage == stage).flat_map(|s| &s.trees).any(TreeState::is_changed);
            Tab {
                title: trf("Evolution {}", &[&stage]),
                subtitle: summon.map_or_else(String::new, |s| format!("{} · {}", s.summon_name, s.summon_id)),
                enabled: summon.is_some(),
                dirty,
                tooltip: match summon {
                    Some(summon) => trf("Skill trees and effective skills of {}", &[&summon.summon_name]),
                    None if session.is_loaded() => trf("This chain has no evolution {}", &[&stage]),
                    None => tr("Select a summon first").to_owned(),
                },
            }
        })
        .collect();

    if let Some(index) = widgets::tab_strip(ui, &tabs, usize::from(state.active_stage.saturating_sub(1))) {
        view.actions.push(Action::SetStage(index as u8 + 1));
    }
}
