//! User settings, stored as JSON in the same file and shape the original
//! .NET editor used, so an existing configuration keeps working.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Result;

const APP_FOLDER: &str = "YSMSummonEditor";
const SETTINGS_FILE: &str = "settings.json";
/// Default MSSQL port.
pub const DEFAULT_PORT: i32 = 1433;
/// Default PostgreSQL port.
pub const DEFAULT_POSTGRES_PORT: i32 = 5432;

/// The database server the editor talks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DbProvider {
    #[default]
    #[serde(rename = "MSSQL")]
    Mssql,
    #[serde(rename = "PostgreSQL")]
    PostgreSql,
}

impl DbProvider {
    pub const ALL: [DbProvider; 2] = [DbProvider::Mssql, DbProvider::PostgreSql];

    pub fn default_port(self) -> i32 {
        match self {
            DbProvider::Mssql => DEFAULT_PORT,
            DbProvider::PostgreSql => DEFAULT_POSTGRES_PORT,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            DbProvider::Mssql => "MSSQL",
            DbProvider::PostgreSql => "PostgreSQL",
        }
    }

    /// Parses the value of `YSM_DB_PROVIDER`.
    pub fn parse(text: &str) -> Option<DbProvider> {
        match text.trim().to_ascii_lowercase().as_str() {
            "mssql" | "sqlserver" | "sql server" => Some(DbProvider::Mssql),
            "postgresql" | "postgres" | "pgsql" | "pg" => Some(DbProvider::PostgreSql),
            _ => None,
        }
    }

    /// Flavour of the `.sql` export.
    pub fn sql_dialect(self) -> ysm_core::sql_script::SqlDialect {
        match self {
            DbProvider::Mssql => ysm_core::sql_script::SqlDialect::TSql,
            DbProvider::PostgreSql => ysm_core::sql_script::SqlDialect::PostgreSql,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct DbSettings {
    /// Missing in files written before PostgreSQL support: MSSQL.
    pub provider: DbProvider,
    pub server: String,
    pub port: i32,
    pub database: String,
    pub user_id: String,
    pub password: String,
    pub trust_server_certificate: bool,
}

impl Default for DbSettings {
    fn default() -> Self {
        Self {
            provider: DbProvider::Mssql,
            server: String::new(),
            port: DEFAULT_PORT,
            database: String::new(),
            user_id: String::new(),
            password: String::new(),
            trust_server_certificate: true,
        }
    }
}

impl DbSettings {
    pub fn is_configured(&self) -> bool {
        !self.server.trim().is_empty() && !self.database.trim().is_empty() && !self.user_id.trim().is_empty()
    }

    /// Empty settings for a provider, with its default port.
    pub fn for_provider(provider: DbProvider) -> Self {
        Self { provider, port: provider.default_port(), ..Self::default() }
    }

    /// The configured port, or the provider's default when it is out of range.
    pub fn effective_port(&self) -> u16 {
        u16::try_from(self.port).ok().filter(|p| *p > 0).unwrap_or(self.provider.default_port() as u16)
    }

    /// `server,port / database`, without credentials.
    pub fn display_target(&self) -> String {
        format!("{},{} / {}", self.server, self.effective_port(), self.database)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct AppSettings {
    pub database: DbSettings,
    pub dump_folder_path: String,
    pub include_riding_only_by_default: bool,
    pub include_non_form1_by_default: bool,
    pub show_all_evolutions_by_default: bool,
    pub save_to_sql_mode: bool,
}

pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// `%LOCALAPPDATA%\YSMSummonEditor\settings.json`.
    pub fn at_default_location() -> Self {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("share")))
            .unwrap_or_else(std::env::temp_dir);
        Self::new(base.join(APP_FOLDER).join(SETTINGS_FILE))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads the settings; a missing file yields the defaults.
    pub fn load(&self) -> Result<AppSettings> {
        let json = match std::fs::read_to_string(&self.path) {
            Ok(json) => json,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(AppSettings::default()),
            Err(error) => return Err(error.into()),
        };
        // A UTF-8 byte order mark is not valid JSON but .NET tools sometimes write one.
        let mut settings: AppSettings = serde_json::from_str(json.trim_start_matches('\u{feff}'))
            .map_err(|error| crate::DataError::Invalid(format!("settings file is not valid: {error}")))?;
        if settings.database.port <= 0 {
            settings.database.port = settings.database.provider.default_port();
        }
        Ok(settings)
    }

    pub fn save(&self, settings: &AppSettings) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(settings)
            .map_err(|error| crate::DataError::Invalid(format!("could not serialize settings: {error}")))?;
        std::fs::write(&self.path, json)?;
        Ok(())
    }
}

/// `%USERPROFILE%\Desktop\Dump`.
pub fn default_dump_folder() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    Some(PathBuf::from(home).join("Desktop").join("Dump"))
}

/// Points the dump folder at the default location when none is set,
/// creating the folder. Returns whether the setting was changed.
pub fn ensure_default_dump_folder(settings: &mut AppSettings) -> io::Result<bool> {
    if !settings.dump_folder_path.trim().is_empty() {
        return Ok(false);
    }
    let Some(folder) = default_dump_folder() else {
        return Ok(false);
    };
    std::fs::create_dir_all(&folder)?;
    settings.dump_folder_path = folder.to_string_lossy().into_owned();
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ysm-settings-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("nested").join("settings.json")
    }

    #[test]
    fn reads_a_file_written_by_the_dotnet_editor() {
        let json = r#"{
  "Database": {
    "Server": "10.0.0.5",
    "Port": 14330,
    "Database": "Arcadia",
    "UserId": "sa",
    "Password": "p;w=d",
    "TrustServerCertificate": false
  },
  "DumpFolderPath": "C:\\Dump",
  "IncludeRidingOnlyByDefault": false,
  "IncludeNonForm1ByDefault": true,
  "ShowAllEvolutionsByDefault": false,
  "SaveToSqlMode": true
}"#;
        let path = temp_file("dotnet");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, format!("\u{feff}{json}")).unwrap();

        let settings = SettingsStore::new(&path).load().unwrap();
        assert_eq!(settings.database.server, "10.0.0.5");
        assert_eq!(settings.database.effective_port(), 14330);
        assert_eq!(settings.database.password, "p;w=d");
        assert!(!settings.database.trust_server_certificate);
        assert_eq!(settings.dump_folder_path, "C:\\Dump");
        assert!(settings.include_non_form1_by_default);
        assert!(settings.save_to_sql_mode);
        assert!(settings.database.is_configured());
        assert_eq!(settings.database.provider, DbProvider::Mssql, "files without Provider stay on MSSQL");
    }

    #[test]
    fn postgres_provider_round_trips_with_its_default_port() {
        let path = temp_file("postgres");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{ "Database": { "Provider": "PostgreSQL", "Server": "h", "Port": 0 } }"#).unwrap();
        let store = SettingsStore::new(&path);
        let settings = store.load().unwrap();
        assert_eq!(settings.database.provider, DbProvider::PostgreSql);
        assert_eq!(settings.database.port, DEFAULT_POSTGRES_PORT);

        store.save(&settings).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("\"Provider\": \"PostgreSQL\""));
        assert_eq!(store.load().unwrap(), settings);
        assert_eq!(DbSettings { port: -1, ..DbSettings::for_provider(DbProvider::PostgreSql) }.effective_port(), 5432);
    }

    #[test]
    fn provider_names_are_parsed_loosely() {
        assert_eq!(DbProvider::parse(" PostgreSQL "), Some(DbProvider::PostgreSql));
        assert_eq!(DbProvider::parse("postgres"), Some(DbProvider::PostgreSql));
        assert_eq!(DbProvider::parse("MSSQL"), Some(DbProvider::Mssql));
        assert_eq!(DbProvider::parse("oracle"), None);
    }

    #[test]
    fn missing_file_and_missing_keys_fall_back_to_defaults() {
        let path = temp_file("missing");
        let store = SettingsStore::new(&path);
        assert_eq!(store.load().unwrap(), AppSettings::default());

        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{ "Database": { "Server": "x", "Port": 0 } }"#).unwrap();
        let settings = store.load().unwrap();
        assert_eq!(settings.database.server, "x");
        assert_eq!(settings.database.port, DEFAULT_PORT);
        assert!(settings.database.trust_server_certificate);
        assert!(!settings.database.is_configured());
    }

    #[test]
    fn save_writes_pascal_case_keys_and_round_trips() {
        let path = temp_file("roundtrip");
        let store = SettingsStore::new(&path);
        let mut settings =
            AppSettings { dump_folder_path: "D:\\Assets".into(), save_to_sql_mode: true, ..Default::default() };
        settings.database.user_id = "editor".into();

        store.save(&settings).unwrap();
        let json = std::fs::read_to_string(&path).unwrap();
        for key in
            ["\"Database\"", "\"UserId\"", "\"TrustServerCertificate\"", "\"DumpFolderPath\"", "\"SaveToSqlMode\""]
        {
            assert!(json.contains(key), "missing {key} in {json}");
        }
        assert_eq!(store.load().unwrap(), settings);
    }

    #[test]
    fn invalid_json_is_an_error_not_a_silent_reset() {
        let path = temp_file("invalid");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ not json").unwrap();
        assert!(SettingsStore::new(&path).load().is_err());
    }

    #[test]
    fn out_of_range_port_uses_the_default() {
        let settings = DbSettings { port: 99_999, ..Default::default() };
        assert_eq!(settings.effective_port(), 1433);
    }
}
