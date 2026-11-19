//! Everything the views read to draw a frame.

use std::time::Instant;

use ysm_core::model::{SummonListFilter, SummonListItem};
use ysm_core::session::{EditSession, TreeRef, TreeState};
use ysm_data::AppSettings;

use crate::worker::{Failure, FailureKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionStatus {
    NotConfigured,
    Connecting,
    /// Connected; carries a short server description.
    Connected(String),
    /// The initial connection could not be established.
    Failed(String),
    /// A connection that worked has since broken. The repository reconnects
    /// by itself on the next request, so requests are still allowed.
    Lost(String),
    /// Running on built-in sample data.
    Demo,
}

impl ConnectionStatus {
    /// Whether requests can be sent to the repository.
    pub fn is_ready(&self) -> bool {
        matches!(self, ConnectionStatus::Connected(_) | ConnectionStatus::Lost(_) | ConnectionStatus::Demo)
    }

    /// Whether offering a manual reconnect makes sense.
    pub fn can_reconnect(&self) -> bool {
        matches!(self, ConnectionStatus::Failed(_) | ConnectionStatus::Lost(_))
    }
}

pub struct AppState {
    pub settings: AppSettings,
    pub demo: bool,
    pub connection: ConnectionStatus,
    /// Description of the server last connected to, to restore the status after a reconnect.
    pub server_description: String,
    /// Number of indexed image files; `None` while the dump folder is being scanned.
    pub asset_count: Option<usize>,

    pub search_text: String,
    /// When the search box was last edited, for debounced live search.
    pub search_edited_at: Option<Instant>,
    /// Set to move keyboard focus into the search box on the next frame.
    pub focus_search: bool,
    pub include_riding_only: bool,
    pub summons: Vec<SummonListItem>,
    pub summons_loading: bool,
    /// The summon whose chain is loaded (or loading).
    pub selected_summon: Option<i32>,
    pub chain_loading: bool,

    pub session: EditSession,
    /// 1-based evolution stage shown in the tree panel.
    pub active_stage: u8,
    /// 1-based tree slot shown in the tree panel.
    pub active_slot: u8,
    pub selected_row: Option<u64>,
    /// Selected effective skill as `(skill_id, lv)`.
    pub selected_effective: Option<(i32, i32)>,

    pub status: String,
}

impl AppState {
    pub fn new(settings: AppSettings, demo: bool) -> Self {
        Self {
            settings,
            demo,
            connection: ConnectionStatus::NotConfigured,
            server_description: String::new(),
            asset_count: None,
            search_text: String::new(),
            search_edited_at: None,
            focus_search: false,
            include_riding_only: false,
            summons: Vec::new(),
            summons_loading: false,
            selected_summon: None,
            chain_loading: false,
            session: EditSession::new(),
            active_stage: 1,
            active_slot: 1,
            selected_row: None,
            selected_effective: None,
            status: String::new(),
        }
    }

    /// Records a successful (re)connection.
    pub fn connected(&mut self, server_description: String) {
        self.connection =
            if self.demo { ConnectionStatus::Demo } else { ConnectionStatus::Connected(server_description.clone()) };
        self.server_description = server_description;
    }

    /// Updates the connection status from the outcome of a repository
    /// request: a connection problem marks it lost, and the next request
    /// that gets through marks it connected again.
    pub fn note_request_outcome(&mut self, failure: Option<&Failure>) {
        match failure {
            Some(failure) if failure.kind.is_connection_problem() => {
                if matches!(self.connection, ConnectionStatus::Connected(_) | ConnectionStatus::Lost(_)) {
                    self.connection = ConnectionStatus::Lost(failure.message.clone());
                }
            }
            // An answer from the server, even a rejection, proves the connection works again.
            None | Some(Failure { kind: FailureKind::Request, .. }) => {
                if matches!(self.connection, ConnectionStatus::Lost(_)) {
                    self.connection = ConnectionStatus::Connected(self.server_description.clone());
                }
            }
            // Not having a repository, or a bug in the worker, says nothing about the connection.
            Some(_) => {}
        }
    }

    pub fn active_tree_ref(&self) -> TreeRef {
        TreeRef::new(self.active_stage, self.active_slot)
    }

    pub fn active_tree(&self) -> Option<&TreeState> {
        self.session.tree(self.active_tree_ref())
    }

    pub fn summon_filter(&self) -> SummonListFilter {
        SummonListFilter {
            search_text: self.search_text.trim().to_owned(),
            include_riding_only: self.include_riding_only,
            include_non_form1: true,
            show_all_evolutions: false,
        }
    }

    /// The stat editor works on the final evolution of the loaded chain.
    pub fn can_open_stats(&self) -> bool {
        self.connection.is_ready() && self.session.is_loaded()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidebar_always_requests_final_evolutions() {
        let settings = AppSettings { show_all_evolutions_by_default: true, ..Default::default() };
        let mut state = AppState::new(settings, false);
        assert!(!state.summon_filter().show_all_evolutions);

        state.search_text = "1011".into();
        assert!(!state.summon_filter().show_all_evolutions);
    }

    fn failure(kind: FailureKind) -> Failure {
        Failure { kind, message: "socket closed".into() }
    }

    fn connected_state() -> AppState {
        let mut state = AppState::new(AppSettings::default(), false);
        state.connected("SQL Server 2019".into());
        state
    }

    #[test]
    fn a_connection_problem_marks_the_connection_lost_and_the_next_answer_restores_it() {
        let mut state = connected_state();

        state.note_request_outcome(Some(&failure(FailureKind::Connection)));
        assert_eq!(state.connection, ConnectionStatus::Lost("socket closed".into()));
        assert!(state.connection.is_ready(), "the repository reconnects on the next request");
        assert!(state.connection.can_reconnect());

        state.note_request_outcome(None);
        assert_eq!(state.connection, ConnectionStatus::Connected("SQL Server 2019".into()));
        assert!(!state.connection.can_reconnect());
    }

    #[test]
    fn a_timeout_counts_as_a_lost_connection() {
        let mut state = connected_state();
        state.note_request_outcome(Some(&failure(FailureKind::Timeout)));
        assert!(matches!(state.connection, ConnectionStatus::Lost(_)));
    }

    #[test]
    fn a_rejected_request_does_not_touch_a_working_connection_and_proves_a_lost_one_is_back() {
        let mut state = connected_state();
        state.note_request_outcome(Some(&failure(FailureKind::Request)));
        assert!(matches!(state.connection, ConnectionStatus::Connected(_)));

        state.note_request_outcome(Some(&failure(FailureKind::Connection)));
        state.note_request_outcome(Some(&failure(FailureKind::Internal)));
        assert!(matches!(state.connection, ConnectionStatus::Lost(_)), "a worker bug proves nothing");
        state.note_request_outcome(Some(&failure(FailureKind::Request)));
        assert!(matches!(state.connection, ConnectionStatus::Connected(_)));
    }

    #[test]
    fn demo_and_unconnected_states_are_left_alone() {
        let mut demo = AppState::new(AppSettings::default(), true);
        demo.connected("In-memory demo data".into());
        demo.note_request_outcome(Some(&failure(FailureKind::Connection)));
        assert_eq!(demo.connection, ConnectionStatus::Demo);

        let mut failed = AppState::new(AppSettings::default(), false);
        failed.connection = ConnectionStatus::Failed("refused".into());
        failed.note_request_outcome(Some(&failure(FailureKind::NotConnected)));
        assert_eq!(failed.connection, ConnectionStatus::Failed("refused".into()));
    }
}
