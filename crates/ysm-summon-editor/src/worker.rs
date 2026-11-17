//! Background thread that owns the repository.
//!
//! The UI sends [`Command`]s and receives [`Event`]s; nothing on the UI
//! thread ever waits for the database or the file system.
//!
//! Every command is answered by exactly one event, even when handling it
//! panics, so the UI can never be left waiting. The exception is a request
//! that a newer one of the same kind has made pointless (a search typed
//! over, a summon clicked away from): those are dropped unanswered, and the
//! generation number they carry lets the UI ignore answers that still arrive.

use std::any::Any;
use std::cell::Cell;
use std::collections::VecDeque;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use eframe::egui;
use ysm_core::commit::{CommitRequest, CommitResult};
use ysm_core::i18n::Language;
use ysm_core::model::{CreatureEnhance, SkillDisplay, StatScalingSnapshot, SummonListFilter, SummonListItem};
use ysm_core::random_group::RandomSkillGroup;
use ysm_core::save_plan::UsageReport;
use ysm_core::session::{ChainBundle, TreeRef};
use ysm_core::value::SkillRecord;
use ysm_data::repository::SKILL_SEARCH_LIMIT;
use ysm_data::{
    AssetIndex, DataError, DbProvider, DbSettings, InMemoryRepository, MssqlRepository, PgRepository, SummonRepository,
    loader,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// No repository is configured.
    NotConnected,
    /// The connection could not be opened or broke while in use.
    Connection,
    /// The server stopped answering; the connection was closed.
    Timeout,
    /// The request itself failed; the connection is fine.
    Request,
    /// A bug: handling the command panicked.
    Internal,
}

impl FailureKind {
    /// Whether the connection is gone. For a write this means its outcome is unknown.
    pub fn is_connection_problem(self) -> bool {
        matches!(self, FailureKind::Connection | FailureKind::Timeout)
    }
}

/// Why a command failed, in a form the UI can both react to and show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub kind: FailureKind,
    pub message: String,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<Failure> for String {
    fn from(failure: Failure) -> Self {
        failure.message
    }
}

impl From<DataError> for Failure {
    fn from(error: DataError) -> Self {
        let kind = match &error {
            DataError::NotConfigured => FailureKind::NotConnected,
            DataError::Connection(_) => FailureKind::Connection,
            DataError::Timeout(_) => FailureKind::Timeout,
            _ => FailureKind::Request,
        };
        Self { kind, message: error.to_string() }
    }
}

pub type Outcome<T> = Result<T, Failure>;

pub enum Command {
    /// Switch to the database described by the settings and verify it; game
    /// texts are read in the given language.
    Connect(DbSettings, Language),
    /// Switch to the built-in demo data.
    UseDemo,
    /// Try a connection without touching the active repository.
    TestConnection(DbSettings),
    BuildAssetIndex(PathBuf),
    SearchSummons {
        filter: SummonListFilter,
        generation: u64,
    },
    LoadChain {
        summon: SummonListItem,
        generation: u64,
    },
    LoadSkillDisplay {
        ids: Vec<i32>,
    },
    LoadRandomGroups {
        ids: Vec<i32>,
    },
    LoadSkillRecord {
        skill_id: i32,
    },
    LoadScaling {
        summon_id: i32,
    },
    SearchSkills {
        text: String,
        generation: u64,
    },
    /// Ask for the next free random group ID, to create a group for a tree row.
    NextRandomGroupId {
        tree: TreeRef,
        uid: u64,
    },
    AnalyzeSave {
        request: CommitRequest,
        session_tree_ids: Vec<i32>,
    },
    Commit {
        request: CommitRequest,
    },
}

/// Kinds of request where only the newest one queued is worth running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Replaceable {
    SummonSearch,
    ChainLoad,
    SkillSearch,
}

impl Command {
    fn replaceable(&self) -> Option<Replaceable> {
        match self {
            Command::SearchSummons { .. } => Some(Replaceable::SummonSearch),
            Command::LoadChain { .. } => Some(Replaceable::ChainLoad),
            Command::SearchSkills { .. } => Some(Replaceable::SkillSearch),
            _ => None,
        }
    }

    /// The event that reports this command as failed.
    fn failed(&self, failure: Failure) -> Event {
        match self {
            Command::Connect(..) | Command::UseDemo => Event::Connected(Err(failure)),
            Command::TestConnection(_) => Event::ConnectionTested(Err(failure)),
            // The index has no failure channel; an empty one is the honest result.
            Command::BuildAssetIndex(_) => Event::AssetIndexBuilt(Arc::new(AssetIndex::empty())),
            Command::SearchSummons { generation, .. } => {
                Event::SummonsFound { generation: *generation, result: Err(failure) }
            }
            Command::LoadChain { summon, generation } => {
                Event::ChainLoaded { generation: *generation, summon: summon.clone(), result: Err(failure) }
            }
            Command::LoadSkillDisplay { .. } => Event::SkillDisplayLoaded(Err(failure)),
            Command::LoadRandomGroups { ids } => {
                Event::RandomGroupsLoaded { requested: ids.clone(), result: Err(failure) }
            }
            Command::LoadSkillRecord { skill_id } => {
                Event::SkillRecordLoaded { skill_id: *skill_id, result: Err(failure) }
            }
            Command::LoadScaling { .. } => Event::ScalingLoaded(Err(failure)),
            Command::SearchSkills { generation, .. } => {
                Event::SkillsFound { generation: *generation, result: Err(failure) }
            }
            Command::NextRandomGroupId { tree, uid } => {
                Event::NextRandomGroupId { tree: *tree, uid: *uid, result: Err(failure) }
            }
            Command::AnalyzeSave { .. } => Event::SaveAnalyzed(Err(failure)),
            Command::Commit { .. } => Event::Committed(Err(failure)),
        }
    }
}

pub enum Event {
    Connected(Outcome<String>),
    ConnectionTested(Outcome<String>),
    AssetIndexBuilt(Arc<AssetIndex>),
    SummonsFound { generation: u64, result: Outcome<Vec<SummonListItem>> },
    ChainLoaded { generation: u64, summon: SummonListItem, result: Outcome<Box<ChainBundle>> },
    SkillDisplayLoaded(Outcome<Vec<SkillDisplay>>),
    RandomGroupsLoaded { requested: Vec<i32>, result: Outcome<Vec<RandomSkillGroup>> },
    SkillRecordLoaded { skill_id: i32, result: Outcome<SkillRecord> },
    ScalingLoaded(Outcome<(StatScalingSnapshot, Vec<CreatureEnhance>)>),
    SkillsFound { generation: u64, result: Outcome<Vec<SkillDisplay>> },
    NextRandomGroupId { tree: TreeRef, uid: u64, result: Outcome<i32> },
    SaveAnalyzed(Outcome<UsageReport>),
    Committed(Outcome<CommitResult>),
}

impl Event {
    /// The failure this event reports, if any.
    pub fn failure(&self) -> Option<&Failure> {
        match self {
            Event::Connected(result) | Event::ConnectionTested(result) => result.as_ref().err(),
            Event::AssetIndexBuilt(_) => None,
            Event::SummonsFound { result, .. } => result.as_ref().err(),
            Event::ChainLoaded { result, .. } => result.as_ref().err(),
            Event::SkillDisplayLoaded(result) => result.as_ref().err(),
            Event::RandomGroupsLoaded { result, .. } => result.as_ref().err(),
            Event::SkillRecordLoaded { result, .. } => result.as_ref().err(),
            Event::ScalingLoaded(result) => result.as_ref().err(),
            Event::SkillsFound { result, .. } => result.as_ref().err(),
            Event::NextRandomGroupId { result, .. } => result.as_ref().err(),
            Event::SaveAnalyzed(result) => result.as_ref().err(),
            Event::Committed(result) => result.as_ref().err(),
        }
    }

    /// Whether the event is the answer to a request against the active
    /// repository, and so says something about the state of its connection.
    pub fn is_repository_answer(&self) -> bool {
        !matches!(self, Event::Connected(_) | Event::ConnectionTested(_) | Event::AssetIndexBuilt(_))
    }
}

pub struct WorkerHandle {
    commands: Sender<Command>,
    events: Receiver<Event>,
    alive: Cell<bool>,
}

impl WorkerHandle {
    /// Starts the worker thread. `ctx` is used to wake the UI when an event is ready.
    pub fn spawn(ctx: egui::Context) -> Self {
        let (command_tx, command_rx) = channel::<Command>();
        let (event_tx, event_rx) = channel::<Event>();

        let spawned = std::thread::Builder::new()
            .name("ysm-worker".into())
            .spawn(move || run(Worker::new(), command_rx, event_tx, ctx));

        Self { commands: command_tx, events: event_rx, alive: Cell::new(spawned.is_ok()) }
    }

    pub fn send(&self, command: Command) {
        if self.commands.send(command).is_err() {
            self.alive.set(false);
        }
    }

    pub fn try_recv(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }

    /// `false` once the worker thread is gone and commands can no longer be answered.
    pub fn is_alive(&self) -> bool {
        self.alive.get()
    }
}

fn run(mut worker: Worker, commands: Receiver<Command>, events: Sender<Event>, ctx: egui::Context) {
    let mut queue: VecDeque<Command> = VecDeque::new();
    loop {
        if queue.is_empty() {
            match commands.recv() {
                Ok(command) => queue.push_back(command),
                // The application is shutting down.
                Err(_) => return,
            }
        }
        queue.extend(commands.try_iter());

        let Some(command) = next_command(&mut queue) else {
            continue;
        };
        if events.send(worker.handle_guarded(&command)).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}

/// Takes the next command worth running, dropping requests that a newer one
/// of the same kind already waiting in the queue has superseded.
fn next_command(queue: &mut VecDeque<Command>) -> Option<Command> {
    while let Some(command) = queue.pop_front() {
        let superseded =
            command.replaceable().is_some_and(|kind| queue.iter().any(|later| later.replaceable() == Some(kind)));
        if !superseded {
            return Some(command);
        }
    }
    None
}

/// Where the active repository came from, so it can be rebuilt.
enum Source {
    None,
    Demo,
    Database(DbSettings, Language),
}

/// A repository for the configured server; no connection is opened yet.
fn open_repository(settings: &DbSettings, language: Language) -> ysm_data::Result<Box<dyn SummonRepository>> {
    let mut repository: Box<dyn SummonRepository> = match settings.provider {
        DbProvider::Mssql => Box::new(MssqlRepository::new(settings.clone())?),
        DbProvider::PostgreSql => Box::new(PgRepository::new(settings.clone())?),
    };
    repository.set_text_language(language);
    Ok(repository)
}

struct Worker {
    repository: Option<Box<dyn SummonRepository>>,
    source: Source,
}

impl Worker {
    fn new() -> Self {
        Self { repository: None, source: Source::None }
    }

    /// Runs an operation against the active repository.
    fn with_repository<T>(
        &mut self,
        operation: impl FnOnce(&mut dyn SummonRepository) -> ysm_data::Result<T>,
    ) -> Outcome<T> {
        let repository = self.repository.as_deref_mut().ok_or_else(|| Failure::from(DataError::NotConfigured))?;
        operation(repository).map_err(Failure::from)
    }

    /// Handles a command and always produces its event, even if the handler panics.
    fn handle_guarded(&mut self, command: &Command) -> Event {
        match catch_unwind(AssertUnwindSafe(|| self.handle(command))) {
            Ok(event) => event,
            Err(payload) => {
                self.recover_after_panic();
                command.failed(Failure {
                    kind: FailureKind::Internal,
                    message: format!(
                        "Internal error in the background worker: {}. \
                         The request was abandoned; if it was a save, reload to see what the database contains.",
                        panic_message(payload.as_ref())
                    ),
                })
            }
        }
    }

    /// A panic may have left a database connection half-way through a
    /// conversation, so the repository is rebuilt and reconnects on next use.
    fn recover_after_panic(&mut self) {
        if let Source::Database(settings, language) = &self.source {
            self.repository = open_repository(settings, *language).ok();
        }
    }

    fn handle(&mut self, command: &Command) -> Event {
        match command {
            Command::Connect(settings, language) => {
                self.repository = None;
                self.source = Source::Database(settings.clone(), *language);
                let result = open_repository(settings, *language).and_then(|mut repository| {
                    let version = repository.test_connection()?;
                    self.repository = Some(repository);
                    Ok(version)
                });
                Event::Connected(result.map_err(Failure::from))
            }
            Command::UseDemo => {
                let mut repository = InMemoryRepository::demo();
                let description = repository.test_connection().map_err(Failure::from);
                self.repository = Some(Box::new(repository));
                self.source = Source::Demo;
                Event::Connected(description)
            }
            Command::TestConnection(settings) => Event::ConnectionTested(
                open_repository(settings, Language::default())
                    .and_then(|mut repository| repository.test_connection())
                    .map_err(Failure::from),
            ),
            Command::BuildAssetIndex(path) => Event::AssetIndexBuilt(Arc::new(AssetIndex::build(path))),
            Command::SearchSummons { filter, generation } => Event::SummonsFound {
                generation: *generation,
                result: self.with_repository(|r| r.search_summons(filter)),
            },
            Command::LoadChain { summon, generation } => {
                let result = self.with_repository(|r| loader::load_chain(r, summon)).map(Box::new);
                Event::ChainLoaded { generation: *generation, summon: summon.clone(), result }
            }
            Command::LoadSkillDisplay { ids } => Event::SkillDisplayLoaded(
                self.with_repository(|r| r.skill_display(ids)).map(|found| found.into_values().collect()),
            ),
            Command::LoadRandomGroups { ids } => {
                let result = self.with_repository(|r| r.random_groups(ids)).map(|found| found.into_values().collect());
                Event::RandomGroupsLoaded { requested: ids.clone(), result }
            }
            Command::LoadSkillRecord { skill_id } => Event::SkillRecordLoaded {
                skill_id: *skill_id,
                result: self.with_repository(|r| r.skill_record(*skill_id)),
            },
            Command::LoadScaling { summon_id } => {
                Event::ScalingLoaded(self.with_repository(|r| Ok((r.stat_scaling(*summon_id)?, r.creature_enhance()?))))
            }
            Command::SearchSkills { text, generation } => Event::SkillsFound {
                generation: *generation,
                result: self.with_repository(|r| r.search_skills(text, SKILL_SEARCH_LIMIT)),
            },
            Command::NextRandomGroupId { tree, uid } => Event::NextRandomGroupId {
                tree: *tree,
                uid: *uid,
                result: self.with_repository(|r| r.next_random_group_id()),
            },
            Command::AnalyzeSave { request, session_tree_ids } => {
                Event::SaveAnalyzed(self.with_repository(|r| loader::analyze_save(r, request, session_tree_ids)))
            }
            Command::Commit { request } => Event::Committed(self.with_repository(|r| r.commit(request))),
        }
    }
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".to_owned())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use ysm_core::model::{LevelBonusUsage, RandomGroupUsage, SkillTreeEntry, SkillUsage, StatUsage, TreeUsage};

    use super::*;

    /// What every method of the stub repository does.
    #[derive(Clone, Copy)]
    enum Behaviour {
        Panic,
        ConnectionLost,
        TimedOut,
        Rejected,
    }

    struct StubRepository(Behaviour);

    impl StubRepository {
        fn respond<T>(&self) -> ysm_data::Result<T> {
            match self.0 {
                Behaviour::Panic => panic!("stub exploded"),
                Behaviour::ConnectionLost => Err(DataError::Connection("socket closed".into())),
                Behaviour::TimedOut => Err(DataError::Timeout("no response".into())),
                Behaviour::Rejected => Err(DataError::Query("syntax error".into())),
            }
        }
    }

    impl SummonRepository for StubRepository {
        fn test_connection(&mut self) -> ysm_data::Result<String> {
            self.respond()
        }
        fn summon_by_id(&mut self, _: i32) -> ysm_data::Result<Option<SummonListItem>> {
            self.respond()
        }
        fn summons_by_card_id(&mut self, _: i32) -> ysm_data::Result<Vec<SummonListItem>> {
            self.respond()
        }
        fn search_summons(&mut self, _: &SummonListFilter) -> ysm_data::Result<Vec<SummonListItem>> {
            self.respond()
        }
        fn skill_tree_entries(&mut self, _: i32) -> ysm_data::Result<Vec<SkillTreeEntry>> {
            self.respond()
        }
        fn stat_scaling(&mut self, _: i32) -> ysm_data::Result<StatScalingSnapshot> {
            self.respond()
        }
        fn creature_enhance(&mut self) -> ysm_data::Result<Vec<CreatureEnhance>> {
            self.respond()
        }
        fn skill_record(&mut self, _: i32) -> ysm_data::Result<SkillRecord> {
            self.respond()
        }
        fn skill_display(&mut self, _: &[i32]) -> ysm_data::Result<HashMap<i32, SkillDisplay>> {
            self.respond()
        }
        fn search_skills(&mut self, _: &str, _: usize) -> ysm_data::Result<Vec<SkillDisplay>> {
            self.respond()
        }
        fn random_groups(&mut self, _: &[i32]) -> ysm_data::Result<HashMap<i32, RandomSkillGroup>> {
            self.respond()
        }
        fn next_random_group_id(&mut self) -> ysm_data::Result<i32> {
            self.respond()
        }
        fn skill_usage(&mut self, _: &[i32]) -> ysm_data::Result<Vec<SkillUsage>> {
            self.respond()
        }
        fn tree_usage(&mut self, _: &[i32]) -> ysm_data::Result<Vec<TreeUsage>> {
            self.respond()
        }
        fn stat_usage(&mut self, _: &[i32]) -> ysm_data::Result<Vec<StatUsage>> {
            self.respond()
        }
        fn level_bonus_usage(&mut self, _: &[i32]) -> ysm_data::Result<Vec<LevelBonusUsage>> {
            self.respond()
        }
        fn random_group_usage(&mut self, _: &[i32]) -> ysm_data::Result<Vec<RandomGroupUsage>> {
            self.respond()
        }
        fn commit(&mut self, _: &CommitRequest) -> ysm_data::Result<CommitResult> {
            self.respond()
        }
    }

    fn worker_with(behaviour: Behaviour) -> Worker {
        Worker { repository: Some(Box::new(StubRepository(behaviour))), source: Source::None }
    }

    fn search(generation: u64) -> Command {
        Command::SearchSummons { filter: SummonListFilter::default(), generation }
    }

    fn commit() -> Command {
        Command::Commit { request: CommitRequest::default() }
    }

    fn kind_of(event: &Event) -> Option<FailureKind> {
        event.failure().map(|failure| failure.kind)
    }

    #[test]
    fn a_panicking_handler_still_answers_and_the_worker_keeps_working() {
        let mut worker = worker_with(Behaviour::Panic);

        let event = worker.handle_guarded(&commit());
        let failure = event.failure().expect("the panic is reported as a failure");
        assert_eq!(failure.kind, FailureKind::Internal);
        assert!(failure.message.contains("stub exploded"), "{}", failure.message);
        assert!(matches!(event, Event::Committed(Err(_))), "the answer matches the command");

        // The same worker handles the next command normally.
        let event = worker.handle_guarded(&Command::UseDemo);
        assert!(matches!(event, Event::Connected(Ok(_))));
        let event = worker.handle_guarded(&search(1));
        assert!(matches!(event, Event::SummonsFound { generation: 1, result: Ok(_) }));
    }

    #[test]
    fn failures_carry_their_kind_to_the_ui() {
        assert_eq!(
            kind_of(&worker_with(Behaviour::ConnectionLost).handle_guarded(&search(1))),
            Some(FailureKind::Connection)
        );
        assert_eq!(kind_of(&worker_with(Behaviour::TimedOut).handle_guarded(&commit())), Some(FailureKind::Timeout));
        assert_eq!(kind_of(&worker_with(Behaviour::Rejected).handle_guarded(&commit())), Some(FailureKind::Request));

        let mut unconfigured = Worker::new();
        assert_eq!(kind_of(&unconfigured.handle_guarded(&search(1))), Some(FailureKind::NotConnected));

        assert!(FailureKind::Connection.is_connection_problem());
        assert!(FailureKind::Timeout.is_connection_problem());
        assert!(!FailureKind::Request.is_connection_problem());
        assert!(!FailureKind::Internal.is_connection_problem());
    }

    #[test]
    fn every_command_has_a_matching_failure_event() {
        let failure = || Failure { kind: FailureKind::Internal, message: "x".into() };
        let tree = TreeRef::new(1, 1);
        let commands = [
            Command::Connect(DbSettings::default(), Language::default()),
            Command::UseDemo,
            Command::TestConnection(DbSettings::default()),
            search(4),
            Command::LoadSkillDisplay { ids: vec![1] },
            Command::LoadRandomGroups { ids: vec![2] },
            Command::LoadSkillRecord { skill_id: 3 },
            Command::LoadScaling { summon_id: 4 },
            Command::SearchSkills { text: String::new(), generation: 5 },
            Command::NextRandomGroupId { tree, uid: 6 },
            Command::AnalyzeSave { request: CommitRequest::default(), session_tree_ids: vec![] },
            commit(),
        ];
        for command in &commands {
            assert!(command.failed(failure()).failure().is_some());
        }

        assert!(matches!(search(4).failed(failure()), Event::SummonsFound { generation: 4, .. }));
        assert!(matches!(
            Command::NextRandomGroupId { tree, uid: 6 }.failed(failure()),
            Event::NextRandomGroupId { uid: 6, .. }
        ));
    }

    #[test]
    fn only_the_newest_queued_search_is_run() {
        let mut queue: VecDeque<Command> = VecDeque::from([
            search(1),
            Command::SearchSkills { text: "a".into(), generation: 10 },
            search(2),
            commit(),
            search(3),
            Command::SearchSkills { text: "ab".into(), generation: 11 },
        ]);

        let mut ran = Vec::new();
        while let Some(command) = next_command(&mut queue) {
            ran.push(match command {
                Command::SearchSummons { generation, .. } => format!("summons {generation}"),
                Command::SearchSkills { generation, .. } => format!("skills {generation}"),
                Command::Commit { .. } => "commit".to_owned(),
                _ => "other".to_owned(),
            });
        }
        assert_eq!(ran, ["commit", "summons 3", "skills 11"], "writes are never dropped");
    }

    #[test]
    fn events_know_whether_they_speak_for_the_repository() {
        let lost = || Failure { kind: FailureKind::Connection, message: "x".into() };
        assert!(Event::SummonsFound { generation: 1, result: Err(lost()) }.is_repository_answer());
        assert!(Event::Committed(Ok(CommitResult::default())).is_repository_answer());
        assert!(!Event::ConnectionTested(Err(lost())).is_repository_answer());
        assert!(!Event::Connected(Err(lost())).is_repository_answer());
    }
}
