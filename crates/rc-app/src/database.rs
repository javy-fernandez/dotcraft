//! Read-only SQLite workers and UI state. Adapted from CrabCraft's database_core.rs
//! scheduling, quoting, bounds and progress-hook patterns (MIT OR Apache-2.0).
//! Connections and SQL run only after explicit actions, always off the UI thread.

use fallible_iterator::FallibleIterator;
use rc_core::{Inbox, Waker};
use rusqlite::{Connection, OpenFlags, config::DbConfig, hooks::{AuthAction, Authorization}, limits::Limit, types::ValueRef};
use std::{path::{Path, PathBuf}, sync::{Arc, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};

pub const MAX_ROWS: usize = 500;
pub const MAX_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_SQL: usize = 64 * 1024;
pub const MAX_TABLES: usize = 200;
const TIME_LIMIT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    Path,
    Input,
    ReadOnly,
    Limit,
    Cancelled,
    TimedOut,
    Sql(String),
}

impl From<rusqlite::Error> for Error {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sql(error.to_string().chars().take(2048).collect())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl Cell {
    pub fn display(&self) -> String {
        match self {
            Self::Null => "NULL".into(),
            Self::Integer(value) => value.to_string(),
            Self::Real(value) => value.to_string(),
            Self::Text(value) => { let shown: String = value.chars().take(4096).collect(); if shown.len() < value.len() { format!("{shown}…") } else { shown } },
            Self::Blob(value) => format!("BLOB ({} bytes): {}{}", value.len(), value.iter().take(32).map(|b| format!("{b:02x}")).collect::<String>(), if value.len() > 32 { "…" } else { "" }),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Grid {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Cell>>,
    pub truncated: bool,
}

#[derive(Debug, Clone)]
pub struct Table {
    pub name: String,
    pub kind: String,
    pub sql: String,
}

#[derive(Debug, Clone)]
pub enum Action {
    Schema,
    Browse(String),
    Query(String),
}

#[derive(Debug)]
pub enum Reply {
    Schema { path: PathBuf, tables: Vec<Table>, truncated: bool },
    Grid(Grid),
}

fn check(cancel: &AtomicBool, deadline: Instant) -> Result<(), Error> {
    if cancel.load(Ordering::Relaxed) { Err(Error::Cancelled) }
    else if Instant::now() >= deadline { Err(Error::TimedOut) }
    else { Ok(()) }
}

fn ident(value: &str) -> Result<String, Error> {
    if value.is_empty() || value.len() > 1024 || value.contains('\0') { return Err(Error::Input); }
    Ok(format!("\"{}\"", value.replace('"', "\"\"")))
}

fn open(path: &Path, cancel: Arc<AtomicBool>, deadline: Instant) -> Result<Connection, Error> {
    check(&cancel, deadline)?;
    if path.as_os_str().len() > 8192 || !std::fs::metadata(path).is_ok_and(|m| m.is_file()) { return Err(Error::Path); }
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    connection.busy_timeout(Duration::from_millis(150))?;
    connection.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
    connection.set_db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false)?;
    connection.pragma_update(None, "temp_store", "MEMORY")?;
    connection.pragma_update(None, "cache_size", -4096)?;
    for (limit, value) in [
        (Limit::SQLITE_LIMIT_LENGTH, 1024 * 1024),
        (Limit::SQLITE_LIMIT_SQL_LENGTH, MAX_SQL as i32),
        (Limit::SQLITE_LIMIT_COLUMN, 256),
        (Limit::SQLITE_LIMIT_EXPR_DEPTH, 100),
        (Limit::SQLITE_LIMIT_COMPOUND_SELECT, 50),
        (Limit::SQLITE_LIMIT_VDBE_OP, 100_000),
        (Limit::SQLITE_LIMIT_ATTACHED, 0),
        (Limit::SQLITE_LIMIT_WORKER_THREADS, 0),
    ] { connection.set_limit(limit, value)?; }
    connection.progress_handler(1000, Some(move || cancel.load(Ordering::Relaxed) || Instant::now() >= deadline))?;
    connection.authorizer(Some(|context: rusqlite::hooks::AuthContext<'_>| match context.action {
        AuthAction::Select | AuthAction::Read { .. } | AuthAction::Recursive => Authorization::Allow,
        AuthAction::Function { function_name } if !function_name.eq_ignore_ascii_case("load_extension") => Authorization::Allow,
        _ => Authorization::Deny,
    }))?;
    Ok(connection)
}

fn query(connection: &Connection, sql: &str, cancel: &AtomicBool, deadline: Instant) -> Result<Grid, Error> {
    if sql.len() > MAX_SQL { return Err(Error::Limit); }
    check(cancel, deadline)?;
    let mut batch = rusqlite::Batch::new(connection, sql);
    let mut statement = batch.next()?.ok_or(Error::Input)?;
    if batch.next()?.is_some() { return Err(Error::Input); }
    if !statement.readonly() || statement.column_count() == 0 { return Err(Error::ReadOnly); }
    let count = statement.column_count();
    let mut bytes: usize = statement.column_names().iter().map(|name| name.len()).sum();
    if bytes > MAX_BYTES { return Err(Error::Limit); }
    let mut grid = Grid { columns: statement.column_names().iter().map(|name| name.to_string()).collect(), ..Grid::default() };
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        check(cancel, deadline)?;
        if grid.rows.len() >= MAX_ROWS { grid.truncated = true; break; }
        let mut cells = Vec::with_capacity(count);
        for index in 0..count {
            let value = row.get_ref(index)?;
            let size = match value { ValueRef::Text(value) | ValueRef::Blob(value) => value.len().max(16), _ => 16 };
            bytes = bytes.saturating_add(size);
            if bytes > MAX_BYTES { grid.truncated = true; return Ok(grid); }
            cells.push(match value {
                ValueRef::Null => Cell::Null,
                ValueRef::Integer(value) => Cell::Integer(value),
                ValueRef::Real(value) => Cell::Real(value),
                ValueRef::Text(value) => Cell::Text(std::str::from_utf8(value).map_err(|_| Error::Input)?.into()),
                ValueRef::Blob(value) => Cell::Blob(value.to_vec()),
            });
        }
        grid.rows.push(cells);
    }
    Ok(grid)
}

pub fn run(path: &Path, action: Action, cancel: Arc<AtomicBool>) -> Result<Reply, Error> {
    run_until(path, action, cancel, Instant::now() + TIME_LIMIT)
}

fn run_until(path: &Path, action: Action, cancel: Arc<AtomicBool>, deadline: Instant) -> Result<Reply, Error> {
    let result = (|| {
        let path = path.canonicalize().map_err(|_| Error::Path)?;
        let connection = open(&path, cancel.clone(), deadline)?;
        match action {
            Action::Schema => {
                let grid = query(&connection, "SELECT name, type, substr(sql,1,8192) FROM sqlite_schema WHERE type IN ('table','view') AND name NOT GLOB 'sqlite_*' ORDER BY name LIMIT 201", &cancel, deadline)?;
                let mut tables = Vec::new();
                for row in grid.rows.iter().take(MAX_TABLES) {
                    if let [Cell::Text(name), Cell::Text(kind), sql] = row.as_slice() {
                        ident(name)?;
                        tables.push(Table { name: name.clone(), kind: kind.clone(), sql: match sql { Cell::Text(value) => value.clone(), _ => String::new() } });
                    }
                }
                Ok(Reply::Schema { path, tables, truncated: grid.truncated || grid.rows.len() > MAX_TABLES })
            }
            Action::Browse(name) => {
                // Bind the name when verifying the table exists; quote it for SQL identifiers.
                let exists: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name=?1 AND type IN ('table','view'))", [&name], |row| row.get(0))?;
                if !exists { return Err(Error::Input); }
                Ok(Reply::Grid(query(&connection, &format!("SELECT * FROM {}", ident(&name)?), &cancel, deadline)?))
            }
            Action::Query(sql) => Ok(Reply::Grid(query(&connection, &sql, &cancel, deadline)?)),
        }
    })();
    match result {
        Err(_) if cancel.load(Ordering::Relaxed) => Err(Error::Cancelled),
        Err(_) if Instant::now() >= deadline => Err(Error::TimedOut),
        other => other,
    }
}

struct Work {
    cancel: Arc<AtomicBool>,
    inbox: Inbox<Result<Reply, Error>>,
}

pub struct Database {
    pub path: String,
    pub connected: Option<PathBuf>,
    pub tables: Vec<Table>,
    pub schema_truncated: bool,
    pub selected: usize,
    pub sql: String,
    pub grid: Option<Grid>,
    pub error: Option<Error>,
    work: Option<Work>,
    waker: Waker,
}

impl Database {
    pub fn new(waker: Waker) -> Self {
        Self { path: String::new(), connected: None, tables: Vec::new(), schema_truncated: false, selected: 0,
            sql: "SELECT sqlite_version();".into(), grid: None, error: None, work: None, waker }
    }

    pub fn busy(&self) -> bool { self.work.is_some() }

    pub fn connect(&mut self, root: Option<&Path>) {
        if self.busy() || self.connected.is_some() { return; }
        let path = PathBuf::from(self.path.trim());
        if path.as_os_str().is_empty() { self.error = Some(Error::Path); return; }
        let path = if path.is_absolute() { path } else if let Some(root) = root { root.join(path) } else { self.error = Some(Error::Path); return; };
        self.start(path, Action::Schema);
    }

    pub fn execute(&mut self, action: Action) {
        if self.busy() { return; }
        if let Some(path) = &self.connected { self.start(path.clone(), action); }
    }

    fn start(&mut self, path: PathBuf, action: Action) {
        self.error = None;
        self.grid = None;
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let inbox = Inbox::new(self.waker.clone());
        let out = inbox.sender();
        match std::thread::Builder::new().name("sqlite-query".into()).spawn(move || { out.send(run(&path, action, worker_cancel)); }) {
            Ok(_) => self.work = Some(Work { cancel, inbox }),
            Err(error) => self.error = Some(Error::Sql(error.to_string())),
        }
    }

    pub fn update(&mut self) {
        let reply = self.work.as_ref().and_then(|work| work.inbox.drain().into_iter().next());
        if let Some(reply) = reply {
            self.work = None;
            match reply {
                Ok(Reply::Schema { path, tables, truncated }) => {
                    self.connected = Some(path);
                    self.tables = tables;
                    self.selected = 0;
                    self.schema_truncated = truncated;
                }
                Ok(Reply::Grid(grid)) => self.grid = Some(grid),
                Err(error) => self.error = Some(error),
            }
        }
    }

    pub fn cancel(&mut self) {
        if let Some(work) = &self.work { work.cancel.store(true, Ordering::Relaxed); }
    }

    pub fn disconnect(&mut self) {
        self.cancel();
        // Dropping the inbox discards late results from the previous connection.
        self.work = None;
        self.connected = None;
        self.tables.clear();
        self.grid = None;
        self.error = None;
        self.schema_truncated = false;
    }
}

impl Drop for Database {
    fn drop(&mut self) { self.cancel(); }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.sqlite");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch("CREATE TABLE people(id INTEGER PRIMARY KEY, name TEXT, data BLOB); INSERT INTO people VALUES(1,'hello 🦀',X'00ff'),(2,NULL,NULL); CREATE VIEW names AS SELECT name FROM people;").unwrap();
        (dir, path)
    }

    fn call(path: &Path, action: Action) -> Result<Reply, Error> { run(path, action, Arc::new(AtomicBool::new(false))) }

    #[test]
    fn sqlite_reads_real_schema_rows_and_types_without_changing_file() {
        let (_dir, path) = fixture();
        let original = std::fs::read(&path).unwrap();
        let Reply::Schema { tables, .. } = call(&path, Action::Schema).unwrap() else { panic!() };
        assert_eq!(tables.iter().map(|table| table.name.as_str()).collect::<Vec<_>>(), ["names", "people"]);
        assert!(tables.iter().any(|table| table.sql.contains("CREATE TABLE")));
        let Reply::Grid(grid) = call(&path, Action::Browse("people".into())).unwrap() else { panic!() };
        assert_eq!(grid.columns, ["id", "name", "data"]);
        assert_eq!(grid.rows[0], [Cell::Integer(1), Cell::Text("hello 🦀".into()), Cell::Blob(vec![0,255])]);
        assert_eq!(grid.rows[1][1], Cell::Null);
        assert_eq!(std::fs::read(path).unwrap(), original);
    }

    #[test]
    fn sqlite_rejects_writes_multiple_statements_and_external_database_access() {
        let (_dir, path) = fixture();
        for sql in ["DELETE FROM people", "CREATE TABLE unsafe(x)", "PRAGMA user_version=7", "ATTACH ':memory:' AS other", "SELECT load_extension('missing')", "SELECT 1; SELECT 2", "BEGIN"] {
            assert!(call(&path, Action::Query(sql.into())).is_err(), "{sql}");
        }
        assert!(call(&path, Action::Browse("people\"; DROP TABLE people;--".into())).is_err());
        let Reply::Grid(grid) = call(&path, Action::Query("SELECT count(*) FROM people".into())).unwrap() else { panic!() };
        assert_eq!(grid.rows[0][0], Cell::Integer(2));
    }

    #[test]
    fn sqlite_bounds_rows_bytes_and_input_and_never_creates_missing_files() {
        let (dir, path) = fixture();
        let Reply::Grid(grid) = call(&path, Action::Query("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000) SELECT x FROM n".into())).unwrap() else { panic!() };
        assert_eq!(grid.rows.len(), MAX_ROWS);
        assert!(grid.truncated);
        let Reply::Grid(grid) = call(&path, Action::Query("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<10) SELECT randomblob(524288) FROM n".into())).unwrap() else { panic!() };
        assert!(grid.truncated && grid.rows.len() < 5);
        assert_eq!(call(&path, Action::Query(" ".repeat(MAX_SQL + 1))).unwrap_err(), Error::Limit);
        let missing = dir.path().join("missing.sqlite");
        assert_eq!(call(&missing, Action::Schema).unwrap_err(), Error::Path);
        assert!(!missing.exists());
    }

    #[test]
    fn sqlite_cancels_running_queries_and_honors_deadlines() {
        let (_dir, path) = fixture();
        let sql = "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000000000) SELECT sum(x) FROM n";
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let worker_path = path.clone();
        let worker = std::thread::spawn(move || run(&worker_path, Action::Query(sql.into()), worker_cancel));
        std::thread::sleep(Duration::from_millis(20));
        cancel.store(true, Ordering::Relaxed);
        assert_eq!(worker.join().unwrap().unwrap_err(), Error::Cancelled);
        assert_eq!(run_until(&path, Action::Query(sql.into()), Arc::new(AtomicBool::new(false)), Instant::now() + Duration::from_millis(5)).unwrap_err(), Error::TimedOut);
    }

    #[test]
    fn sqlite_disconnect_discards_late_worker_results() {
        let (_dir, path) = fixture();
        let mut database = Database::new(Waker::noop());
        database.path = path.display().to_string();
        database.connect(None);
        assert!(database.busy());
        database.disconnect();
        std::thread::sleep(Duration::from_millis(20));
        database.update();
        assert!(database.connected.is_none() && database.tables.is_empty() && database.grid.is_none());
    }
}
