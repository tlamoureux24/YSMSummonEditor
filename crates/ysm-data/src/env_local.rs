//! Bootstrap of database settings from a local `.env.local` file.
//!
//! The file holds one ADO-style connection string under
//! `YSM_DB_CONNECTION_STRING`, optionally the server kind under
//! `YSM_DB_PROVIDER` (`MSSQL`, the default, or `PostgreSQL`), and is looked
//! up from the working directory and the executable's directory upwards.

use std::path::{Path, PathBuf};

use crate::settings::{DbProvider, DbSettings};

const CONNECTION_STRING_KEY: &str = "YSM_DB_CONNECTION_STRING";
const PROVIDER_KEY: &str = "YSM_DB_PROVIDER";
const ENV_FILE_NAME: &str = ".env.local";

/// Database settings from the nearest `.env.local`, with the file they came from.
pub fn load_db_settings() -> Option<(DbSettings, PathBuf)> {
    let mut roots = Vec::new();
    roots.extend(std::env::current_dir().ok());
    roots.extend(std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf)));

    let path = find_env_local(&roots)?;
    let content = std::fs::read_to_string(&path).ok()?;
    let provider = read_value(&content, PROVIDER_KEY).and_then(|v| DbProvider::parse(&v)).unwrap_or_default();
    let settings = parse_connection_string_for(provider, &read_connection_string(&content)?)?;
    Some((settings, path))
}

/// First `.env.local` found walking up from each root.
pub fn find_env_local(roots: &[PathBuf]) -> Option<PathBuf> {
    roots.iter().flat_map(|root| root.ancestors()).map(|dir| dir.join(ENV_FILE_NAME)).find(|path| path.is_file())
}

/// Extracts the connection string from the contents of an env file.
pub fn read_connection_string(content: &str) -> Option<String> {
    read_value(content, CONNECTION_STRING_KEY)
}

/// Value of one `KEY=value` line of an env file.
fn read_value(content: &str, wanted: &str) -> Option<String> {
    content.lines().find_map(|raw_line| {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            return None;
        }
        let (key, value) = line.split_once('=')?;
        if key.trim() != wanted {
            return None;
        }
        let value = value.trim();
        let unquoted = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(value);
        Some(unquoted.to_owned())
    })
}

/// Parses an ADO-style MSSQL connection string. Returns `None` unless
/// server, database and user are all present.
pub fn parse_connection_string(connection_string: &str) -> Option<DbSettings> {
    parse_connection_string_for(DbProvider::Mssql, connection_string)
}

/// Parses a connection string for a provider. PostgreSQL strings in the
/// Npgsql style (`Host=…;Port=…;Username=…`) are accepted too.
pub fn parse_connection_string_for(provider: DbProvider, connection_string: &str) -> Option<DbSettings> {
    let pairs = split_pairs(connection_string);
    let get = |keys: &[&str]| {
        keys.iter().find_map(|key| {
            pairs.iter().find(|(k, v)| k.eq_ignore_ascii_case(key) && !v.is_empty()).map(|(_, v)| v.clone())
        })
    };

    let mut settings = DbSettings::for_provider(provider);
    if let Some(server) = get(&["Server", "Host", "Data Source", "Address", "Addr", "Network Address"]) {
        let server = server.strip_prefix("tcp:").unwrap_or(&server);
        match server.split_once(',') {
            Some((host, port)) => {
                settings.server = host.trim().to_owned();
                if let Ok(port) = port.trim().parse() {
                    settings.port = port;
                }
            }
            None => settings.server = server.trim().to_owned(),
        }
    }
    if let Some(port) = get(&["Port"]).and_then(|p| p.trim().parse().ok()) {
        settings.port = port;
    }
    if let Some(database) = get(&["Database", "Initial Catalog"]) {
        settings.database = database;
    }
    if let Some(user) = get(&["User Id", "UID", "User", "Username", "User Name"]) {
        settings.user_id = user;
    }
    if let Some(password) = get(&["Password", "Pwd"]) {
        settings.password = password;
    }
    if let Some(trust) = get(&["TrustServerCertificate", "Trust Server Certificate"]) {
        match trust.to_ascii_lowercase().as_str() {
            "true" | "yes" | "1" => settings.trust_server_certificate = true,
            "false" | "no" | "0" => settings.trust_server_certificate = false,
            _ => {}
        }
    }

    settings.is_configured().then_some(settings)
}

/// Splits `key=value;key='va;lue'` into pairs, honouring quoted values.
fn split_pairs(connection_string: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let mut rest = connection_string;
    while !rest.trim().is_empty() {
        let Some((key, after_key)) = rest.split_once('=') else {
            break;
        };
        let after_key = after_key.trim_start();
        let (value, remainder) = match after_key.chars().next() {
            Some(quote @ ('\'' | '"')) => match after_key[1..].split_once(quote) {
                Some((value, remainder)) => (value, remainder.trim_start().strip_prefix(';').unwrap_or(remainder)),
                None => (&after_key[1..], ""),
            },
            _ => after_key.split_once(';').unwrap_or((after_key, "")),
        };
        pairs.push((key.trim().trim_start_matches(';').trim().to_owned(), value.trim().to_owned()));
        rest = remainder;
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_template_connection_string() {
        let settings = parse_connection_string(
            "Server=localhost,1433;Database=ArcadiaDB;User Id=sa;Password=CHANGE_ME;TrustServerCertificate=True;Encrypt=False",
        )
        .unwrap();

        assert_eq!(settings.server, "localhost");
        assert_eq!(settings.port, 1433);
        assert_eq!(settings.database, "ArcadiaDB");
        assert_eq!(settings.user_id, "sa");
        assert_eq!(settings.password, "CHANGE_ME");
        assert!(settings.trust_server_certificate);
    }

    #[test]
    fn accepts_alternative_keys_and_keeps_the_default_port() {
        let settings = parse_connection_string(
            "data source=tcp:10.1.1.9; initial catalog=Game; uid=dev; pwd=x; trustservercertificate=false",
        )
        .unwrap();

        assert_eq!(settings.server, "10.1.1.9");
        assert_eq!(settings.port, 1433);
        assert_eq!(settings.database, "Game");
        assert_eq!(settings.user_id, "dev");
        assert_eq!(settings.password, "x");
        assert!(!settings.trust_server_certificate);
    }

    #[test]
    fn quoted_values_may_contain_separators() {
        let settings = parse_connection_string("Server=h,2000;Database=d;User Id=u;Password='a;b=c';").unwrap();
        assert_eq!(settings.password, "a;b=c");
        assert_eq!(settings.port, 2000);
    }

    #[test]
    fn postgres_strings_use_their_own_keys_and_default_port() {
        let settings =
            parse_connection_string_for(DbProvider::PostgreSql, "Host=127.0.0.1;Database=arcadia;Username=rz").unwrap();
        assert_eq!(settings.provider, DbProvider::PostgreSql);
        assert_eq!(settings.server, "127.0.0.1");
        assert_eq!(settings.port, 5432);
        assert_eq!(settings.user_id, "rz");
        assert!(settings.password.is_empty(), "the password may come from the pgpass file");

        let with_port =
            parse_connection_string_for(DbProvider::PostgreSql, "Host=h;Port=5433;Database=d;Username=u").unwrap();
        assert_eq!(with_port.port, 5433);
    }

    #[test]
    fn provider_is_read_from_the_env_file() {
        let content = "YSM_DB_PROVIDER=PostgreSQL\nYSM_DB_CONNECTION_STRING=Host=h;Database=d;Username=u\n";
        assert_eq!(read_value(content, PROVIDER_KEY).as_deref(), Some("PostgreSQL"));
        assert_eq!(read_connection_string(content).as_deref(), Some("Host=h;Database=d;Username=u"));
    }

    #[test]
    fn incomplete_connection_strings_are_rejected() {
        assert_eq!(parse_connection_string("Server=h;Database=d"), None);
        assert_eq!(parse_connection_string(""), None);
        assert_eq!(parse_connection_string("garbage"), None);
    }

    #[test]
    fn reads_the_key_from_env_file_content() {
        let content = "# comment\nOTHER=1\n\nYSM_DB_CONNECTION_STRING=\"Server=a;Database=b;User Id=c\"\n";
        assert_eq!(read_connection_string(content).as_deref(), Some("Server=a;Database=b;User Id=c"));
        assert_eq!(read_connection_string("# YSM_DB_CONNECTION_STRING=x\n"), None);
    }

    #[test]
    fn finds_the_file_in_a_parent_directory() {
        let root = std::env::temp_dir().join(format!("ysm-env-test-{}", std::process::id()));
        let nested = root.join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(root.join("a").join(ENV_FILE_NAME), "x").unwrap();

        assert_eq!(find_env_local(&[nested]), Some(root.join("a").join(ENV_FILE_NAME)));
        let _ = std::fs::remove_dir_all(&root);
    }
}
