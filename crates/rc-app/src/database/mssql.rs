//! Read-only SQL Server connections through tiberius (TDS over TCP, TLS with rustls).
//! One worker thread per connection owns a small tokio runtime and the client; the UI sends it
//! jobs and drains replies once per frame. Every query runs inside a transaction that is rolled
//! back, after the read-only guard accepted it.

pub mod conn;
mod driver;
pub mod guard;
pub mod plan;
pub mod schema;
pub mod secrets;

use crate::database::Grid;
use conn::{Auth, ConnSpec, Found, ParseError};
use guard::GuardError;
use rc_core::{Inbox, Waker};
use std::{collections::HashSet, path::{Path, PathBuf}, sync::{Arc, atomic::{AtomicBool, Ordering}, mpsc}, time::{Duration, Instant}};

pub const MAX_TABLES: usize = 1000;
const CONNECT_LIMIT: Duration = Duration::from_secs(20);
const QUERY_LIMIT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ServerInfo {
    pub version: String,
    pub edition: String,
    pub database: String,
    pub login: String,
    /// The login is db_owner, db_datawriter, db_ddladmin or sysadmin.
    pub can_write: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MsError {
    Parse(ParseError),
    Guard(GuardError),
    IntegratedUnavailable,
    NoPassword,
    Connect(String),
    Tls(String),
    Server(String),
    Cancelled,
    TimedOut,
}

#[derive(Debug, Clone)]
pub enum Request {
    Schema,
    Query(String),
    /// Schema and table name.
    Browse(String, String),
    Plan(String),
}

enum Job {
    Connect { spec: ConnSpec, remember: bool },
    Run(Request),
}

#[derive(Debug)]
pub enum Reply {
    Connected { info: ServerInfo, schema: schema::Schema, keychain: Option<String> },
    Schema(schema::Schema),
    Grid(Grid),
    Plan(plan::Plan),
}

#[derive(Debug, Clone)]
pub enum Output {
    Grid(Grid),
    Plan(plan::Plan),
}

/// The connect form.
#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    pub server: String,
    pub database: String,
    pub auth: Auth,
    pub user: String,
    pub password: String,
    pub trust_cert: bool,
    pub no_encrypt: bool,
    pub remember: bool,
}

impl Default for Form {
    fn default() -> Self {
        Form { server: String::new(), database: String::new(), auth: Auth::Sql, user: String::new(), password: String::new(), trust_cert: false, no_encrypt: false, remember: true }
    }
}

impl Form {
    pub fn fill(&mut self, spec: &ConnSpec) {
        self.server = spec.server();
        self.database = spec.database.clone();
        self.auth = spec.auth;
        self.user = spec.user.clone();
        self.password = spec.password.clone();
        self.trust_cert = spec.trust_cert;
        self.no_encrypt = spec.no_encrypt;
    }

    pub fn spec(&self) -> Result<ConnSpec, MsError> {
        let mut spec = ConnSpec { database: self.database.trim().into(), auth: self.auth, user: self.user.trim().into(), password: self.password.clone(), trust_cert: self.trust_cert, no_encrypt: self.no_encrypt, ..ConnSpec::default() };
        spec.set_server(&self.server).map_err(MsError::Parse)?;
        if spec.auth == Auth::Integrated && !cfg!(windows) { return Err(MsError::IntegratedUnavailable); }
        Ok(spec)
    }
}

struct Worker {
    jobs: mpsc::Sender<(Job, Arc<AtomicBool>)>,
    inbox: Inbox<Result<Reply, MsError>>,
    running: Option<Arc<AtomicBool>>,
}

pub struct SqlServer {
    pub form: Form,
    pub found: Vec<Found>,
    scanned: Option<PathBuf>,
    scan: Option<Inbox<Vec<Found>>>,
    pub info: Option<ServerInfo>,
    /// Server and database shown in the header.
    pub title: String,
    pub schema: schema::Schema,
    pub expanded: HashSet<String>,
    /// Index into `schema.tables`.
    pub selected: Option<usize>,
    pub filter: String,
    pub sql: String,
    pub output: Option<Output>,
    pub error: Option<MsError>,
    /// A keychain failure after a successful connection.
    pub keychain_error: Option<String>,
    pub connecting: bool,
    worker: Option<Worker>,
    waker: Waker,
    secrets: Arc<dyn secrets::Secrets>,
}

impl SqlServer {
    pub fn new(waker: Waker) -> Self { Self::with_secrets(waker, Arc::new(secrets::Keychain)) }

    pub fn with_secrets(waker: Waker, secrets: Arc<dyn secrets::Secrets>) -> Self {
        SqlServer {
            form: Form::default(), found: Vec::new(), scanned: None, scan: None, info: None, title: String::new(), schema: schema::Schema::default(),
            expanded: HashSet::new(), selected: None, filter: String::new(), sql: "SELECT TOP (100) * FROM INFORMATION_SCHEMA.TABLES;".into(),
            output: None, error: None, keychain_error: None, connecting: false, worker: None, waker, secrets,
        }
    }

    pub fn connected(&self) -> bool { self.info.is_some() }
    pub fn busy(&self) -> bool { self.worker.as_ref().is_some_and(|w| w.running.is_some()) }

    /// Looks for connection strings in the workspace's appsettings files (once per root).
    pub fn scan(&mut self, root: Option<&Path>) {
        let Some(root) = root else { return };
        if self.scanned.as_deref() == Some(root) { return; }
        self.scanned = Some(root.to_path_buf());
        let inbox = Inbox::new(self.waker.clone());
        let out = inbox.sender();
        let root = root.to_path_buf();
        if std::thread::Builder::new().name("appsettings-scan".into()).spawn(move || { out.send(conn::scan(&root)); }).is_ok() { self.scan = Some(inbox); }
    }

    /// The appsettings scan is still running.
    #[cfg(test)]
    pub fn scanning(&self) -> bool { self.scan.is_some() }

    /// For tests and captures: results as if scanned.
    #[cfg(test)]
    pub fn set_found(&mut self, found: Vec<Found>) { self.found = found; self.scan = None; }

    pub fn connect(&mut self) {
        if self.busy() || self.connected() { return; }
        self.error = None;
        let spec = match self.form.spec() { Ok(spec) => spec, Err(e) => { self.error = Some(e); return } };
        self.disconnect();
        let (jobs, rx) = mpsc::channel();
        let inbox = Inbox::new(self.waker.clone());
        let out = inbox.sender();
        let secrets = self.secrets.clone();
        if let Err(e) = std::thread::Builder::new().name("sqlserver".into()).spawn(move || worker(rx, out, secrets)) {
            self.error = Some(MsError::Connect(e.to_string()));
            return;
        }
        self.title = format!("{} · {}", spec.server(), if spec.database.is_empty() { "master" } else { &spec.database });
        self.worker = Some(Worker { jobs, inbox, running: None });
        self.connecting = true;
        self.send(Job::Connect { spec, remember: self.form.remember && !self.form.password.is_empty() });
    }

    fn send(&mut self, job: Job) {
        let Some(worker) = &mut self.worker else { return };
        let cancel = Arc::new(AtomicBool::new(false));
        if worker.jobs.send((job, cancel.clone())).is_ok() { worker.running = Some(cancel); }
    }

    pub fn execute(&mut self, request: Request) {
        if self.busy() || !self.connected() { return; }
        if let Request::Query(sql) | Request::Plan(sql) = &request
            && let Err(e) = guard::check(sql)
        {
            self.error = Some(MsError::Guard(e));
            return;
        }
        self.error = None;
        if !matches!(request, Request::Schema) { self.output = None; }
        self.send(Job::Run(request));
    }

    pub fn cancel(&mut self) {
        if let Some(cancel) = self.worker.as_ref().and_then(|w| w.running.as_ref()) { cancel.store(true, Ordering::Relaxed); }
    }

    /// Removes the saved password of the login in the form.
    pub fn forget_password(&mut self) {
        let Ok(spec) = self.form.spec() else { return };
        let secrets = self.secrets.clone();
        let _ = std::thread::Builder::new().name("keychain".into()).spawn(move || { let _ = secrets.delete(&spec.account()); });
    }

    pub fn update(&mut self) {
        if let Some(found) = self.scan.as_ref().and_then(|s| s.drain().into_iter().next()) { self.found = found; self.scan = None; }
        let Some(reply) = self.worker.as_ref().and_then(|w| w.inbox.drain().into_iter().next()) else { return };
        if let Some(w) = &mut self.worker { w.running = None; }
        match reply {
            Ok(Reply::Connected { info, schema, keychain }) => {
                self.connecting = false;
                self.form.password.clear();
                self.info = Some(info);
                self.schema = schema;
                self.keychain_error = keychain;
                self.selected = None;
                self.expanded = self.schema.by_schema().keys().map(|s| format!("s:{s}")).collect();
            }
            Ok(Reply::Schema(schema)) => { self.schema = schema; self.selected = None; }
            Ok(Reply::Grid(grid)) => self.output = Some(Output::Grid(grid)),
            Ok(Reply::Plan(plan)) => self.output = Some(Output::Plan(plan)),
            Err(e) => {
                if self.connecting { self.connecting = false; self.worker = None; }
                self.error = Some(e);
            }
        }
    }

    pub fn disconnect(&mut self) {
        self.cancel();
        // Dropping the job sender ends the worker, which closes the connection.
        self.worker = None;
        self.connecting = false;
        self.info = None;
        self.schema = schema::Schema::default();
        self.selected = None;
        self.output = None;
        self.keychain_error = None;
        self.expanded.clear();
    }

    /// For tests and captures: a connected state without a server.
    #[cfg(test)]
    pub fn set_connected_for_test(&mut self, info: ServerInfo, title: &str, schema: schema::Schema) {
        self.info = Some(info);
        self.title = title.into();
        self.expanded = schema.by_schema().keys().map(|s| format!("s:{s}")).collect();
        self.schema = schema;
    }
}

impl Drop for SqlServer {
    fn drop(&mut self) { self.cancel(); }
}

async fn watch(cancel: &AtomicBool, deadline: Instant) -> MsError {
    loop {
        if cancel.load(Ordering::Relaxed) { return MsError::Cancelled; }
        if Instant::now() >= deadline { return MsError::TimedOut; }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn worker(rx: mpsc::Receiver<(Job, Arc<AtomicBool>)>, out: rc_core::inbox::Outbox<Result<Reply, MsError>>, secrets: Arc<dyn secrets::Secrets>) {
    let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => { out.send(Err(MsError::Connect(e.to_string()))); return; }
    };
    let mut client: Option<driver::Conn> = None;
    let mut spec: Option<ConnSpec> = None;
    while let Ok((job, cancel)) = rx.recv() {
        let reply = rt.block_on(async {
            match job {
                Job::Connect { spec: mut s, remember } => {
                    let deadline = Instant::now() + CONNECT_LIMIT;
                    if s.auth == Auth::Sql && s.password.is_empty() {
                        s.password = secrets.get(&s.account()).ok_or(MsError::NoPassword)?;
                    }
                    let (mut c, info) = tokio::select! { r = driver::connect(&s) => r?, e = watch(&cancel, deadline) => return Err(e) };
                    let schema = tokio::select! { r = driver::schema(&mut c, MAX_TABLES) => r?, e = watch(&cancel, deadline + QUERY_LIMIT) => return Err(e) };
                    let keychain = if remember { secrets.set(&s.account(), &s.password).err() } else { None };
                    client = Some(c);
                    spec = Some(s);
                    Ok(Reply::Connected { info, schema, keychain })
                }
                Job::Run(request) => {
                    let deadline = Instant::now() + QUERY_LIMIT;
                    if client.is_none() {
                        // The previous connection was dropped by a cancel or timeout.
                        let s = spec.as_ref().ok_or(MsError::Cancelled)?;
                        let c = tokio::select! { r = driver::connect(s) => r?.0, e = watch(&cancel, deadline) => return Err(e) };
                        client = Some(c);
                    }
                    let c = client.as_mut().expect("connected");
                    let result = tokio::select! {
                        r = run(c, request) => r,
                        e = watch(&cancel, deadline) => Err(e),
                    };
                    if matches!(result, Err(MsError::Cancelled | MsError::TimedOut)) { client = None; }
                    result
                }
            }
        });
        if !out.send(reply) { break; }
    }
}

async fn run(client: &mut driver::Conn, request: Request) -> Result<Reply, MsError> {
    match request {
        Request::Schema => Ok(Reply::Schema(driver::schema(client, MAX_TABLES).await?)),
        Request::Query(sql) => {
            guard::check(&sql).map_err(MsError::Guard)?;
            Ok(Reply::Grid(driver::query(client, &sql).await?))
        }
        Request::Browse(schema_name, table) => {
            let sql = format!("SELECT TOP ({}) * FROM {}", crate::database::MAX_ROWS + 1, schema::quote(&schema_name, &table));
            Ok(Reply::Grid(driver::query(client, &sql).await?))
        }
        Request::Plan(sql) => {
            guard::check(&sql).map_err(MsError::Guard)?;
            Ok(Reply::Plan(driver::plan(client, &sql).await?))
        }
    }
}

/// A small fake schema for tests and captures.
#[cfg(test)]
pub fn sample_schema() -> schema::Schema {
    let col = |name: &str, ty: &str, pk: bool, nullable: bool, references: Option<&str>| schema::Column { name: name.into(), data_type: ty.into(), nullable, primary_key: pk, references: references.map(str::to_string) };
    schema::Schema {
        tables: vec![
            schema::Table { schema: "dbo".into(), name: "Customers".into(), view: false, columns: vec![col("Id", "int", true, false, None), col("Name", "nvarchar(100)", false, false, None), col("Email", "nvarchar(256)", false, true, None)],
                keys: vec![schema::Key { name: "PK_Customers".into(), kind: "PRIMARY KEY".into(), columns: vec!["Id".into()], target: None }] },
            schema::Table { schema: "dbo".into(), name: "Orders".into(), view: false, columns: vec![col("Id", "int", true, false, None), col("CustomerId", "int", false, false, Some("dbo.Customers.Id")), col("Total", "decimal(18,2)", false, false, None), col("Created", "datetime2", false, false, None)],
                keys: vec![schema::Key { name: "PK_Orders".into(), kind: "PRIMARY KEY".into(), columns: vec!["Id".into()], target: None }, schema::Key { name: "FK_Orders_Customers".into(), kind: "FOREIGN KEY".into(), columns: vec!["CustomerId".into()], target: Some("dbo.Customers".into()) }] },
            schema::Table { schema: "sales".into(), name: "TopCustomers".into(), view: true, columns: vec![col("Name", "nvarchar(100)", false, false, None), col("Total", "decimal(38,2)", false, true, None)], keys: Vec::new() },
        ],
        truncated: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrets::Secrets;

    #[test]
    fn form_rejects_localdb_and_named_pipes_before_connecting() {
        let mut server = SqlServer::with_secrets(Waker::noop(), Arc::new(secrets::Memory::default()));
        server.form.server = r"(localdb)\MSSQLLocalDB".into();
        server.connect();
        assert_eq!(server.error, Some(MsError::Parse(ParseError::LocalDb)));
        assert!(!server.busy());
        server.form.server = r"np:\\.\pipe\sql\query".into();
        server.connect();
        assert_eq!(server.error, Some(MsError::Parse(ParseError::NamedPipes)));
    }

    #[test]
    fn guard_errors_never_reach_the_worker() {
        let mut server = SqlServer::with_secrets(Waker::noop(), Arc::new(secrets::Memory::default()));
        server.set_connected_for_test(ServerInfo::default(), "x", schema::Schema::default());
        server.execute(Request::Query("DELETE FROM t".into()));
        assert_eq!(server.error, Some(MsError::Guard(GuardError::NotSelect("DELETE".into()))));
        assert!(!server.busy());
    }

    #[test]
    fn connection_failures_are_reported_and_saved_passwords_are_looked_up() {
        // A closed local port: the connect fails quickly without any server.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let memory = Arc::new(secrets::Memory::default());
        let mut server = SqlServer::with_secrets(Waker::noop(), memory.clone());
        server.form.server = format!("127.0.0.1,{port}");
        server.form.user = "reader".into();
        server.connect();
        let deadline = Instant::now() + Duration::from_secs(10);
        while server.error.is_none() { assert!(Instant::now() < deadline); server.update(); std::thread::sleep(Duration::from_millis(10)); }
        assert_eq!(server.error, Some(MsError::NoPassword));
        memory.set(&format!("reader@127.0.0.1,{port}"), "secret").unwrap();
        server.connect();
        let deadline = Instant::now() + Duration::from_secs(10);
        while server.busy() || server.connecting { assert!(Instant::now() < deadline); server.update(); std::thread::sleep(Duration::from_millis(10)); }
        assert!(matches!(server.error, Some(MsError::Connect(_))), "{:?}", server.error);
        assert!(!server.connected());
    }

    /// Runs against a real server when `DOTCRAFT_TEST_MSSQL` holds an ADO.NET connection string
    /// (e.g. `Server=localhost,1433;Database=master;User Id=sa;Password=…;TrustServerCertificate=true`).
    #[test]
    fn real_sql_server_when_configured() {
        let Ok(cs) = std::env::var("DOTCRAFT_TEST_MSSQL") else { return };
        let spec = conn::parse(&cs).expect("DOTCRAFT_TEST_MSSQL is not a valid connection string");
        let mut server = SqlServer::with_secrets(Waker::noop(), Arc::new(secrets::Memory::default()));
        server.form.fill(&spec);
        server.form.remember = false;
        server.connect();
        let settle = |server: &mut SqlServer| {
            let deadline = Instant::now() + Duration::from_secs(60);
            loop { server.update(); if !server.busy() && !server.connecting { break; } assert!(Instant::now() < deadline); std::thread::sleep(Duration::from_millis(10)); }
        };
        settle(&mut server);
        assert!(server.connected(), "{:?}", server.error);
        server.execute(Request::Query("SELECT 42 AS answer, CAST('2026-11-01' AS date) AS d".into()));
        settle(&mut server);
        let Some(Output::Grid(grid)) = &server.output else { panic!("{:?}", server.error) };
        assert_eq!(grid.columns, ["answer", "d"]);
        assert_eq!(grid.rows[0], [crate::database::Cell::Integer(42), crate::database::Cell::Text("2026-11-01".into())]);
        server.execute(Request::Plan("SELECT name FROM sys.objects WHERE object_id > 10".into()));
        settle(&mut server);
        assert!(matches!(&server.output, Some(Output::Plan(p)) if !p.nodes.is_empty()), "{:?}", server.error);
        server.execute(Request::Query("DELETE FROM sys.objects".into()));
        assert!(matches!(server.error, Some(MsError::Guard(_))));
        server.disconnect();
    }
}
