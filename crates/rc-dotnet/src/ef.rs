//! Entity Framework Core through the `dotnet ef` tool: command lines for migrations, database
//! updates, SQL scripts and reverse engineering (scaffolding), the `--json` listings of
//! DbContexts and migrations, and detecting / installing the `dotnet-ef` tool.
//!
//! Every command uses `--prefix-output`, so each line starts with `info:`, `warn:`, `error:`,
//! `verbose:` or `data:`; the JSON of `--json` is the concatenation of the `data:` lines.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Instant;

use rc_core::Runtime;
use rc_core::inbox::Outbox;
use serde::Deserialize;

use crate::build::{BuildEvent, BuildResult};
use crate::process::{CancelToken, OpHandle, OutputLine, Stream, StreamOpts, run_streaming};
use crate::sdk::Dotnet;

/// Database providers offered when scaffolding (package names, as `dotnet ef` expects them).
pub const PROVIDERS: &[&str] = &[
    "Microsoft.EntityFrameworkCore.SqlServer",
    "Npgsql.EntityFrameworkCore.PostgreSQL",
    "Microsoft.EntityFrameworkCore.Sqlite",
    "Pomelo.EntityFrameworkCore.MySql",
    "MySql.EntityFrameworkCore",
    "Oracle.EntityFrameworkCore",
    "FirebirdSql.EntityFrameworkCore.Firebird",
];

/// The package the startup project needs for design-time commands.
pub const DESIGN_PACKAGE: &str = "Microsoft.EntityFrameworkCore.Design";

/// Which projects a command works on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EfTarget {
    /// The project with the DbContext and the migrations (`--project`).
    pub project: PathBuf,
    /// The project that is built and run to create the DbContext (`--startup-project`).
    pub startup: PathBuf,
    /// The DbContext type, when the project has several (`--context`).
    pub context: Option<String>,
}

/// Options of `dbcontext scaffold`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scaffold {
    pub connection: String,
    pub provider: String,
    /// Folder of the entity classes, relative to the project (`--output-dir`).
    pub output_dir: String,
    /// Folder of the DbContext (`--context-dir`); the output folder when empty.
    pub context_dir: String,
    /// Name of the generated DbContext (`--context`); derived from the database when empty.
    pub context_name: String,
    pub tables: Vec<String>,
    pub schemas: Vec<String>,
    pub data_annotations: bool,
    pub use_database_names: bool,
    /// Do not write the connection string into `OnConfiguring`.
    pub no_onconfiguring: bool,
    /// Overwrite existing files.
    pub force: bool,
}

/// A `dotnet ef` command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EfCommand {
    AddMigration { name: String, output_dir: Option<String> },
    /// Removes the last migration (`force` reverts it first when it is applied).
    RemoveMigration { force: bool },
    /// Updates the database to `target` (a migration id or name, `0` for none); the last
    /// migration when `None`.
    UpdateDatabase { target: Option<String>, connection: Option<String> },
    /// A SQL script from `from` (exclusive, `0` = empty database) to `to` (the last one when
    /// `None`), written to `output`.
    Script { from: Option<String>, to: Option<String>, idempotent: bool, output: PathBuf },
    Scaffold(Scaffold),
    ListMigrations,
    ListContexts,
}

impl EfCommand {
    /// Arguments after `dotnet`.
    pub fn args(&self, target: &EfTarget) -> Vec<String> {
        let s = |v: &str| v.to_string();
        let mut a: Vec<String> = vec![s("ef")];
        let mut context = target.context.clone().filter(|c| !c.trim().is_empty());
        match self {
            EfCommand::AddMigration { name, output_dir } => {
                a.extend([s("migrations"), s("add"), name.trim().to_string()]);
                if let Some(d) = output_dir.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
                    a.extend([s("--output-dir"), d.to_string()]);
                }
                a.push(s("--json"));
            }
            EfCommand::RemoveMigration { force } => {
                a.extend([s("migrations"), s("remove")]);
                if *force {
                    a.push(s("--force"));
                }
            }
            EfCommand::UpdateDatabase { target, connection } => {
                a.extend([s("database"), s("update")]);
                if let Some(t) = target.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
                    a.push(t.to_string());
                }
                if let Some(c) = connection.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
                    a.extend([s("--connection"), c.to_string()]);
                }
            }
            EfCommand::Script { from, to, idempotent, output } => {
                a.extend([s("migrations"), s("script")]);
                let from = from.as_deref().map(str::trim).filter(|f| !f.is_empty());
                let to = to.as_deref().map(str::trim).filter(|t| !t.is_empty());
                // `to` is positional after `from`: an explicit `0` keeps the order.
                if from.is_some() || to.is_some() {
                    a.push(from.unwrap_or("0").to_string());
                }
                if let Some(t) = to {
                    a.push(t.to_string());
                }
                if *idempotent {
                    a.push(s("--idempotent"));
                }
                a.extend([s("--output"), output.to_string_lossy().into_owned()]);
            }
            EfCommand::Scaffold(sc) => {
                a.extend([s("dbcontext"), s("scaffold"), sc.connection.trim().to_string(), sc.provider.trim().to_string()]);
                // `--context` names the generated DbContext here.
                context = Some(sc.context_name.trim().to_string()).filter(|c| !c.is_empty());
                if !sc.output_dir.trim().is_empty() {
                    a.extend([s("--output-dir"), sc.output_dir.trim().to_string()]);
                }
                if !sc.context_dir.trim().is_empty() {
                    a.extend([s("--context-dir"), sc.context_dir.trim().to_string()]);
                }
                for t in sc.tables.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
                    a.extend([s("--table"), t.to_string()]);
                }
                for t in sc.schemas.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
                    a.extend([s("--schema"), t.to_string()]);
                }
                if sc.data_annotations {
                    a.push(s("--data-annotations"));
                }
                if sc.use_database_names {
                    a.push(s("--use-database-names"));
                }
                if sc.no_onconfiguring {
                    a.push(s("--no-onconfiguring"));
                }
                if sc.force {
                    a.push(s("--force"));
                }
            }
            EfCommand::ListMigrations => a.extend([s("migrations"), s("list"), s("--json")]),
            EfCommand::ListContexts => {
                a.extend([s("dbcontext"), s("list"), s("--json")]);
                context = None;
            }
        }
        a.extend([s("--project"), target.project.to_string_lossy().into_owned()]);
        a.extend([s("--startup-project"), target.startup.to_string_lossy().into_owned()]);
        if let Some(c) = context {
            a.extend([s("--context"), c]);
        }
        a.push(s("--prefix-output"));
        a
    }

    /// What is wrong with the command before running it (an English message for `tr`).
    pub fn problem(&self) -> Option<&'static str> {
        match self {
            EfCommand::AddMigration { name, .. } => {
                let n = name.trim();
                if n.is_empty() {
                    Some("Enter a migration name")
                } else if !n.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_') || !n.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    Some("The migration name must be a valid C# identifier")
                } else {
                    None
                }
            }
            EfCommand::Script { output, .. } if output.as_os_str().is_empty() => Some("Choose the output file"),
            EfCommand::Scaffold(sc) if sc.connection.trim().is_empty() => Some("Enter the connection string"),
            EfCommand::Scaffold(sc) if sc.provider.trim().is_empty() => Some("Choose the database provider"),
            _ => None,
        }
    }
}

/// A DbContext type (`dbcontext list --json`).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbContextInfo {
    pub full_name: String,
    #[serde(default)]
    pub safe_name: String,
    #[serde(default)]
    pub name: String,
}

impl DbContextInfo {
    /// What `--context` accepts unambiguously.
    pub fn id(&self) -> &str {
        if self.full_name.is_empty() { &self.name } else { &self.full_name }
    }
}

/// A migration (`migrations list --json`). `applied` is `None` when the database could not
/// be reached.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Migration {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub safe_name: String,
    #[serde(default)]
    pub applied: Option<bool>,
}

/// Files written by `migrations add --json`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddedMigration {
    pub migration_file: PathBuf,
    #[serde(default)]
    pub metadata_file: Option<PathBuf>,
    #[serde(default)]
    pub snapshot_file: Option<PathBuf>,
}

/// Severity of a `--prefix-output` line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Data,
    Info,
    Verbose,
    Warn,
    Error,
    /// No prefix (output of a child process, the .NET host…).
    Plain,
}

/// Splits a `--prefix-output` line into its level and text.
pub fn classify(line: &str) -> (Level, &str) {
    for (prefix, level) in [("data:", Level::Data), ("info:", Level::Info), ("verbose:", Level::Verbose), ("warn:", Level::Warn), ("error:", Level::Error)] {
        if let Some(rest) = line.strip_prefix(prefix) {
            return (level, rest.trim_start());
        }
    }
    (Level::Plain, line)
}

/// The JSON written with `--json --prefix-output`: the `data:` lines joined. Without prefixes
/// (older tools), from the first line that opens an object or an array.
pub fn data_json<'a>(lines: impl IntoIterator<Item = &'a str>) -> String {
    let lines: Vec<&str> = lines.into_iter().collect();
    let data: Vec<&str> = lines
        .iter()
        .filter_map(|l| {
            let (level, _) = classify(l);
            // Keep the indentation after `data:    ` (four spaces of padding).
            (level == Level::Data).then(|| l.strip_prefix("data:").unwrap_or(l).strip_prefix("    ").unwrap_or_else(|| l["data:".len()..].trim_start()))
        })
        .collect();
    if !data.is_empty() {
        return data.join("\n");
    }
    match lines.iter().position(|l| l.trim_start().starts_with('{') || l.trim_start().starts_with('[')) {
        Some(i) => lines[i..].join("\n"),
        None => String::new(),
    }
}

pub fn parse_contexts(json: &str) -> anyhow::Result<Vec<DbContextInfo>> {
    Ok(serde_json::from_str(json)?)
}

pub fn parse_migrations(json: &str) -> anyhow::Result<Vec<Migration>> {
    Ok(serde_json::from_str(json)?)
}

pub fn parse_added(json: &str) -> anyhow::Result<AddedMigration> {
    Ok(serde_json::from_str(json)?)
}

/// The error lines of a failed command, for a message (the last lines when there are none).
pub fn error_text<'a>(lines: impl IntoIterator<Item = &'a str>) -> String {
    let lines: Vec<&str> = lines.into_iter().filter(|l| !l.trim().is_empty()).collect();
    let errors: Vec<&str> = lines.iter().filter_map(|l| match classify(l) {
        (Level::Error, t) => Some(t),
        _ => None,
    }).collect();
    if !errors.is_empty() {
        return errors.join("\n");
    }
    lines.iter().rev().take(3).rev().map(|l| classify(l).1).collect::<Vec<_>>().join("\n")
}

/// The `dotnet-ef` version printed by `dotnet ef --version` (its last line).
pub fn parse_version(output: &str) -> Option<String> {
    output.lines().map(str::trim).rev().find(|l| !l.is_empty() && l.chars().next().is_some_and(|c| c.is_ascii_digit())).map(str::to_string)
}

/// `PATH` for `dotnet ef`: the folder of the host and the global tools folder first. An app
/// started from Finder gets a bare `PATH` where neither `dotnet` nor `dotnet-ef` is found.
pub fn tool_path(dotnet_root: Option<&Path>, home: Option<&Path>, path_var: Option<OsString>) -> OsString {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(r) = dotnet_root {
        dirs.push(r.to_path_buf());
    }
    if let Some(h) = home {
        dirs.push(h.join(".dotnet").join("tools"));
    }
    if let Some(p) = &path_var {
        for d in std::env::split_paths(p) {
            if !dirs.contains(&d) {
                dirs.push(d);
            }
        }
    }
    std::env::join_paths(dirs).unwrap_or_else(|_| path_var.unwrap_or_default())
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from)
}

/// A `dotnet` command able to find `dotnet-ef` (global or local tool).
pub fn command(dotnet: &Dotnet, args: &[String], dir: Option<&Path>) -> tokio::process::Command {
    let mut cmd = dotnet.command();
    cmd.args(args);
    cmd.env("PATH", tool_path(dotnet.root(), home().as_deref(), std::env::var_os("PATH")));
    if let Some(root) = dotnet.root() {
        // The tool's apphost looks for the runtime here.
        cmd.env("DOTNET_ROOT", root);
    }
    if let Some(d) = dir {
        cmd.current_dir(d);
    }
    cmd
}

/// The local tool manifest (`.config/dotnet-tools.json` or `dotnet-tools.json`) that applies
/// to `dir`, if any.
pub fn tool_manifest(dir: &Path) -> Option<PathBuf> {
    dir.ancestors().flat_map(|d| [d.join(".config").join("dotnet-tools.json"), d.join("dotnet-tools.json")]).find(|p| p.is_file())
}

/// Installs `dotnet-ef`: into the local tool manifest when the solution has one, globally
/// otherwise.
pub fn install_args(local_manifest: bool) -> Vec<String> {
    let mut a: Vec<String> = ["tool", "install"].map(String::from).to_vec();
    if !local_manifest {
        a.push("--global".into());
    }
    a.push("dotnet-ef".into());
    a
}

/// Whether a project references Entity Framework Core (a provider, the Design package…).
pub fn uses_ef(info: &crate::ProjectInfo) -> bool {
    info.packages.iter().any(|p| p.name.to_ascii_lowercase().contains("entityframeworkcore"))
}

/// Whether a project references EF Core itself or a provider (not only the design-time or
/// tools packages): where the DbContext and the migrations usually live.
pub fn has_runtime(info: &crate::ProjectInfo) -> bool {
    info.packages.iter().any(|p| {
        let n = p.name.to_ascii_lowercase();
        n.contains("entityframeworkcore") && !n.ends_with(".design") && !n.ends_with(".tools")
    })
}

/// Whether a project references the design-time package (needed by the startup project).
pub fn has_design(info: &crate::ProjectInfo) -> bool {
    info.packages.iter().any(|p| p.name.eq_ignore_ascii_case(DESIGN_PACKAGE) || p.name.eq_ignore_ascii_case("Microsoft.EntityFrameworkCore.Tools"))
}

/// The provider package a project references, if any.
pub fn provider_of(info: &crate::ProjectInfo) -> Option<&'static str> {
    PROVIDERS.iter().copied().find(|prov| info.packages.iter().any(|p| p.name.eq_ignore_ascii_case(prov)))
}

/// `dotnet ef --version` in `dir` (so a local tool manifest applies); `None` when the tool is
/// not installed. Blocking-free: runs on the caller's runtime.
pub async fn tool_version(dotnet: &Dotnet, dir: &Path) -> Option<String> {
    let mut cmd = command(dotnet, &["ef".into(), "--version".into()], Some(dir));
    cmd.stdin(std::process::Stdio::null());
    cmd.kill_on_drop(true);
    let out = tokio::time::timeout(std::time::Duration::from_secs(60), cmd.output()).await.ok()?.ok()?;
    if !out.status.success() {
        return None;
    }
    parse_version(&String::from_utf8_lossy(&out.stdout))
}

/// Runs a listing command and returns its JSON (the `data:` lines); an error carries the
/// tool's error lines.
pub async fn run_json(dotnet: &Dotnet, cmd: &EfCommand, target: &EfTarget, cancel: &CancelToken) -> anyhow::Result<String> {
    let args = cmd.args(target);
    let mut lines: Vec<String> = Vec::new();
    let outcome = run_streaming(command(dotnet, &args, target.project.parent()), cancel, StreamOpts::default(), &mut |_| {}, &mut |l: OutputLine| lines.push(l.text)).await?;
    if outcome.cancelled {
        anyhow::bail!("cancelled");
    }
    if outcome.code != Some(0) {
        anyhow::bail!("{}", error_text(lines.iter().map(String::as_str)));
    }
    Ok(data_json(lines.iter().map(String::as_str)))
}

pub async fn list_contexts(dotnet: &Dotnet, target: &EfTarget, cancel: &CancelToken) -> anyhow::Result<Vec<DbContextInfo>> {
    parse_contexts(&run_json(dotnet, &EfCommand::ListContexts, target, cancel).await?)
}

pub async fn list_migrations(dotnet: &Dotnet, target: &EfTarget, cancel: &CancelToken) -> anyhow::Result<Vec<Migration>> {
    parse_migrations(&run_json(dotnet, &EfCommand::ListMigrations, target, cancel).await?)
}

/// Starts a `dotnet` command (an EF command or the tool install) streamed as [`BuildEvent`]s,
/// so it shows in the Build window. Error lines go to stderr (red); `Finished` reports the
/// exit code. Dropping or cancelling the handle kills it.
pub fn start(dotnet: &Dotnet, args: Vec<String>, dir: Option<PathBuf>, outbox: Outbox<BuildEvent>) -> OpHandle {
    let cancel = CancelToken::new();
    let token = cancel.clone();
    let dotnet = dotnet.clone();
    let task = Runtime::spawn(async move {
        let started = Instant::now();
        outbox.send(BuildEvent::Started { command: format!("dotnet {}", args.join(" ")) });
        let out = outbox.clone();
        let mut errors = 0;
        let mut warnings = 0;
        let result = run_streaming(command(&dotnet, &args, dir.as_deref()), &token, StreamOpts::default(), &mut |_| {}, &mut |mut l: OutputLine| {
            match classify(&l.text).0 {
                Level::Error => {
                    errors += 1;
                    l.stream = Stream::Stderr;
                }
                Level::Warn => {
                    warnings += 1;
                    l.text = format!("\x1b[33m{}\x1b[0m", l.text);
                }
                Level::Verbose => l.text = format!("\x1b[90m{}\x1b[0m", l.text),
                _ => {}
            }
            out.send(BuildEvent::Output(l));
        })
        .await;
        match result {
            Ok(o) => outbox.send(BuildEvent::Finished(BuildResult {
                success: !o.cancelled && o.code == Some(0),
                cancelled: o.cancelled,
                exit_code: o.code,
                errors,
                warnings,
                diagnostics: Vec::new(),
                duration: started.elapsed(),
            })),
            Err(e) => outbox.send(BuildEvent::Failed(format!("{e:#}"))),
        };
    });
    OpHandle::new(cancel, task)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> EfTarget {
        EfTarget { project: PathBuf::from("/s/Data/Data.csproj"), startup: PathBuf::from("/s/Api/Api.csproj"), context: Some("ShopContext".into()) }
    }

    fn tail(a: &[String]) -> Vec<&str> {
        a.iter().map(String::as_str).collect()
    }

    #[test]
    fn migration_commands_name_the_projects_and_the_context() {
        let a = EfCommand::AddMigration { name: " AddOrders ".into(), output_dir: Some("Data/Migrations".into()) }.args(&target());
        assert_eq!(
            tail(&a),
            ["ef", "migrations", "add", "AddOrders", "--output-dir", "Data/Migrations", "--json", "--project", "/s/Data/Data.csproj", "--startup-project", "/s/Api/Api.csproj", "--context", "ShopContext", "--prefix-output"]
        );
        let a = EfCommand::RemoveMigration { force: true }.args(&EfTarget { context: None, ..target() });
        assert_eq!(tail(&a)[..4], ["ef", "migrations", "remove", "--force"]);
        assert!(!a.contains(&"--context".to_string()));
        let a = EfCommand::UpdateDatabase { target: Some("0".into()), connection: Some("Data Source=x.db".into()) }.args(&target());
        assert_eq!(tail(&a)[..6], ["ef", "database", "update", "0", "--connection", "Data Source=x.db"]);
        let a = EfCommand::UpdateDatabase { target: None, connection: Some("  ".into()) }.args(&target());
        assert_eq!(tail(&a)[..4], ["ef", "database", "update", "--project"]);
    }

    #[test]
    fn scripts_keep_from_before_to() {
        let out = PathBuf::from("/tmp/m.sql");
        let a = EfCommand::Script { from: None, to: Some("Init".into()), idempotent: true, output: out.clone() }.args(&target());
        assert_eq!(tail(&a)[..8], ["ef", "migrations", "script", "0", "Init", "--idempotent", "--output", "/tmp/m.sql"]);
        let a = EfCommand::Script { from: Some("Init".into()), to: None, idempotent: false, output: out.clone() }.args(&target());
        assert_eq!(tail(&a)[..6], ["ef", "migrations", "script", "Init", "--output", "/tmp/m.sql"]);
        let a = EfCommand::Script { from: None, to: None, idempotent: false, output: out }.args(&target());
        assert_eq!(tail(&a)[..5], ["ef", "migrations", "script", "--output", "/tmp/m.sql"]);
    }

    #[test]
    fn scaffold_uses_the_new_context_name_not_the_selected_one() {
        let sc = Scaffold {
            connection: "Server=.;Database=Shop".into(),
            provider: "Microsoft.EntityFrameworkCore.SqlServer".into(),
            output_dir: "Models".into(),
            context_name: "ShopDb".into(),
            tables: vec!["dbo.Orders".into(), " ".into()],
            data_annotations: true,
            force: true,
            ..Default::default()
        };
        let a = EfCommand::Scaffold(sc.clone()).args(&target());
        assert_eq!(
            tail(&a),
            [
                "ef", "dbcontext", "scaffold", "Server=.;Database=Shop", "Microsoft.EntityFrameworkCore.SqlServer", "--output-dir", "Models", "--table", "dbo.Orders", "--data-annotations", "--force", "--project", "/s/Data/Data.csproj",
                "--startup-project", "/s/Api/Api.csproj", "--context", "ShopDb", "--prefix-output"
            ]
        );
        let a = EfCommand::Scaffold(Scaffold { context_name: String::new(), ..sc }).args(&target());
        assert!(!a.contains(&"--context".to_string()));
        let a = EfCommand::ListContexts.args(&target());
        assert!(!a.contains(&"--context".to_string()) && a.contains(&"--json".to_string()));
    }

    #[test]
    fn problems_block_invalid_input() {
        assert!(EfCommand::AddMigration { name: "".into(), output_dir: None }.problem().is_some());
        assert!(EfCommand::AddMigration { name: "1st".into(), output_dir: None }.problem().is_some());
        assert!(EfCommand::AddMigration { name: "Add Orders".into(), output_dir: None }.problem().is_some());
        assert!(EfCommand::AddMigration { name: "AddOrders_2".into(), output_dir: None }.problem().is_none());
        assert!(EfCommand::Scaffold(Scaffold { provider: "x".into(), ..Default::default() }).problem().is_some());
        assert!(EfCommand::Script { from: None, to: None, idempotent: false, output: PathBuf::new() }.problem().is_some());
    }

    #[test]
    fn parses_prefixed_json_listings() {
        let out = "info:    Build started...\ninfo:    Build succeeded.\nwarn:    The Entity Framework tools version '10.0.8' is older than that of the runtime.\ndata:    [\ndata:      {\ndata:        \"id\": \"20261111223732_Initial\",\ndata:        \"name\": \"Initial\",\ndata:        \"safeName\": \"Initial\",\ndata:        \"applied\": false\ndata:      },\ndata:      {\"id\": \"20261112000000_Orders\", \"name\": \"Orders\", \"safeName\": \"Orders\", \"applied\": null}\ndata:    ]";
        let m = parse_migrations(&data_json(out.lines())).unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!((m[0].id.as_str(), m[0].name.as_str(), m[0].applied), ("20261111223732_Initial", "Initial", Some(false)));
        assert_eq!(m[1].applied, None);

        let out = "info:    Build started...\ndata:    [\ndata:      {\ndata:         \"fullName\": \"Shop.Data.ShopContext\",\ndata:         \"safeName\": \"ShopContext\",\ndata:         \"name\": \"ShopContext\",\ndata:         \"assemblyQualifiedName\": \"Shop.Data.ShopContext, Shop\"\ndata:      }\ndata:    ]";
        let c = parse_contexts(&data_json(out.lines())).unwrap();
        assert_eq!(c[0].id(), "Shop.Data.ShopContext");
        assert_eq!(c[0].name, "ShopContext");
    }

    #[test]
    fn parses_unprefixed_add_output_and_errors() {
        let out = "Build started...\nBuild succeeded.\n{\n  \"migrationFile\": \"/s/Migrations/1_Init.cs\",\n  \"metadataFile\": \"/s/Migrations/1_Init.Designer.cs\",\n  \"snapshotFile\": \"/s/Migrations/ShopModelSnapshot.cs\"\n}";
        assert_eq!(parse_added(&data_json(out.lines())).unwrap().migration_file, PathBuf::from("/s/Migrations/1_Init.cs"));
        let err = "info:    Build started...\nerror:   No DbContext was found in assembly 'Api'.\nerror:   Ensure that you're using the correct startup project.";
        assert_eq!(error_text(err.lines()), "No DbContext was found in assembly 'Api'.\nEnsure that you're using the correct startup project.");
        assert_eq!(error_text("a\nb\nc\nd".lines()), "b\nc\nd");
    }

    #[test]
    fn versions_paths_and_install() {
        assert_eq!(parse_version("Entity Framework Core .NET Command-line Tools\n10.0.8\n"), Some("10.0.8".into()));
        assert_eq!(parse_version("Could not execute because the specified command or file was not found."), None);
        let p = tool_path(Some(Path::new("/usr/local/share/dotnet")), Some(Path::new("/Users/me")), Some(std::env::join_paths(["/usr/bin", "/usr/local/share/dotnet"]).unwrap()));
        let dirs: Vec<PathBuf> = std::env::split_paths(&p).collect();
        assert_eq!(dirs, [PathBuf::from("/usr/local/share/dotnet"), Path::new("/Users/me").join(".dotnet").join("tools"), PathBuf::from("/usr/bin")]);
        assert_eq!(install_args(false), ["tool", "install", "--global", "dotnet-ef"]);
        assert_eq!(install_args(true), ["tool", "install", "dotnet-ef"]);
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("src/App");
        std::fs::create_dir_all(&sub).unwrap();
        assert_eq!(tool_manifest(&sub), None);
        std::fs::create_dir_all(dir.path().join(".config")).unwrap();
        std::fs::write(dir.path().join(".config/dotnet-tools.json"), "{}").unwrap();
        assert_eq!(tool_manifest(&sub), Some(dir.path().join(".config/dotnet-tools.json")));
    }

    #[test]
    fn classifies_prefixed_lines() {
        assert_eq!(classify("warn:    old tools"), (Level::Warn, "old tools"));
        assert_eq!(classify("error:   boom"), (Level::Error, "boom"));
        assert_eq!(classify("Build started"), (Level::Plain, "Build started"));
    }
}
