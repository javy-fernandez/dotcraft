//! Live Linux SDK acceptance for build output streaming. Run with
//! `DOTCRAFT_TEST_DOTNET=/path/to/dotnet cargo test -p rc-dotnet --test build_stream_live -- --ignored`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rc_core::{Inbox, Waker};
use rc_dotnet::build::{BuildAction, BuildEvent, BuildRequest, start_build};
use rc_dotnet::{Dotnet, Stream};

fn copy_shop_core_with_probe(to: &Path) -> PathBuf {
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
        "  <Target Name=\"DotCraftStreamProbe\" BeforeTargets=\"CoreCompile\">\n    <Message Importance=\"high\" Text=\"DOTCRAFT_STREAM_BEGIN\" />\n    <Exec Command=\"sleep 2\" />\n    <Message Importance=\"high\" Text=\"DOTCRAFT_STREAM_END\" />\n  </Target>\n</Project>",
    );
    assert_ne!(source, with_probe);
    std::fs::write(&project, with_probe).unwrap();
    project
}

#[test]
#[cfg(target_os = "linux")]
#[ignore = "requires a locally installed .NET 8 SDK; set DOTCRAFT_TEST_DOTNET"]
fn emits_build_lines_before_the_sdk_process_exits() {
    let host = std::env::var_os("DOTCRAFT_TEST_DOTNET")
        .expect("set DOTCRAFT_TEST_DOTNET to a .NET 8 dotnet host");
    let dotnet = Dotnet::new(host);
    assert!(dotnet.version(None).unwrap().starts_with("8."));

    let dir = tempfile::tempdir().unwrap();
    let project = copy_shop_core_with_probe(&dir.path().join("Shop.Core"));
    let inbox = Inbox::new(Waker::noop());
    let handle = start_build(
        &dotnet,
        BuildRequest::new(&project, BuildAction::Build),
        inbox.sender(),
    );

    let deadline = Instant::now() + Duration::from_secs(60);
    let mut events = Vec::new();
    let mut saw_begin_while_running = false;
    while Instant::now() < deadline {
        let Some(event) = inbox.recv_timeout(Duration::from_millis(200)) else {
            continue;
        };
        if let BuildEvent::Output(line) = &event {
            if line.text.contains("DOTCRAFT_STREAM_BEGIN") {
                assert_eq!(line.stream, Stream::Stdout);
                assert!(line.newline);
                assert!(
                    !handle.is_finished(),
                    "begin marker arrived after process completion"
                );
                saw_begin_while_running = true;
            }
        }
        let done = matches!(event, BuildEvent::Finished(_) | BuildEvent::Failed(_));
        events.push(event);
        if done {
            break;
        }
    }

    assert!(
        saw_begin_while_running,
        "no output while the SDK was running: {events:?}"
    );
    assert!(
        matches!(&events[0], BuildEvent::Started { command } if command.contains("dotnet build"))
    );
    let begin = events.iter().position(|e| matches!(e, BuildEvent::Output(line) if line.text.contains("DOTCRAFT_STREAM_BEGIN"))).unwrap();
    let end = events
        .iter()
        .position(
            |e| matches!(e, BuildEvent::Output(line) if line.text.contains("DOTCRAFT_STREAM_END")),
        )
        .unwrap();
    assert!(begin < end);
    assert!(
        matches!(events.last(), Some(BuildEvent::Finished(result)) if result.success && result.exit_code == Some(0))
    );
}
