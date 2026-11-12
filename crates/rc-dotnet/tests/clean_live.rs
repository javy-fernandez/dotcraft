//! Live SDK acceptance for the clean command. Run with
//! `DOTCRAFT_TEST_DOTNET=/path/to/dotnet cargo test -p rc-dotnet --test clean_live -- --ignored`.

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
async fn cleans_real_shop_project_outputs_and_allows_build_again() {
    let host = std::env::var_os("DOTCRAFT_TEST_DOTNET")
        .expect("set DOTCRAFT_TEST_DOTNET to a .NET 8 dotnet host");
    let dotnet = Dotnet::new(host);
    assert!(dotnet.version(None).unwrap().starts_with("8."));

    let dir = tempfile::tempdir().unwrap();
    let project = copy_shop_core(&dir.path().join("Shop.Core"));
    let root = project.parent().unwrap();
    let dll = root.join("bin/Debug/net8.0/Shop.Core.dll");
    let intermediate = root.join("obj/Debug/net8.0/Shop.Core.dll");
    let release_dll = root.join("bin/Release/net8.0/Shop.Core.dll");
    let release_intermediate = root.join("obj/Release/net8.0/Shop.Core.dll");

    let (initial, _) = execute(&dotnet, &BuildRequest::new(&project, BuildAction::Build)).await;
    assert!(initial.success, "initial build: {initial:?}");
    assert!(dll.is_file() && intermediate.is_file());

    let mut release_build = BuildRequest::new(&project, BuildAction::Build);
    release_build.configuration = "Release".into();
    release_build.no_restore = true;
    let (release_result, _) = execute(&dotnet, &release_build).await;
    assert!(release_result.success, "Release build: {release_result:?}");
    assert!(release_dll.is_file() && release_intermediate.is_file());

    let (clean, events) = execute(&dotnet, &BuildRequest::new(&project, BuildAction::Clean)).await;
    assert!(
        clean.success && !clean.cancelled && clean.exit_code == Some(0),
        "{clean:?}"
    );
    assert_eq!(clean.errors, 0);
    assert!(!dll.exists() && !intermediate.exists());
    assert!(release_dll.is_file() && release_intermediate.is_file());
    assert!(
        matches!(&events[0], BuildEvent::Started { command } if command.contains("dotnet clean") && command.contains("-c Debug"))
    );

    let mut clean_release = BuildRequest::new(&project, BuildAction::Clean);
    clean_release.configuration = "Release".into();
    let (release_clean, release_events) = execute(&dotnet, &clean_release).await;
    assert!(release_clean.success, "Release clean: {release_clean:?}");
    assert!(!release_dll.exists() && !release_intermediate.exists());
    assert!(
        matches!(&release_events[0], BuildEvent::Started { command } if command.contains("dotnet clean") && command.contains("-c Release"))
    );

    let mut build_again = BuildRequest::new(&project, BuildAction::Build);
    build_again.no_restore = true;
    let (restored, _) = execute(&dotnet, &build_again).await;
    assert!(restored.success, "build after clean: {restored:?}");
    assert!(dll.is_file() && intermediate.is_file());
}
