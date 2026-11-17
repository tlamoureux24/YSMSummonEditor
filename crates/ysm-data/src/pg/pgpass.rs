//! Password lookup in the PostgreSQL password file, the way libpq does it.
//!
//! When the password field of the settings is empty, the editor reads it
//! from `$PGPASSFILE`, else `~/.pgpass` (`%APPDATA%\postgresql\pgpass.conf`
//! on Windows), so it never has to be stored in `settings.json`.

use std::path::PathBuf;

/// The password file libpq would read.
pub fn default_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("PGPASSFILE") {
        return Some(PathBuf::from(path));
    }
    if cfg!(windows) {
        std::env::var_os("APPDATA").map(|appdata| PathBuf::from(appdata).join("postgresql").join("pgpass.conf"))
    } else {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".pgpass"))
    }
}

/// Password of the first matching line of the default password file.
pub fn lookup(host: &str, port: u16, database: &str, user: &str) -> Option<String> {
    let content = std::fs::read_to_string(default_path()?).ok()?;
    find(&content, host, port, database, user)
}

/// Password of the first line of `content` matching the connection.
///
/// Lines are `host:port:database:user:password`; `*` matches anything in the
/// first four fields, `\:` and `\\` escape a colon and a backslash.
pub fn find(content: &str, host: &str, port: u16, database: &str, user: &str) -> Option<String> {
    let port = port.to_string();
    let wanted = [host, port.as_str(), database, user];
    content.lines().find_map(|line| {
        if line.trim_start().starts_with('#') {
            return None;
        }
        let fields = split(line)?;
        let matches = fields.iter().zip(wanted).all(|(field, value)| field == "*" || field == value);
        matches.then(|| fields[4].clone())
    })
}

/// The five fields of a line, unescaped; `None` for a malformed line.
fn split(line: &str) -> Option<[String; 5]> {
    let mut fields: Vec<String> = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => fields.last_mut()?.push(chars.next().unwrap_or('\\')),
            // The password is the rest of the line, colons included.
            ':' if fields.len() < 5 => fields.push(String::new()),
            other => fields.last_mut()?.push(other),
        }
    }
    fields.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "\
# comment line
localhost:5432:arcadia:rappelz:first
*:5432:*:rappelz:second
h\\:x:5432:db:u:with\\\\back\\:slash
other:*:*:*:p:with:colons
";

    #[test]
    fn first_matching_line_wins() {
        assert_eq!(find(FILE, "localhost", 5432, "arcadia", "rappelz").as_deref(), Some("first"));
        assert_eq!(find(FILE, "127.0.0.1", 5432, "arcadia", "rappelz").as_deref(), Some("second"));
        assert_eq!(find(FILE, "localhost", 5433, "arcadia", "rappelz"), None);
    }

    #[test]
    fn escapes_and_colons_in_the_password_are_handled() {
        assert_eq!(find(FILE, "h:x", 5432, "db", "u").as_deref(), Some("with\\back:slash"));
        assert_eq!(find(FILE, "other", 1, "any", "one").as_deref(), Some("p:with:colons"));
    }

    #[test]
    fn malformed_lines_are_skipped() {
        assert_eq!(find("only:three:fields\n", "only", 0, "three", "fields"), None);
        assert_eq!(find("", "h", 5432, "d", "u"), None);
    }
}
