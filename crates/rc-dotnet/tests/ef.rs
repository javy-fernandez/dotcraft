//! `dotnet ef` against a tiny SQLite project: list contexts, add a migration, update the
//! database, script and remove it. Skipped (with a message) when `dotnet-ef` is not installed
//! or the EF Core packages cannot be restored (offline).

use std::path::Path;
use std::time::{Duration, Instant};

use rc_core::{Inbox, Waker};
use rc_dotnet::ef::{self, EfCommand, EfTarget};
use rc_dotnet::{BuildEvent, BuildResult, CancelToken, Dotnet};

const PROJECT: &str = r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <OutputType>Exe</OutputType>
    <TargetFramework>net10.0</TargetFramework>
    <Nullable>enable</Nullable>
  </PropertyGroup>
  <ItemGroup>
    <PackageReference Include="Microsoft.EntityFrameworkCore.Sqlite" Version="10.0.*" />
    <PackageReference Include="Microsoft.EntityFrameworkCore.Design" Version="10.0.*" />
  </ItemGroup>
</Project>
"#;

const PROGRAM: &str = r#"using Microsoft.EntityFrameworkCore;
public class Blog { public int Id { get; set; } public string Name { get; set; } = ""; }
public class BlogContext : DbContext {
    public DbSet<Blog> Blogs => Set<Blog>();
    protected override void OnConfiguring(DbContextOptionsBuilder o) => o.UseSqlite("Data Source=blog.db");
}
public static class Program { public static void Main() {} }
"#;

fn run(dotnet: &Dotnet, args: Vec<String>, dir: &Path) -> (BuildResult, Vec<String>) {
    let inbox = Inbox::new(Waker::noop());
    let _h = ef::start(dotnet, args, Some(dir.to_path_buf()), inbox.sender());
    let deadline = Instant::now() + Duration::from_secs(600);
    let mut lines = Vec::new();
    while Instant::now() < deadline {
        match inbox.recv_timeout(Duration::from_secs(1)) {
            Some(BuildEvent::Output(l)) => lines.push(l.text),
            Some(BuildEvent::Finished(r)) => return (r, lines),
            Some(BuildEvent::Failed(e)) => panic!("{e}"),
            _ => {}
        }
    }
    panic!("dotnet ef timed out: {lines:?}");
}

#[test]
fn migrations_round_trip_on_sqlite() {
    let dotnet = Dotnet::locate(None).expect("the dotnet SDK must be installed");
    let tmp = tempfile::Builder::new().prefix("dotcraft-ef").tempdir().unwrap();
    let dir = tmp.path().canonicalize().unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let Some(version) = rt.block_on(ef::tool_version(&dotnet, &dir)) else {
        eprintln!("dotnet-ef is not installed: skipped");
        return;
    };
    assert!(version.chars().next().unwrap().is_ascii_digit());
    let project = dir.join("Blogs.csproj");
    std::fs::write(&project, PROJECT).unwrap();
    std::fs::write(dir.join("Program.cs"), PROGRAM).unwrap();
    let restore = dotnet.std_command().args(["restore", "-nologo"]).current_dir(&dir).output().unwrap();
    if !restore.status.success() {
        eprintln!("could not restore EF Core (offline?): skipped\n{}", String::from_utf8_lossy(&restore.stdout));
        return;
    }
    let target = EfTarget { project: project.clone(), startup: project.clone(), context: None };
    let cancel = CancelToken::new();

    let contexts = rt.block_on(ef::list_contexts(&dotnet, &target, &cancel)).unwrap();
    assert_eq!(contexts.iter().map(|c| c.id()).collect::<Vec<_>>(), ["BlogContext"]);
    let target = EfTarget { context: Some(contexts[0].id().to_string()), ..target };

    let (r, lines) = run(&dotnet, EfCommand::AddMigration { name: "Initial".into(), output_dir: None }.args(&target), &dir);
    assert!(r.success, "{lines:?}");
    let added = ef::parse_added(&ef::data_json(lines.iter().map(String::as_str))).unwrap();
    assert!(added.migration_file.is_file() && added.migration_file.starts_with(dir.join("Migrations")), "{added:?}");

    let migrations = rt.block_on(ef::list_migrations(&dotnet, &target, &cancel)).unwrap();
    assert_eq!(migrations.len(), 1);
    assert_eq!((migrations[0].name.as_str(), migrations[0].applied), ("Initial", Some(false)));

    let (r, lines) = run(&dotnet, EfCommand::UpdateDatabase { target: None, connection: None }.args(&target), &dir);
    assert!(r.success, "{lines:?}");
    let migrations = rt.block_on(ef::list_migrations(&dotnet, &target, &cancel)).unwrap();
    assert_eq!(migrations[0].applied, Some(true));

    let sql = dir.join("script.sql");
    let (r, lines) = run(&dotnet, EfCommand::Script { from: None, to: None, idempotent: false, output: sql.clone() }.args(&target), &dir);
    assert!(r.success, "{lines:?}");
    assert!(std::fs::read_to_string(&sql).unwrap().contains("CREATE TABLE"));

    // Removing an applied migration needs --force (it reverts it first).
    let (r, lines) = run(&dotnet, EfCommand::RemoveMigration { force: true }.args(&target), &dir);
    assert!(r.success, "{lines:?}");
    assert!(!added.migration_file.exists());

    // A failing command reports its error lines.
    let bad = EfTarget { context: Some("NoSuchContext".into()), ..target };
    let err = rt.block_on(ef::list_migrations(&dotnet, &bad, &cancel)).unwrap_err().to_string();
    assert!(err.contains("NoSuchContext"), "{err}");
}
