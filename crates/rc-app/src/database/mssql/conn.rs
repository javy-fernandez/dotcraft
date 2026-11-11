//! ADO.NET connection strings (`Server=…;Database=…`) and the `ConnectionStrings` sections of
//! a workspace's `appsettings*.json` files.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Auth {
    #[default]
    Sql,
    /// Windows integrated security (SSPI); only available on Windows.
    Integrated,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConnSpec {
    pub host: String,
    pub port: Option<u16>,
    pub instance: Option<String>,
    pub database: String,
    pub auth: Auth,
    pub user: String,
    pub password: String,
    /// `Encrypt=false` (the login packet is still encrypted).
    pub no_encrypt: bool,
    pub trust_cert: bool,
}

/// Why a connection string cannot be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// `(localdb)\…`: LocalDB only listens on a named pipe.
    LocalDb,
    /// `np:`/`lpc:` prefixes or `Network Library=dbnmpntw`.
    NamedPipes,
    /// `Authentication=Active Directory…` and other unsupported keywords.
    Unsupported(String),
    MissingServer,
    BadPort(String),
    Syntax,
}

impl ConnSpec {
    /// `host`, `host,port` or `host\instance`, as typed in the Server field.
    pub fn server(&self) -> String {
        let mut s = self.host.clone();
        if let Some(instance) = &self.instance { s.push('\\'); s.push_str(instance); }
        if let Some(port) = self.port { s.push_str(&format!(",{port}")); }
        s
    }

    /// The keychain account for this login: the same password serves every database.
    pub fn account(&self) -> String {
        format!("{}@{}", self.user, self.server().to_lowercase())
    }

    /// Fills host, port and instance from a Server value (`tcp:host,1433`, `host\SQLEXPRESS`).
    pub fn set_server(&mut self, value: &str) -> Result<(), ParseError> {
        let value = value.trim();
        let lower = value.to_ascii_lowercase();
        if lower.starts_with("(localdb)") { return Err(ParseError::LocalDb); }
        if lower.starts_with("np:") || lower.starts_with("lpc:") || lower.starts_with(r"\\") { return Err(ParseError::NamedPipes); }
        let value = if lower.starts_with("tcp:") { &value[4..] } else { value };
        let (rest, port) = match value.rsplit_once(',') {
            Some((rest, port)) => (rest.trim(), Some(port.trim().parse::<u16>().map_err(|_| ParseError::BadPort(port.trim().into()))?)),
            None => (value, None),
        };
        let (host, instance) = match rest.split_once('\\') {
            Some((host, instance)) if !instance.trim().is_empty() => (host.trim(), Some(instance.trim().to_string())),
            Some((host, _)) => (host.trim(), None),
            None => (rest, None),
        };
        if host.is_empty() { return Err(ParseError::MissingServer); }
        self.host = match host.to_ascii_lowercase().as_str() { "." | "(local)" | "localhost" => "localhost".into(), _ => host.into() };
        self.port = port;
        self.instance = instance;
        Ok(())
    }
}

/// Splits `key=value;…` pairs. Values may be quoted with `'` or `"` (doubled to escape);
/// keys are lowercased with inner whitespace collapsed.
pub fn pairs(s: &str) -> Result<Vec<(String, String)>, ParseError> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| c.is_whitespace() || *c == ';') { chars.next(); }
        if chars.peek().is_none() { break; }
        let mut key = String::new();
        for c in chars.by_ref() {
            if c == '=' { break; }
            key.push(c);
        }
        let key = key.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_lowercase();
        if key.is_empty() { return Err(ParseError::Syntax); }
        while chars.peek().is_some_and(|c| *c == ' ' || *c == '\t') { chars.next(); }
        let mut value = String::new();
        match chars.peek().copied() {
            Some(quote @ ('\'' | '"')) => {
                chars.next();
                loop {
                    match chars.next() {
                        Some(c) if c == quote => {
                            if chars.peek() == Some(&quote) { chars.next(); value.push(quote); } else { break; }
                        }
                        Some(c) => value.push(c),
                        None => return Err(ParseError::Syntax),
                    }
                }
                while chars.peek().is_some_and(|c| *c != ';') {
                    if !chars.next().is_some_and(char::is_whitespace) { return Err(ParseError::Syntax); }
                }
            }
            _ => {
                while let Some(&c) = chars.peek() {
                    if c == ';' { break; }
                    value.push(c);
                    chars.next();
                }
                value = value.trim().to_string();
            }
        }
        out.push((key, value));
    }
    Ok(out)
}

fn truthy(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "sspi" | "mandatory" | "strict" => Some(true),
        "false" | "no" | "optional" => Some(false),
        _ => None,
    }
}

/// Parses an ADO.NET / Microsoft.Data.SqlClient connection string.
pub fn parse(s: &str) -> Result<ConnSpec, ParseError> {
    let mut spec = ConnSpec::default();
    let mut server = None;
    for (key, value) in pairs(s)? {
        match key.as_str() {
            "server" | "data source" | "address" | "addr" | "network address" => server = Some(value),
            "database" | "initial catalog" => spec.database = value,
            "user id" | "uid" | "user" | "username" | "user name" => spec.user = value,
            "password" | "pwd" => spec.password = value,
            "integrated security" | "trusted_connection" | "trusted connection" => {
                if truthy(&value) == Some(true) { spec.auth = Auth::Integrated; }
            }
            "encrypt" => spec.no_encrypt = truthy(&value) == Some(false),
            "trustservercertificate" | "trust server certificate" => spec.trust_cert = truthy(&value) == Some(true),
            "attachdbfilename" | "attach db filename" | "extended properties" | "initial file name" => return Err(ParseError::LocalDb),
            "network library" | "network" | "net" => {
                let lower = value.to_ascii_lowercase();
                if lower == "dbnmpntw" || lower == "dbmslpcn" { return Err(ParseError::NamedPipes); }
            }
            "authentication" => {
                let lower = value.to_ascii_lowercase().replace(' ', "");
                match lower.as_str() {
                    "sqlpassword" => spec.auth = Auth::Sql,
                    "activedirectoryintegrated" => spec.auth = Auth::Integrated,
                    _ => return Err(ParseError::Unsupported(format!("Authentication={value}"))),
                }
            }
            _ => {}
        }
    }
    spec.set_server(server.as_deref().ok_or(ParseError::MissingServer)?)?;
    Ok(spec)
}

/// What a connection string in appsettings points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    SqlServer(Result<ConnSpec, ParseError>),
    /// `Data Source=app.db` (opened with the SQLite browser).
    Sqlite(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub file: PathBuf,
    pub name: String,
    pub kind: Kind,
}

/// Classifies one connection string; PostgreSQL, MySQL and others return `None`.
pub fn classify(value: &str) -> Option<Kind> {
    let pairs = pairs(value).ok()?;
    let keys: Vec<&str> = pairs.iter().map(|(k, _)| k.as_str()).collect();
    // PostgreSQL (Host=) and MySQL (Port=) strings; SQL Server puts the port in Server.
    if keys.iter().any(|k| matches!(*k, "host" | "port" | "sslmode" | "ssl mode")) { return None; }
    let source = pairs.iter().find(|(k, _)| matches!(k.as_str(), "data source" | "datasource" | "filename")).map(|(_, v)| v.clone());
    if let Some(source) = &source {
        let lower = source.to_ascii_lowercase();
        let sqlite_ext = [".db", ".sqlite", ".sqlite3", ".db3"].iter().any(|ext| lower.ends_with(ext)) || lower == ":memory:";
        if sqlite_ext && !keys.iter().any(|k| matches!(*k, "database" | "initial catalog" | "user id" | "integrated security")) {
            return Some(Kind::Sqlite(source.clone()));
        }
    }
    if !keys.iter().any(|k| matches!(*k, "server" | "data source" | "address" | "addr" | "network address")) { return None; }
    Some(Kind::SqlServer(parse(value)))
}

/// Strips `//` and `/* */` comments and trailing commas (appsettings allow both).
pub fn relaxed_json(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if c == '\\' { if let Some(n) = chars.next() { out.push(n); } } else if c == '"' { in_string = false; }
            continue;
        }
        match c {
            '"' => { in_string = true; out.push(c); }
            '/' if chars.peek() == Some(&'/') => { for c in chars.by_ref() { if c == '\n' { out.push('\n'); break; } } }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut last = ' ';
                for c in chars.by_ref() { if last == '*' && c == '/' { break; } last = c; }
            }
            ',' => {
                let rest: String = chars.clone().take_while(|c| c.is_whitespace()).collect();
                let next = chars.clone().nth(rest.chars().count());
                if !matches!(next, Some('}') | Some(']')) { out.push(','); }
            }
            _ => out.push(c),
        }
    }
    out
}

/// The `ConnectionStrings` entries of one appsettings file (key matched case-insensitively),
/// sorted by name.
pub fn connection_strings(text: &str) -> Vec<(String, String)> {
    let Ok(serde_json::Value::Object(root)) = serde_json::from_str::<serde_json::Value>(&relaxed_json(text)) else { return Vec::new() };
    let Some(serde_json::Value::Object(section)) = root.iter().find(|(k, _)| k.eq_ignore_ascii_case("ConnectionStrings")).map(|(_, v)| v) else { return Vec::new() };
    section.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string()))).collect()
}

const SKIP: &[&str] = &["bin", "obj", "node_modules", ".git", ".vs", ".idea", "packages", "target", "TestResults", "wwwroot"];

/// Every SQL Server / SQLite connection string in `appsettings*.json` under `root`
/// (build output and dependency folders skipped; at most 6 levels and 200 files).
pub fn scan(root: &Path) -> Vec<Found> {
    let mut files = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                if depth < 6 && !SKIP.iter().any(|s| s.eq_ignore_ascii_case(&name)) && !name.starts_with('.') { stack.push((entry.path(), depth + 1)); }
            } else if kind.is_file() {
                let lower = name.to_ascii_lowercase();
                if lower.starts_with("appsettings") && lower.ends_with(".json") && files.len() < 200 { files.push(entry.path()); }
            }
        }
    }
    files.sort();
    let mut out = Vec::new();
    for file in files {
        let Ok(text) = std::fs::read_to_string(&file) else { continue };
        for (name, value) in connection_strings(&text) {
            if let Some(kind) = classify(&value) {
                out.push(Found { file: file.strip_prefix(root).map(Path::to_path_buf).unwrap_or(file.clone()), name, kind });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_sql_server_connection_strings() {
        let spec = parse("Server=tcp:db.example.com,1433;Initial Catalog=Shop;Persist Security Info=False;User ID=reader;Password='p;a''ss';MultipleActiveResultSets=False;Encrypt=True;TrustServerCertificate=False;Connection Timeout=30;").unwrap();
        assert_eq!((spec.host.as_str(), spec.port, spec.database.as_str(), spec.user.as_str(), spec.password.as_str()), ("db.example.com", Some(1433), "Shop", "reader", "p;a'ss"));
        assert!(!spec.no_encrypt && !spec.trust_cert && spec.auth == Auth::Sql);
        assert_eq!(spec.server(), "db.example.com,1433");
        assert_eq!(spec.account(), "reader@db.example.com,1433");

        let spec = parse("Data Source=.\\SQLEXPRESS;Database=Orders;Integrated Security=SSPI;TrustServerCertificate=yes;Encrypt=false").unwrap();
        assert_eq!((spec.host.as_str(), spec.instance.as_deref(), spec.auth), ("localhost", Some("SQLEXPRESS"), Auth::Integrated));
        assert!(spec.no_encrypt && spec.trust_cert);

        let spec = parse("server = \"sql01\" ; database=App; uid=sa; pwd=\"x\"\"y\"; Trusted_Connection=False").unwrap();
        assert_eq!((spec.host.as_str(), spec.user.as_str(), spec.password.as_str(), spec.auth), ("sql01", "sa", "x\"y", Auth::Sql));
    }

    #[test]
    fn rejects_localdb_named_pipes_and_unsupported_auth() {
        assert_eq!(parse(r"Server=(localdb)\mssqllocaldb;Database=Shop;Trusted_Connection=True").unwrap_err(), ParseError::LocalDb);
        assert_eq!(parse(r"Data Source=np:\\.\pipe\sql\query;Database=x").unwrap_err(), ParseError::NamedPipes);
        assert_eq!(parse("Server=myhost;Network Library=dbnmpntw").unwrap_err(), ParseError::NamedPipes);
        assert_eq!(parse(r"Server=.\SQLEXPRESS;AttachDbFilename=|DataDirectory|\app.mdf").unwrap_err(), ParseError::LocalDb);
        assert!(matches!(parse("Server=x.database.windows.net;Authentication=Active Directory Default").unwrap_err(), ParseError::Unsupported(_)));
        assert_eq!(parse("Server=host,abc").unwrap_err(), ParseError::BadPort("abc".into()));
        assert_eq!(parse("Database=x").unwrap_err(), ParseError::MissingServer);
        assert_eq!(parse("Server='unterminated").unwrap_err(), ParseError::Syntax);
    }

    #[test]
    fn classifies_sqlite_and_skips_other_providers() {
        assert_eq!(classify("Data Source=app.db"), Some(Kind::Sqlite("app.db".into())));
        assert_eq!(classify("Host=localhost;Port=5432;Database=x;Username=u"), None);
        assert!(matches!(classify("Server=localhost;Database=x;User Id=sa;Password=p"), Some(Kind::SqlServer(Ok(_)))));
        assert!(matches!(classify(r"Server=(localdb)\mssqllocaldb;Database=x"), Some(Kind::SqlServer(Err(ParseError::LocalDb)))));
        assert_eq!(classify("redis:6379"), None);
    }

    #[test]
    fn reads_connection_strings_from_appsettings_with_comments() {
        let text = r#"{
            // comment
            "Logging": { "LogLevel": { "Default": "Information" } },
            /* block */
            "connectionStrings": {
                "Default": "Server=localhost;Database=Shop;User Id=reader;Password=secret;",
                "Cache": "Data Source=cache.db",
                "Url": "https://x//y",
            },
        }"#;
        assert_eq!(connection_strings(text), vec![
            ("Cache".to_string(), "Data Source=cache.db".to_string()),
            ("Default".to_string(), "Server=localhost;Database=Shop;User Id=reader;Password=secret;".to_string()),
            ("Url".to_string(), "https://x//y".to_string()),
        ]);
    }

    #[test]
    fn scans_workspace_appsettings_and_skips_build_output() {
        let dir = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        std::fs::create_dir_all(root.join("src/Api/bin/Debug")).unwrap();
        std::fs::write(root.join("src/Api/appsettings.json"), r#"{"ConnectionStrings":{"Shop":"Server=sql;Database=Shop;User Id=u"}}"#).unwrap();
        std::fs::write(root.join("src/Api/appsettings.Development.json"), r#"{"ConnectionStrings":{"Local":"Server=(localdb)\\mssqllocaldb;Database=Shop","Pg":"Host=x;Database=y"}}"#).unwrap();
        std::fs::write(root.join("src/Api/bin/Debug/appsettings.json"), r#"{"ConnectionStrings":{"Copy":"Server=sql;Database=Shop"}}"#).unwrap();
        let found = scan(&root);
        let names: Vec<_> = found.iter().map(|f| (f.file.to_string_lossy().replace('\\', "/"), f.name.clone())).collect();
        assert_eq!(names, vec![("src/Api/appsettings.Development.json".to_string(), "Local".to_string()), ("src/Api/appsettings.json".to_string(), "Shop".to_string())]);
        assert!(matches!(found[0].kind, Kind::SqlServer(Err(ParseError::LocalDb))));
    }
}
