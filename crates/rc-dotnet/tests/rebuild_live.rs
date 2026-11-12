//! Live SDK acceptance for the rebuild command. Run with
//! `DOTCRAFT_TEST_DOTNET=/path/to/dotnet cargo test -p rc-dotnet --test rebuild_live -- --ignored`.

use std::path::{Path, PathBuf};

use rc_dotnet::build::{BuildAction, BuildEvent, BuildRequest, build};
use rc_dotnet::{CancelToken, Dotnet};

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

async fn execute(
    dotnet: &Dotnet,
    request: &BuildRequest,
) -> (rc_dotnet::BuildResult, Vec<BuildEvent>) {
    let mut events = Vec::new();
    let result = build(dotnet, request, &CancelToken::new(), &mut |event| {
        events.push(event)
    })
    .await
    .unwrap();
    (result, events)
}

#[tokio::test]
#[ignore = "requires a locally installed .NET 8 SDK; set DOTCRAFT_TEST_DOTNET"]
async fn rebuilds_a_real_shop_project_and_restores_its_output() {
    let host = std::env::var_os("DOTCRAFT_TEST_DOTNET")
        .expect("set DOTCRAFT_TEST_DOTNET to a .NET 8 dotnet host");
    let dotnet = Dotnet::new(host);
    assert!(dotnet.version(None).unwrap().starts_with("8."));

    let dir = tempfile::tempdir().unwrap();
    let project = copy_shop_core(&dir.path().join("Shop.Core"));
    let dll = project
        .parent()
        .unwrap()
        .join("bin/Debug/net8.0/Shop.Core.dll");
    let intermediate = project
        .parent()
        .unwrap()
        .join("obj/Debug/net8.0/Shop.Core.dll");

    let (initial, _) = execute(&dotnet, &BuildRequest::new(&project, BuildAction::Build)).await;
    assert!(initial.success, "initial build: {initial:?}");
    let built = std::fs::read(&dll).unwrap();
    assert!(built.len() > 1024 && built.starts_with(b"MZ"));
    assert_eq!(std::fs::read(&intermediate).unwrap(), built);

    std::fs::write(&dll, b"corrupt output; source files are unchanged").unwrap();
    std::fs::write(
        &intermediate,
        b"corrupt intermediate; source files are unchanged",
    )
    .unwrap();
    let mut rebuild = BuildRequest::new(&project, BuildAction::Rebuild);
    rebuild.no_restore = true;
    let (result, events) = execute(&dotnet, &rebuild).await;
    assert!(
        result.success && !result.cancelled && result.exit_code == Some(0),
        "{result:?}"
    );
    assert_eq!(result.errors, 0);
    assert_eq!(std::fs::read(&dll).unwrap(), built);
    assert_eq!(std::fs::read(&intermediate).unwrap(), built);
    assert!(
        matches!(&events[0], BuildEvent::Started { command } if command.contains("--no-incremental") && command.contains("--no-restore"))
    );
    assert!(events.iter().any(
        |event| matches!(event, BuildEvent::Output(line) if line.text.contains("Shop.Core ->"))
    ));
}
