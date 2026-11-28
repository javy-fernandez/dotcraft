//! The Avalonia previewer end to end: the Avalonia 11 app of `tests/fixtures/AvaloniaOrders`
//! is built in a temporary folder, `Avalonia.Designer.HostApp` is started in it and renders
//! XAML sent over the remote designer protocol.

use rc_core::{Inbox, Waker};
use rc_dotnet::Dotnet;
use rc_dotnet::avalonia_preview::protocol::Frame;
use rc_dotnet::avalonia_preview::{self, DesignerHost, HostEvent};
use std::path::Path;
use std::time::{Duration, Instant};

/// A window with a solid background, to check the pixels.
const WINDOW: &str = r##"<Window xmlns="https://github.com/avaloniaui" xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml" x:Class="Orders.Views.MainWindow" Width="320" Height="200" Background="#FF0000">
  <TextBlock Text="Hello" />
</Window>
"##;

/// A copy of `tests/fixtures/AvaloniaOrders` (built there, so the fixture stays clean).
fn copy_fixture(to: &Path) {
    let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/AvaloniaOrders");
    let mut stack = vec![(from, to.to_path_buf())];
    while let Some((from, to)) = stack.pop() {
        std::fs::create_dir_all(&to).unwrap();
        for entry in std::fs::read_dir(&from).unwrap().map(Result::unwrap) {
            let name = entry.file_name();
            if name == "bin" || name == "obj" {
                continue;
            }
            if entry.path().is_dir() {
                stack.push((entry.path(), to.join(&name)));
            } else {
                std::fs::copy(entry.path(), to.join(&name)).unwrap();
            }
        }
    }
}

/// Waits for the next frame and takes it.
fn frame(host: &DesignerHost, inbox: &Inbox<HostEvent>, until: Instant, want: impl Fn(&Frame) -> bool) -> Frame {
    loop {
        next(inbox, until, |e| matches!(e, HostEvent::FrameReady));
        if let Some(f) = host.take_frame().filter(|f| want(f)) {
            return f;
        }
    }
}

fn next(inbox: &Inbox<HostEvent>, until: Instant, mut want: impl FnMut(&HostEvent) -> bool) -> HostEvent {
    loop {
        let left = until.saturating_duration_since(Instant::now());
        let ev = inbox.recv_timeout(left).expect("the previewer did not answer in time");
        if let HostEvent::Exited { log, code } = &ev {
            panic!("the previewer exited ({code:?}):\n{log}");
        }
        if want(&ev) {
            return ev;
        }
    }
}

#[test]
#[ignore = "Requires the .NET SDK and the Avalonia packages (restored from nuget.org)"]
fn renders_avalonia_xaml_and_reports_errors() {
    let dotnet = Dotnet::locate(None).expect("the dotnet SDK must be installed");
    let tmp = tempfile::Builder::new().prefix("dotcraft-avalonia").tempdir().unwrap();
    let dir = dunce::canonicalize(tmp.path()).unwrap();
    copy_fixture(&dir);
    let csproj = dir.join("Orders.csproj");
    let out = dotnet.std_command().arg("build").arg(&csproj).arg("-nologo").output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stdout));

    let project = rc_core::Runtime::block_on(avalonia_preview::resolve(&dotnet, &csproj, &csproj)).unwrap();
    assert!(project.host_app.is_file(), "{}", project.host_app.display());
    assert!(!project.missing_outputs());
    let main = dir.join("Views/MainWindow.axaml");
    assert_eq!(project.project_path_of(&main), "/Views/MainWindow.axaml");

    let inbox = Inbox::new(Waker::noop());
    let host = DesignerHost::start(&dotnet, &project, 2.0, inbox.sender()).unwrap();
    host.update_xaml(&project, &main, WINDOW.to_string());
    let until = Instant::now() + Duration::from_secs(90);
    next(&inbox, until, |e| matches!(e, HostEvent::Connected));
    assert_eq!(next(&inbox, until, |e| matches!(e, HostEvent::Result(_))), HostEvent::Result(None));
    // 320×200 at 200% (the first frame may still be at 100%), with the red background.
    let shown = frame(&host, &inbox, until, |f| f.dpi_x > 100.0);
    assert_eq!((shown.width, shown.height), (640, 400));
    let rgba = shown.to_rgba();
    assert_eq!(&rgba[rgba.len() - 4..], &[255, 0, 0, 255]);

    // A live edit: a new background, then a typo the previewer reports with its position.
    host.update_xaml(&project, &main, WINDOW.replace("#FF0000", "#0000FF"));
    assert_eq!(next(&inbox, until, |e| matches!(e, HostEvent::Result(_))), HostEvent::Result(None));
    let rgba = frame(&host, &inbox, until, |f| f.width > 0).to_rgba();
    assert_eq!(&rgba[rgba.len() - 4..], &[0, 0, 255, 255]);
    host.update_xaml(&project, &main, WINDOW.replace("<TextBlock", "<TextBlok"));
    let HostEvent::Result(Some(error)) = next(&inbox, until, |e| matches!(e, HostEvent::Result(_))) else { panic!("the typo was accepted") };
    // The inner errors of XamlX's AggregateException, each once.
    assert_eq!(error.message.lines().next(), Some("Unable to resolve type TextBlok from namespace https://github.com/avaloniaui"), "{error:?}");
    assert_eq!(error.message.lines().count(), 2, "{error:?}");
    assert_eq!((error.line, error.column), (Some(2), Some(4)), "{error:?}");

    // Dropping the host kills the process.
    let pid = host.pid();
    drop(host);
    std::thread::sleep(Duration::from_millis(500));
    #[cfg(unix)]
    // SAFETY: signal 0 only checks that the process exists.
    assert_ne!(unsafe { libc::kill(pid as i32, 0) }, 0, "the previewer is still running");
}
