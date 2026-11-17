//! MSSQL implementation of [`SummonRepository`] on top of tiberius.
//!
//! The repository owns a single-threaded tokio runtime and one lazily opened
//! connection, so its blocking methods can be called from a plain worker
//! thread. A connection that failed or sat idle is transparently reopened.

mod cells;
mod commit;
mod queries;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use tiberius::{AuthMethod, Client, Config, EncryptionLevel};
use tokio::net::TcpStream;
use tokio::runtime::Runtime;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};
use ysm_core::commit::{CommitRequest, CommitResult, tree_slot_column};
use ysm_core::model::{
    CreatureEnhance, LevelBonusUsage, RandomGroupUsage, SkillDisplay, SkillTreeEntry, SkillUsage, StatScalingSnapshot,
    StatUsage, SummonListFilter, SummonListItem, TreeUsage,
};
use ysm_core::random_group::RandomSkillGroup;
use ysm_core::value::SkillRecord;

use ysm_core::i18n::{Language, trf};

use crate::settings::DbSettings;
use crate::{DataError, Result, SummonRepository};

type SqlClient = Client<Compat<TcpStream>>;
type SqlResult<T> = std::result::Result<T, tiberius::error::Error>;

/// A connection unused for this long is checked before being trusted again.
const IDLE_CHECK_AFTER: Duration = Duration::from_secs(30);

/// How long the repository waits for the server before giving up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeouts {
    /// Opening the TCP connection, and separately the TDS login.
    pub connect: Duration,
    /// Each server round-trip of a query or statement.
    pub command: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self { connect: Duration::from_secs(10), command: Duration::from_secs(30) }
    }
}

pub struct MssqlRepository {
    runtime: Runtime,
    settings: DbSettings,
    timeouts: Timeouts,
    client: Option<SqlClient>,
    last_used: Instant,
    /// Language of the game texts (names, tooltips).
    language: Language,
    /// The table holding them, chosen once per connection.
    texts: Option<String>,
}

impl MssqlRepository {
    /// Creates the repository. No connection is opened until the first query.
    pub fn new(settings: DbSettings) -> Result<Self> {
        Self::with_timeouts(settings, Timeouts::default())
    }

    pub fn with_timeouts(settings: DbSettings, timeouts: Timeouts) -> Result<Self> {
        if !settings.is_configured() {
            return Err(DataError::NotConfigured);
        }
        let runtime = tokio::runtime::Builder::new_current_thread().enable_io().enable_time().build()?;
        Ok(Self {
            runtime,
            settings,
            timeouts,
            client: None,
            last_used: Instant::now(),
            language: Language::default(),
            texts: None,
        })
    }

    /// Runs one database operation, opening the connection first when needed.
    fn run<T>(&mut self, operation: impl AsyncFnOnce(&mut SqlClient, &str) -> SqlResult<T>) -> Result<T> {
        let Self { runtime, settings, timeouts, client, last_used, language, texts } = self;
        runtime.block_on(async {
            if let Some(existing) = client.as_mut()
                && last_used.elapsed() > IDLE_CHECK_AFTER
                && !is_alive(existing).await
            {
                *client = None;
            }

            let connection = match client {
                Some(connection) => connection,
                None => {
                    *texts = None;
                    client.insert(connect(settings, *timeouts).await?)
                }
            };
            let outcome = async {
                let table = match texts.as_ref() {
                    Some(table) => table.clone(),
                    None => texts.insert(text_table(connection, *language).await?).clone(),
                };
                operation(connection, &table).await
            }
            .await;
            *last_used = Instant::now();

            outcome.map_err(|error| {
                let failure = classify(&error, timeouts.command);
                // Errors reported by the server leave the connection usable. After anything
                // else it is out of sync with the server and must not be reused.
                if failure.is_connection_problem() {
                    *client = None;
                }
                failure
            })
        })
    }
}

/// `StringResource`, or `StringResource_EN` for English when the database has it.
async fn text_table(client: &mut SqlClient, language: Language) -> SqlResult<String> {
    let english = language == Language::English && {
        let sql = "SELECT CASE WHEN OBJECT_ID(N'StringResource_EN') IS NULL THEN 0 ELSE 1 END;";
        let row = client.simple_query(sql).await?.into_row().await?;
        row.is_some_and(|row| cells::Cells::new(row).i32() == 1)
    };
    Ok(if english { "StringResource_EN" } else { "StringResource" }.to_owned())
}

async fn is_alive(client: &mut SqlClient) -> bool {
    match client.simple_query("SELECT 1;").await {
        Ok(stream) => stream.into_results().await.is_ok(),
        Err(_) => false,
    }
}

/// Sorts a driver error into "the server said no" versus "the connection is gone".
fn classify(error: &tiberius::error::Error, command_timeout: Duration) -> DataError {
    use tiberius::error::Error;
    match error {
        Error::Server(_) => DataError::Query(describe(error)),
        Error::Io { kind: std::io::ErrorKind::TimedOut, .. } => {
            DataError::Timeout(trf("no response within {} s; the connection was closed", &[&command_timeout.as_secs()]))
        }
        _ => DataError::Connection(describe(error)),
    }
}

async fn connect(settings: &DbSettings, timeouts: Timeouts) -> Result<SqlClient> {
    let host = normalize_host(&settings.server);
    let mut config = Config::new();
    config.host(&host);
    config.port(settings.effective_port());
    config.database(settings.database.trim());
    config.authentication(AuthMethod::sql_server(settings.user_id.trim(), &settings.password));
    config.application_name("YSM Summon Editor");
    config.command_timeout(Some(timeouts.command));
    // Same as `Encrypt=False` in the original connection string: only the login is encrypted.
    config.encryption(EncryptionLevel::Off);
    if settings.trust_server_certificate {
        config.trust_cert();
    }

    let address = config.get_addr();
    let tcp = tokio::time::timeout(timeouts.connect, TcpStream::connect(&address))
        .await
        .map_err(|_| DataError::Connection(trf("timed out connecting to {}", &[&address])))?
        .map_err(|error| DataError::Connection(format!("{address}: {error}")))?;
    tcp.set_nodelay(true).map_err(|error| DataError::Connection(error.to_string()))?;

    tokio::time::timeout(timeouts.connect, Client::connect(config, tcp.compat_write()))
        .await
        .map_err(|_| DataError::Connection(trf("timed out logging in to {}", &[&address])))?
        .map_err(|error| DataError::Connection(describe(&error)))
}

/// Strips the parts of an ADO server name that only make sense to SqlClient.
///
/// The port is always given explicitly, so an instance name is redundant.
fn normalize_host(server: &str) -> String {
    let server = server.trim();
    let server = server.strip_prefix("tcp:").unwrap_or(server);
    let host = server.split(['\\', ',']).next().unwrap_or(server).trim();
    match host {
        "." | "(local)" | "" => "localhost".to_owned(),
        other => other.to_owned(),
    }
}

fn describe(error: &tiberius::error::Error) -> String {
    match error {
        tiberius::error::Error::Server(token) => {
            format!("{} (SQL error {}, line {})", token.message(), token.code(), token.line())
        }
        other => other.to_string(),
    }
}

/// Rejects requests that cannot be expressed in SQL before a transaction is opened.
fn validate(request: &CommitRequest) -> Result<()> {
    match request.trees.iter().find(|t| t.clone_if_shared && tree_slot_column(t.slot).is_none()) {
        Some(tree) => Err(DataError::Invalid(format!("Tree slot must be in range 1..5, got {}.", tree.slot))),
        None => Ok(()),
    }
}

impl SummonRepository for MssqlRepository {
    fn set_text_language(&mut self, language: Language) {
        if language != self.language {
            self.language = language;
            self.texts = None;
        }
    }

    fn test_connection(&mut self) -> Result<String> {
        self.run(async |c, _| queries::server_version(c).await)
    }

    fn summon_by_id(&mut self, summon_id: i32) -> Result<Option<SummonListItem>> {
        self.run(async |c, t| queries::summon_by_id(c, t, summon_id).await)
    }

    fn summons_by_card_id(&mut self, card_id: i32) -> Result<Vec<SummonListItem>> {
        self.run(async |c, t| queries::summons_by_card_id(c, t, card_id).await)
    }

    fn search_summons(&mut self, filter: &SummonListFilter) -> Result<Vec<SummonListItem>> {
        self.run(async |c, t| queries::search_summons(c, t, filter).await)
    }

    fn skill_tree_entries(&mut self, skill_tree_id: i32) -> Result<Vec<SkillTreeEntry>> {
        self.run(async |c, _| queries::skill_tree_entries(c, skill_tree_id).await)
    }

    fn stat_scaling(&mut self, summon_id: i32) -> Result<StatScalingSnapshot> {
        self.run(async |c, _| queries::stat_scaling(c, summon_id).await)?
            .ok_or_else(|| DataError::NotFound(format!("Summon {summon_id}")))
    }

    fn creature_enhance(&mut self) -> Result<Vec<CreatureEnhance>> {
        self.run(async |c, _| queries::creature_enhance(c).await)
    }

    fn skill_record(&mut self, skill_id: i32) -> Result<SkillRecord> {
        self.run(async |c, _| queries::skill_record(c, skill_id).await)?
            .ok_or_else(|| DataError::NotFound(format!("Skill {skill_id}")))
    }

    fn skill_display(&mut self, skill_ids: &[i32]) -> Result<HashMap<i32, SkillDisplay>> {
        if skill_ids.is_empty() {
            return Ok(HashMap::new());
        }
        self.run(async |c, t| queries::skill_display(c, t, skill_ids).await)
    }

    fn search_skills(&mut self, text: &str, limit: usize) -> Result<Vec<SkillDisplay>> {
        self.run(async |c, t| queries::search_skills(c, t, text, limit).await)
    }

    fn random_groups(&mut self, group_ids: &[i32]) -> Result<HashMap<i32, RandomSkillGroup>> {
        if group_ids.is_empty() {
            return Ok(HashMap::new());
        }
        self.run(async |c, _| queries::random_groups(c, group_ids).await)
    }

    fn next_random_group_id(&mut self) -> Result<i32> {
        self.run(async |c, _| queries::next_random_group_id(c).await)
    }

    fn skill_usage(&mut self, skill_ids: &[i32]) -> Result<Vec<SkillUsage>> {
        if skill_ids.is_empty() {
            return Ok(Vec::new());
        }
        self.run(async |c, t| queries::skill_usage(c, t, skill_ids).await)
    }

    fn tree_usage(&mut self, skill_tree_ids: &[i32]) -> Result<Vec<TreeUsage>> {
        if skill_tree_ids.is_empty() {
            return Ok(Vec::new());
        }
        self.run(async |c, t| queries::tree_usage(c, t, skill_tree_ids).await)
    }

    fn stat_usage(&mut self, stat_ids: &[i32]) -> Result<Vec<StatUsage>> {
        if stat_ids.is_empty() {
            return Ok(Vec::new());
        }
        self.run(async |c, t| queries::stat_usage(c, t, stat_ids).await)
    }

    fn level_bonus_usage(&mut self, bonus_ids: &[i32]) -> Result<Vec<LevelBonusUsage>> {
        if bonus_ids.is_empty() {
            return Ok(Vec::new());
        }
        self.run(async |c, t| queries::level_bonus_usage(c, t, bonus_ids).await)
    }

    fn random_group_usage(&mut self, group_ids: &[i32]) -> Result<Vec<RandomGroupUsage>> {
        if group_ids.is_empty() {
            return Ok(Vec::new());
        }
        self.run(async |c, t| queries::random_group_usage(c, t, group_ids).await)
    }

    fn commit(&mut self, request: &CommitRequest) -> Result<CommitResult> {
        validate(request)?;
        self.run(async |c, _| commit::commit(c, request).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ysm_core::commit::TreeCommit;

    #[test]
    fn host_is_normalized_for_a_plain_tcp_connection() {
        assert_eq!(normalize_host(" 10.0.0.5 "), "10.0.0.5");
        assert_eq!(normalize_host("tcp:db.example,1433"), "db.example");
        assert_eq!(normalize_host("HOST\\SQLEXPRESS"), "HOST");
        assert_eq!(normalize_host("."), "localhost");
        assert_eq!(normalize_host("(local)"), "localhost");
    }

    #[test]
    fn unconfigured_settings_are_rejected_without_connecting() {
        assert!(matches!(MssqlRepository::new(DbSettings::default()), Err(DataError::NotConfigured)));
    }

    #[test]
    fn clone_into_an_invalid_slot_is_rejected_up_front() {
        let tree = |slot, clone| TreeCommit {
            summon_id: 1,
            slot,
            original_tree_id: 1,
            entries: Vec::new(),
            clone_if_shared: clone,
        };
        assert!(validate(&CommitRequest { trees: vec![tree(5, true)], ..Default::default() }).is_ok());
        assert!(validate(&CommitRequest { trees: vec![tree(6, false)], ..Default::default() }).is_ok());
        assert!(validate(&CommitRequest { trees: vec![tree(6, true)], ..Default::default() }).is_err());
    }

    fn settings_for(port: u16) -> DbSettings {
        DbSettings {
            server: "127.0.0.1".into(),
            port: i32::from(port),
            database: "db".into(),
            user_id: "user".into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_server_that_never_answers_is_a_connection_problem_within_the_timeout() {
        // Accepts the TCP connection and then stays silent, like a hung server.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let timeouts = Timeouts { connect: Duration::from_millis(300), command: Duration::from_millis(300) };
        let mut repo = MssqlRepository::with_timeouts(settings_for(port), timeouts).unwrap();

        let started = Instant::now();
        let error = repo.test_connection().unwrap_err();

        assert!(error.is_connection_problem(), "{error}");
        assert!(error.to_string().contains("timed out logging in"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(5), "gave up after {:?}", started.elapsed());
        drop(listener);
    }

    #[test]
    fn a_closed_port_is_a_connection_problem() {
        // Bind and immediately release a port so nothing is listening on it.
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let mut repo = MssqlRepository::new(settings_for(port)).unwrap();

        let error = repo.search_summons(&SummonListFilter::default()).unwrap_err();
        assert!(matches!(error, DataError::Connection(_)), "{error}");
    }

    #[test]
    fn driver_errors_are_classified() {
        let timed_out = tiberius::error::Error::Io { kind: std::io::ErrorKind::TimedOut, message: "deadline".into() };
        let broken = tiberius::error::Error::Io { kind: std::io::ErrorKind::ConnectionReset, message: "reset".into() };
        let protocol = tiberius::error::Error::Protocol("bad token".into());
        let command = Duration::from_secs(30);

        assert!(matches!(classify(&timed_out, command), DataError::Timeout(message) if message.contains("30 s")));
        assert!(matches!(classify(&broken, command), DataError::Connection(_)));
        assert!(matches!(classify(&protocol, command), DataError::Connection(_)));
    }

    /// Smoke test against a real database. Run with
    /// `YSM_MSSQL_TEST_CONNECTION="Server=...;Database=...;User Id=...;Password=..." cargo test -- --ignored`.
    #[test]
    #[ignore = "needs a live MSSQL database"]
    fn live_database_smoke_test() {
        let connection_string =
            std::env::var("YSM_MSSQL_TEST_CONNECTION").expect("set YSM_MSSQL_TEST_CONNECTION to run this test");
        let settings = crate::env_local::parse_connection_string(&connection_string).expect("valid connection string");
        let mut repo = MssqlRepository::new(settings).unwrap();

        println!("server: {}", repo.test_connection().unwrap());
        let summons = repo.search_summons(&SummonListFilter::default()).unwrap();
        assert!(!summons.is_empty());

        let bundle = crate::loader::load_chain(&mut repo, &summons[0]).unwrap();
        println!("chain: {:?}", bundle.chain.summon_ids());
        assert!(repo.next_random_group_id().unwrap() >= 1);
        assert!(!repo.search_skills("", 5).unwrap().is_empty());
    }
}
