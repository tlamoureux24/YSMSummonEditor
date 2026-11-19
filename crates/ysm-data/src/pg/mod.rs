//! PostgreSQL implementation of [`SummonRepository`] on top of
//! `tokio-postgres`.
//!
//! Like the MSSQL repository it owns a single-threaded tokio runtime and one
//! lazily opened connection, reopened after a failure or a long idle time,
//! and bounds both the login and every statement by a timeout. Tables are
//! looked up through the database's `search_path`, so they may live in any
//! schema it names.

mod cells;
mod commit;
pub mod pgpass;
mod queries;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use tokio::runtime::Runtime;
use tokio_postgres::error::SqlState;
use tokio_postgres::{Client, NoTls};
use ysm_core::commit::{CommitRequest, CommitResult, tree_slot_column};
use ysm_core::model::{
    CreatureEnhance, LevelBonusUsage, RandomGroupUsage, SkillDisplay, SkillTreeEntry, SkillUsage, StatScalingSnapshot,
    StatUsage, SummonListFilter, SummonListItem, TreeUsage,
};
use ysm_core::random_group::RandomSkillGroup;
use ysm_core::value::SkillRecord;

pub use crate::mssql::Timeouts;
use crate::settings::DbSettings;
use crate::{DataError, Result, SummonRepository};

type PgResult<T> = std::result::Result<T, tokio_postgres::Error>;

/// A connection unused for this long is checked before being trusted again.
const IDLE_CHECK_AFTER: Duration = Duration::from_secs(30);

pub struct PgRepository {
    runtime: Runtime,
    settings: DbSettings,
    timeouts: Timeouts,
    client: Option<Client>,
    last_used: Instant,
}

impl PgRepository {
    /// Creates the repository. No connection is opened until the first query.
    pub fn new(settings: DbSettings) -> Result<Self> {
        Self::with_timeouts(settings, Timeouts::default())
    }

    pub fn with_timeouts(settings: DbSettings, timeouts: Timeouts) -> Result<Self> {
        if !settings.is_configured() {
            return Err(DataError::NotConfigured);
        }
        let runtime = tokio::runtime::Builder::new_current_thread().enable_io().enable_time().build()?;
        Ok(Self { runtime, settings, timeouts, client: None, last_used: Instant::now() })
    }

    /// Runs one database operation, opening the connection first when needed.
    fn run<T>(&mut self, operation: impl AsyncFnOnce(&mut Client) -> PgResult<T>) -> Result<T> {
        let Self { runtime, settings, timeouts, client, last_used } = self;
        runtime.block_on(async {
            if let Some(existing) = client.as_ref()
                && last_used.elapsed() > IDLE_CHECK_AFTER
                && !is_alive(existing, timeouts.connect).await
            {
                *client = None;
            }

            let connection = match client {
                Some(connection) => connection,
                None => client.insert(connect(settings, *timeouts).await?),
            };
            // The server cancels a long statement itself; this also covers a server that stops answering.
            let outcome = tokio::time::timeout(timeouts.command, operation(connection)).await;
            *last_used = Instant::now();

            let failure = match outcome {
                Ok(Ok(value)) => return Ok(value),
                Ok(Err(error)) => classify(&error, timeouts.command),
                Err(_) => DataError::Timeout(format!(
                    "no response within {} s; the connection was closed",
                    timeouts.command.as_secs()
                )),
            };
            // Errors reported by the server leave the connection usable. After anything
            // else it is out of sync with the server and must not be reused.
            if failure.is_connection_problem() {
                *client = None;
            }
            Err(failure)
        })
    }
}

async fn is_alive(client: &Client, timeout: Duration) -> bool {
    matches!(tokio::time::timeout(timeout, client.simple_query("SELECT 1")).await, Ok(Ok(_)))
}

/// Sorts a driver error into "the server said no" versus "the connection is gone".
fn classify(error: &tokio_postgres::Error, command_timeout: Duration) -> DataError {
    match error.as_db_error() {
        Some(db) if db.code() == &SqlState::QUERY_CANCELED => DataError::Timeout(format!(
            "no response within {} s; the statement was cancelled",
            command_timeout.as_secs()
        )),
        Some(db) => DataError::Query(format!("{} (SQLSTATE {})", db.message(), db.code().code())),
        None => DataError::Connection(error.to_string()),
    }
}

async fn connect(settings: &DbSettings, timeouts: Timeouts) -> Result<Client> {
    let host = settings.server.trim();
    let port = settings.effective_port();
    let database = settings.database.trim();
    let user = settings.user_id.trim();

    let mut config = tokio_postgres::Config::new();
    config
        .host(host)
        .port(port)
        .dbname(database)
        .user(user)
        .application_name("YSM Summon Editor")
        .connect_timeout(timeouts.connect)
        // The server cancels a statement that runs longer than the command timeout.
        .options(format!("-c statement_timeout={}", timeouts.command.as_millis()));

    if settings.password.is_empty() {
        if let Some(password) = pgpass::lookup(host, port, database, user) {
            config.password(password);
        }
    } else {
        config.password(&settings.password);
    }

    let target = format!("{host}:{port}/{database}");
    // `connect_timeout` only bounds the TCP connection; the login is bounded here.
    // ponytail: no TLS, the usual setup is a local server; add postgres-native-tls if a remote one needs it.
    let (client, connection) = tokio::time::timeout(timeouts.connect, config.connect(NoTls))
        .await
        .map_err(|_| DataError::Connection(format!("timed out logging in to {target}")))?
        .map_err(|error| match error.as_db_error() {
            Some(db) => DataError::Connection(format!("{target}: {} (SQLSTATE {})", db.message(), db.code().code())),
            None => DataError::Connection(format!("{target}: {error}")),
        })?;
    // Drives the socket; runs whenever the repository's runtime runs a query, ends with the client.
    tokio::spawn(async move {
        let _ = connection.await;
    });
    Ok(client)
}

/// Rejects requests that cannot be expressed in SQL before a transaction is opened.
fn validate(request: &CommitRequest) -> Result<()> {
    match request.trees.iter().find(|t| t.clone_if_shared && tree_slot_column(t.slot).is_none()) {
        Some(tree) => Err(DataError::Invalid(format!("Tree slot must be in range 1..5, got {}.", tree.slot))),
        None => Ok(()),
    }
}

impl SummonRepository for PgRepository {
    fn test_connection(&mut self) -> Result<String> {
        self.run(async |c| queries::server_version(c).await)
    }

    fn summon_by_id(&mut self, summon_id: i32) -> Result<Option<SummonListItem>> {
        self.run(async |c| queries::summon_by_id(c, summon_id).await)
    }

    fn summons_by_card_id(&mut self, card_id: i32) -> Result<Vec<SummonListItem>> {
        self.run(async |c| queries::summons_by_card_id(c, card_id).await)
    }

    fn search_summons(&mut self, filter: &SummonListFilter) -> Result<Vec<SummonListItem>> {
        self.run(async |c| queries::search_summons(c, filter).await)
    }

    fn skill_tree_entries(&mut self, skill_tree_id: i32) -> Result<Vec<SkillTreeEntry>> {
        self.run(async |c| queries::skill_tree_entries(c, skill_tree_id).await)
    }

    fn stat_scaling(&mut self, summon_id: i32) -> Result<StatScalingSnapshot> {
        self.run(async |c| queries::stat_scaling(c, summon_id).await)?
            .ok_or_else(|| DataError::NotFound(format!("Summon {summon_id}")))
    }

    fn creature_enhance(&mut self) -> Result<Vec<CreatureEnhance>> {
        self.run(async |c| queries::creature_enhance(c).await)
    }

    fn skill_record(&mut self, skill_id: i32) -> Result<SkillRecord> {
        self.run(async |c| queries::skill_record(c, skill_id).await)?
            .ok_or_else(|| DataError::NotFound(format!("Skill {skill_id}")))
    }

    fn skill_display(&mut self, skill_ids: &[i32]) -> Result<HashMap<i32, SkillDisplay>> {
        if skill_ids.is_empty() {
            return Ok(HashMap::new());
        }
        self.run(async |c| queries::skill_display(c, skill_ids).await)
    }

    fn search_skills(&mut self, text: &str, limit: usize) -> Result<Vec<SkillDisplay>> {
        self.run(async |c| queries::search_skills(c, text, limit).await)
    }

    fn random_groups(&mut self, group_ids: &[i32]) -> Result<HashMap<i32, RandomSkillGroup>> {
        if group_ids.is_empty() {
            return Ok(HashMap::new());
        }
        self.run(async |c| queries::random_groups(c, group_ids).await)
    }

    fn next_random_group_id(&mut self) -> Result<i32> {
        self.run(async |c| queries::next_random_group_id(c).await)
    }

    fn skill_usage(&mut self, skill_ids: &[i32]) -> Result<Vec<SkillUsage>> {
        if skill_ids.is_empty() {
            return Ok(Vec::new());
        }
        self.run(async |c| queries::skill_usage(c, skill_ids).await)
    }

    fn tree_usage(&mut self, skill_tree_ids: &[i32]) -> Result<Vec<TreeUsage>> {
        if skill_tree_ids.is_empty() {
            return Ok(Vec::new());
        }
        self.run(async |c| queries::tree_usage(c, skill_tree_ids).await)
    }

    fn stat_usage(&mut self, stat_ids: &[i32]) -> Result<Vec<StatUsage>> {
        if stat_ids.is_empty() {
            return Ok(Vec::new());
        }
        self.run(async |c| queries::stat_usage(c, stat_ids).await)
    }

    fn level_bonus_usage(&mut self, bonus_ids: &[i32]) -> Result<Vec<LevelBonusUsage>> {
        if bonus_ids.is_empty() {
            return Ok(Vec::new());
        }
        self.run(async |c| queries::level_bonus_usage(c, bonus_ids).await)
    }

    fn random_group_usage(&mut self, group_ids: &[i32]) -> Result<Vec<RandomGroupUsage>> {
        if group_ids.is_empty() {
            return Ok(Vec::new());
        }
        self.run(async |c| queries::random_group_usage(c, group_ids).await)
    }

    fn commit(&mut self, request: &CommitRequest) -> Result<CommitResult> {
        validate(request)?;
        self.run(async |c| commit::commit(c, request).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::DatabaseEngine;

    fn settings_for(port: u16) -> DbSettings {
        DbSettings {
            server: "127.0.0.1".into(),
            port: i32::from(port),
            database: "db".into(),
            user_id: "user".into(),
            password: "x".into(),
            ..DbSettings::for_engine(DatabaseEngine::PostgreSql)
        }
    }

    #[test]
    fn unconfigured_settings_are_rejected_without_connecting() {
        let empty = DbSettings::for_engine(DatabaseEngine::PostgreSql);
        assert!(matches!(PgRepository::new(empty), Err(DataError::NotConfigured)));
    }

    #[test]
    fn a_closed_port_is_a_connection_problem() {
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let mut repo = PgRepository::new(settings_for(port)).unwrap();
        let error = repo.search_summons(&SummonListFilter::default()).unwrap_err();
        assert!(matches!(error, DataError::Connection(_)), "{error}");
    }

    #[test]
    fn a_server_that_never_answers_gives_up_within_the_timeout() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let timeouts = Timeouts { connect: Duration::from_millis(300), command: Duration::from_millis(300) };
        let mut repo = PgRepository::with_timeouts(settings_for(port), timeouts).unwrap();

        let started = Instant::now();
        let error = repo.test_connection().unwrap_err();
        assert!(error.is_connection_problem(), "{error}");
        assert!(error.to_string().contains("timed out logging in"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(5), "gave up after {:?}", started.elapsed());
        drop(listener);
    }

    /// Reads against a real database. Run with
    /// `YSM_PG_TEST_CONNECTION="Host=...;Database=...;Username=..." cargo test -p ysm-data -- --ignored`;
    /// the password may come from the pgpass file. Nothing is written.
    #[test]
    #[ignore = "needs a live PostgreSQL database"]
    fn live_database_smoke_test() {
        let connection_string =
            std::env::var("YSM_PG_TEST_CONNECTION").expect("set YSM_PG_TEST_CONNECTION to run this test");
        let settings = crate::env_local::parse_connection_string_for(DatabaseEngine::PostgreSql, &connection_string)
            .expect("valid connection string");
        let mut repo = PgRepository::new(settings).unwrap();

        println!("server: {}", repo.test_connection().unwrap());
        let summons = repo.search_summons(&SummonListFilter::default()).unwrap();
        assert!(!summons.is_empty());

        let bundle = crate::loader::load_chain(&mut repo, &summons[0]).unwrap();
        println!("chain: {:?}", bundle.chain.summon_ids());
        assert!(repo.next_random_group_id().unwrap() >= 1);
        assert!(!repo.search_skills("", 5).unwrap().is_empty());
        assert!(!repo.creature_enhance().unwrap().is_empty());
    }
}
