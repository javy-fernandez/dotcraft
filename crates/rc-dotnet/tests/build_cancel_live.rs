//! Live Linux SDK acceptance for build cancellation. Run with
//! `DOTCRAFT_TEST_DOTNET=/path/to/dotnet cargo test -p rc-dotnet --test build_cancel_live -- --ignored`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rc_core::{Inbox, Waker};
use rc_dotnet::Dotnet;
use rc_dotnet::build::{BuildAction, BuildEvent, BuildRequest, start_build};

fn copy_shop_core_with_slow_target(to: &Path) -> PathBuf {
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
    let project = to.join("Shop.Core.csproj");
    let source = std::fs::read_to_string(&project).unwrap();
    let with_probe = source.replace(
        "</Project>",
        "  <Target Name=\"DotCraftCancelProbe\" BeforeTargets=\"CoreCompile\">\n    <Exec Command=\"sh -c 'sleep 30 &amp; echo DOTCRAFT_CANCEL_CHILD_STARTED; wait'\" />\n    <Message Importance=\"high\" Text=\"DOTCRAFT_CANCEL_END\" />\n  </Target>\n</Project>",
    );
    assert_ne!(source, with_probe);
    std::fs::write(&project, with_probe).unwrap();
    project
}

#[test]
#[cfg(target_os = "linux")]
#[ignore = "requires a locally installed .NET 8 SDK; set DOTCRAFT_TEST_DOTNET"]
fn cancels_a_running_shop_build_before_compilation() {
    let host = std::env::var_os("DOTCRAFT_TEST_DOTNET")
        .expect("set DOTCRAFT_TEST_DOTNET to a .NET 8 dotnet host");
    let dotnet = Dotnet::new(host);
    assert!(dotnet.version(None).unwrap().starts_with("8."));

    let dir = tempfile::tempdir().unwrap();
    let project = copy_shop_core_with_slow_target(&dir.path().join("Shop.Core"));
    let dll = project
        .parent()
        .unwrap()
        .join("bin/Debug/net8.0/Shop.Core.dll");
    let inbox = Inbox::new(Waker::noop());
    let handle = start_build(
        &dotnet,
        BuildRequest::new(&project, BuildAction::Build),
        inbox.sender(),
    );

    let deadline = Instant::now() + Duration::from_secs(60);
    let mut events = Vec::new();
    let mut cancelled_at = None;
    while Instant::now() < deadline {
        let Some(event) = inbox.recv_timeout(Duration::from_millis(200)) else {
            continue;
        };
        if matches!(&event, BuildEvent::Output(line) if line.text.contains("DOTCRAFT_CANCEL_CHILD_STARTED"))
        {
            assert!(!handle.is_finished());
            handle.cancel();
            cancelled_at = Some(Instant::now());
        }
        let done = matches!(event, BuildEvent::Finished(_) | BuildEvent::Failed(_));
        events.push(event);
        if done {
            break;
        }
    }

    let cancelled_at = cancelled_at.expect("the SDK did not reach the cancellation target");
    assert!(
        cancelled_at.elapsed() < Duration::from_secs(10),
        "cancellation took too long: {events:?}"
    );
    assert!(
        matches!(&events[0], BuildEvent::Started { command } if command.contains("dotnet build"))
    );
    assert!(!events.iter().any(
        |e| matches!(e, BuildEvent::Output(line) if line.text.contains("DOTCRAFT_CANCEL_END"))
    ));
    assert!(
        matches!(events.last(), Some(BuildEvent::Finished(result)) if result.cancelled && !result.success && result.exit_code != Some(0))
    );
    assert!(
        !dll.exists(),
        "cancelled build unexpectedly compiled the project"
    );

    let original = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/Shop/src/Shop.Core/Shop.Core.csproj");
    std::fs::copy(original, &project).unwrap();
    let recovery_inbox = Inbox::new(Waker::noop());
    let _recovery = start_build(
        &dotnet,
        BuildRequest::new(&project, BuildAction::Build),
        recovery_inbox.sender(),
    );
    let recovery_deadline = Instant::now() + Duration::from_secs(60);
    let mut final_event = None;
    while Instant::now() < recovery_deadline {
        if let Some(event) = recovery_inbox.recv_timeout(Duration::from_millis(200))
            && matches!(event, BuildEvent::Finished(_) | BuildEvent::Failed(_))
        {
            final_event = Some(event);
            break;
        }
    }
    assert!(matches!(final_event, Some(BuildEvent::Finished(result)) if result.success));
    assert!(
        dll.is_file(),
        "a fresh build after cancellation did not produce the DLL"
    );
}
