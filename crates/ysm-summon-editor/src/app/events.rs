//! Applies results coming back from the worker thread.

use ysm_core::random_group::{RandomSkillGroup, next_free_group_id};
use ysm_core::scaling::GrowthModel;
use ysm_data::DatabaseEngine;

use super::EditorApp;
use crate::dialogs::random_group::RandomGroupDialog;
use crate::dialogs::skill_edit::SkillEditDialog;
use crate::dialogs::stat_scaling::StatScalingDialog;
use crate::dialogs::{Dialog, MessageDialog};
use crate::state::ConnectionStatus;
use crate::widgets::toast::ToastKind;
use crate::worker::Event;
use ysm_core::i18n::{tr, trf};

impl EditorApp {
    pub(super) fn handle_event(&mut self, event: Event) {
        if event.is_repository_answer() {
            self.state.note_request_outcome(event.failure());
        }

        match event {
            Event::Connected(Ok(server)) => {
                self.state.connected(server);
                self.search_summons();
            }
            Event::Connected(Err(error)) => {
                self.state.connection = ConnectionStatus::Failed(error.message.clone());
                self.state.summons.clear();
                self.set_status(tr("Database connection failed (see details)."));
                self.push_dialog(Dialog::message(MessageDialog::error(
                    tr("Database"),
                    tr("Could not connect to the database. Check the connection in Settings."),
                    error,
                )));
            }
            // Only the settings dialog cares; it has already seen the event.
            Event::ConnectionTested(_) | Event::SkillsFound { .. } => {}

            Event::AssetIndexBuilt(index) => {
                self.state.asset_count = Some(index.len());
                self.textures.set_index(index);
            }

            Event::SummonsFound { generation, result } => {
                if generation != self.summon_generation {
                    return;
                }
                self.state.summons_loading = false;
                match result {
                    Ok(summons) => {
                        let count = summons.len();
                        self.set_status(if count == 1 {
                            trf("Loaded {} summon.", &[&count])
                        } else {
                            trf("Loaded {} summons.", &[&count])
                        });
                        self.state.summons = summons;
                    }
                    Err(error) => {
                        self.set_status(tr("Search failed (see details)."));
                        self.push_dialog(Dialog::message(MessageDialog::error(
                            tr("Search failed"),
                            tr("Search query failed."),
                            error,
                        )));
                    }
                }
            }

            Event::ChainLoaded { generation, summon, result } => {
                if generation != self.chain_generation {
                    return;
                }
                self.state.chain_loading = false;
                match result {
                    Ok(bundle) => {
                        // Open the tab of the summon that was clicked, not always the first stage.
                        let stage = (1..=3)
                            .find(|stage| bundle.chain.stage(*stage).is_some_and(|s| s.summon_id == summon.summon_id));
                        self.state.session.load(*bundle);
                        self.state.active_stage = stage.unwrap_or(3);
                        self.state.active_slot = 1;
                        self.set_status(tr("Chain loaded."));
                    }
                    Err(error) => {
                        self.set_status(tr("Chain load failed (see details)."));
                        self.push_dialog(Dialog::message(MessageDialog::error(
                            tr("Chain load failed"),
                            tr("Could not load the evolution chain."),
                            error,
                        )));
                    }
                }
            }

            Event::SkillDisplayLoaded(Ok(found)) => self.state.session.ingest_skill_display(found),
            Event::SkillDisplayLoaded(Err(error)) => {
                self.toasts.push(ToastKind::Warning, trf("Skill names could not be loaded. {}", &[&error]));
            }

            Event::RandomGroupsLoaded { requested, result: Ok(found) } => {
                self.state.session.ingest_random_groups(&requested, found);
            }
            Event::RandomGroupsLoaded { result: Err(error), .. } => {
                self.toasts.push(ToastKind::Warning, trf("Random groups could not be loaded. {}", &[&error]));
            }

            Event::SkillRecordLoaded { skill_id, result } => match result {
                Ok(record) => {
                    self.set_status(trf("Skill {} loaded.", &[&skill_id]));
                    let session = &self.state.session;
                    let dialog =
                        SkillEditDialog::new(record, session.pending_skill(skill_id), session.skill_name(skill_id));
                    self.push_dialog(Dialog::SkillEdit(Box::new(dialog)));
                }
                Err(error) => self.push_dialog(Dialog::message(MessageDialog::error(
                    tr("Skill edit failed"),
                    tr("Could not open the skill editor."),
                    error,
                ))),
            },

            Event::ScalingLoaded(Ok(data)) => {
                let mut db = data.current;
                let Some(chain) = self.state.session.chain().filter(|chain| chain.stage3.summon_id == db.summon_id)
                else {
                    return;
                };
                let name = chain.stage3.summon_name.clone();
                // AurumisRZ (MariaDB) and the Rappelz AIO NG server (PostgreSQL) both run the
                // official GetSummonLevelBonus: the current form's bonus row only.
                let model = if !self.state.demo
                    && matches!(
                        self.state.settings.database.engine,
                        DatabaseEngine::MariaDb | DatabaseEngine::PostgreSql
                    ) {
                    GrowthModel::Aurumis
                } else {
                    GrowthModel::Legacy
                };
                if model == GrowthModel::Aurumis {
                    // The engine looks up the bonus by the actual form ID. Linked forms
                    // are not necessarily consecutive resource IDs.
                    db.stage_bonuses = data
                        .earlier
                        .iter()
                        .chain(std::iter::once(&db))
                        .filter_map(|form| form.stage_bonuses.iter().find(|bonus| bonus.lookup_id == form.summon_id))
                        .cloned()
                        .collect();
                }
                let staged = self.state.session.pending_scaling(&db);
                let earlier = data.earlier.iter().map(|form| self.state.session.pending_scaling(form)).collect();
                let dialog = StatScalingDialog::new(name, &db, &staged, &data.enhance, model, earlier);
                self.push_dialog(Dialog::StatScaling(Box::new(dialog)));
                self.set_status(tr("Stat scaling loaded."));
            }
            Event::ScalingLoaded(Err(error)) => {
                self.set_status(tr("Stat scaling open failed (see details)."));
                self.push_dialog(Dialog::message(MessageDialog::error(
                    tr("Stat scaling"),
                    tr("Could not open the stat scaling editor."),
                    error,
                )));
            }

            Event::NextRandomGroupId { tree, uid, result } => match result {
                Ok(db_next) => {
                    let group_id = next_free_group_id(db_next, &self.state.session.reserved_group_ids());
                    let dialog = RandomGroupDialog::new(tree, uid, &RandomSkillGroup::empty(group_id), true);
                    self.push_dialog(Dialog::RandomGroup(Box::new(dialog)));
                }
                Err(error) => self.push_dialog(Dialog::message(MessageDialog::error(
                    tr("Random group"),
                    tr("Could not allocate a new random group ID."),
                    error,
                ))),
            },

            Event::SaveAnalyzed(result) => self.on_save_analyzed(result),
            Event::Committed(result) => self.on_committed(result),
        }
    }
}
