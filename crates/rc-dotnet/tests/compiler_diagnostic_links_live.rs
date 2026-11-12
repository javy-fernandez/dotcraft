//! Live SDK acceptance for compiler diagnostics used as editor links. Run with
//! `DOTCRAFT_TEST_DOTNET=/path/to/dotnet cargo test -p rc-dotnet --test compiler_diagnostic_links_live -- --ignored`.

use std::path::{Path, PathBuf};

use rc_dotnet::build::{BuildAction, BuildEvent, BuildRequest, build};
use rc_dotnet::{CancelToken, Dotnet, Severity};

fn copy_shop_core(to: &Path) -> PathBuf {
    let from =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop/src/Shop.Core");
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("cs" | "csproj")
        ) {
            std::fs::copy(&path, to.join(entry.file_name())).unwrap();
        }
    }
    to.join("Shop.Core.csproj")
}

#[tokio::test]
#[ignore = "requires a locally installed .NET 8 SDK; set DOTCRAFT_TEST_DOTNET"]
async fn reports_a_real_compiler_error_with_clickable_file_and_position() {
    let host = std::env::var_os("DOTCRAFT_TEST_DOTNET")
        .expect("set DOTCRAFT_TEST_DOTNET to a .NET 8 dotnet host");
    let dotnet = Dotnet::new(host);
    assert!(dotnet.version(None).unwrap().starts_with("8."));

    let dir = tempfile::tempdir().unwrap();
    let project = copy_shop_core(&dir.path().join("Shop.Core"));
    let file = project.parent().unwrap().join("PriceCalculator.cs");
    let source = std::fs::read_to_string(&file).unwrap();
    let mut lines: Vec<&str> = source.lines().collect();
    let insertion = lines
        .iter()
        .position(|line| line.contains("var subtotal = 0m;"))
        .unwrap()
        + 1;
    lines.insert(insertion, "        var broken = undefinedName;");
    std::fs::write(&file, lines.join("\n")).unwrap();

    let mut events = Vec::new();
    let result = build(
        &dotnet,
        &BuildRequest::new(&project, BuildAction::Build),
        &CancelToken::new(),
        &mut |event| events.push(event),
    )
    .await
    .unwrap();
    assert!(!result.success && result.errors >= 1, "{result:?}");

    let diagnostic = result.errors().find(|d| d.code == "CS0103").unwrap();
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(diagnostic.file.as_deref(), Some(file.as_path()));
    assert_eq!(diagnostic.project.as_deref(), Some(project.as_path()));
    assert_eq!(diagnostic.line, Some(insertion as u32 + 1));
    assert_eq!(diagnostic.column, Some(22));
    assert!(diagnostic.message.contains("undefinedName"));
    assert!(
        events
            .iter()
            .any(|event| matches!(event, BuildEvent::Diagnostic(d) if d == diagnostic))
    );
}
