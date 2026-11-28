//! The Avalonia previewer end to end: a minimal Avalonia 11 app is built in a temporary
//! folder, `Avalonia.Designer.HostApp` is started in it and renders XAML sent over the
//! remote designer protocol.

use rc_core::{Inbox, Waker};
use rc_dotnet::Dotnet;
use rc_dotnet::avalonia_preview::{self, DesignerHost, HostEvent};
use std::path::Path;
use std::time::{Duration, Instant};

pub const PROJECT: &str = r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <OutputType>WinExe</OutputType>
    <TargetFramework>net8.0</TargetFramework>
    <Nullable>enable</Nullable>
  </PropertyGroup>
  <ItemGroup>
    <PackageReference Include="Avalonia" Version="11.3.23" />
    <PackageReference Include="Avalonia.Desktop" Version="11.3.23" />
    <PackageReference Include="Avalonia.Themes.Fluent" Version="11.3.23" />
  </ItemGroup>
</Project>
"#;

const FILES: &[(&str, &str)] = &[
    (
        "App.axaml",
        r#"<Application xmlns="https://github.com/avaloniaui" xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml" x:Class="Previewed.App" RequestedThemeVariant="Light">
  <Application.Styles>
    <FluentTheme />
  </Application.Styles>
</Application>
"#,
    ),
    (
        "App.axaml.cs",
        "using Avalonia;\nusing Avalonia.Controls.ApplicationLifetimes;\nusing Avalonia.Markup.Xaml;\nnamespace Previewed;\npublic partial class App : Application\n{\n    public override void Initialize() => AvaloniaXamlLoader.Load(this);\n    public override void OnFrameworkInitializationCompleted()\n    {\n        if (ApplicationLifetime is IClassicDesktopStyleApplicationLifetime desktop)\n            desktop.MainWindow = new MainWindow();\n        base.OnFrameworkInitializationCompleted();\n    }\n}\n",
    ),
    (
        "MainWindow.axaml",
        r##"<Window xmlns="https://github.com/avaloniaui" xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml" x:Class="Previewed.MainWindow" Title="Previewed" Width="320" Height="200" Background="#FF0000">
  <TextBlock Text="Hello" />
</Window>
"##,
    ),
    ("MainWindow.axaml.cs", "using Avalonia.Controls;\nnamespace Previewed;\npublic partial class MainWindow : Window\n{\n    public MainWindow() => InitializeComponent();\n}\n"),
    (
        "Program.cs",
        "using Avalonia;\nnamespace Previewed;\nclass Program\n{\n    [System.STAThread]\n    public static void Main(string[] args) => BuildAvaloniaApp().StartWithClassicDesktopLifetime(args);\n    public static AppBuilder BuildAvaloniaApp() => AppBuilder.Configure<App>().UsePlatformDetect();\n}\n",
    ),
];

fn write_app(dir: &Path) {
    std::fs::write(dir.join("Previewed.csproj"), PROJECT).unwrap();
    for (name, text) in FILES {
        std::fs::write(dir.join(name), text).unwrap();
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
    write_app(&dir);
    let csproj = dir.join("Previewed.csproj");
    let out = dotnet.std_command().arg("build").arg(&csproj).arg("-nologo").output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stdout));

    let project = rc_core::Runtime::block_on(avalonia_preview::resolve(&dotnet, &csproj, &csproj)).unwrap();
    assert!(project.host_app.is_file(), "{}", project.host_app.display());
    assert!(!project.missing_outputs());
    assert_eq!(project.project_path_of(&dir.join("MainWindow.axaml")), "/MainWindow.axaml");

    let inbox = Inbox::new(Waker::noop());
    let host = DesignerHost::start(&dotnet, &project, 2.0, inbox.sender()).unwrap();
    let main = dir.join("MainWindow.axaml");
    host.update_xaml(&project, &main, std::fs::read_to_string(&main).unwrap());
    let until = Instant::now() + Duration::from_secs(90);
    next(&inbox, until, |e| matches!(e, HostEvent::Connected));
    assert_eq!(next(&inbox, until, |e| matches!(e, HostEvent::Result(_))), HostEvent::Result(None));
    let HostEvent::Frame(frame) = next(&inbox, until, |e| matches!(e, HostEvent::Frame(f) if f.width > 0 && f.dpi_x > 100.0)) else { unreachable!() };
    // 320×200 at 200% (the first frame may still be at 100%), with the red background.
    assert_eq!((frame.width, frame.height), (640, 400));
    assert_eq!(frame.dpi_x, 192.0);
    let rgba = frame.to_rgba();
    assert_eq!(&rgba[rgba.len() - 4..], &[255, 0, 0, 255]);

    // A live edit: a new background, then a typo the previewer reports with its position.
    host.update_xaml(&project, &main, FILES[2].1.replace("#FF0000", "#0000FF"));
    assert_eq!(next(&inbox, until, |e| matches!(e, HostEvent::Result(_))), HostEvent::Result(None));
    let HostEvent::Frame(frame) = next(&inbox, until, |e| matches!(e, HostEvent::Frame(f) if f.width > 0)) else { unreachable!() };
    let rgba = frame.to_rgba();
    assert_eq!(&rgba[rgba.len() - 4..], &[0, 0, 255, 255]);
    host.update_xaml(&project, &main, FILES[2].1.replace("<TextBlock", "<TextBlok"));
    let HostEvent::Result(Some(error)) = next(&inbox, until, |e| matches!(e, HostEvent::Result(_))) else { panic!("the typo was accepted") };
    // The inner errors of XamlX's AggregateException, each once.
    assert_eq!(error.message.lines().next(), Some("Unable to resolve type TextBlok from namespace https://github.com/avaloniaui Line 2, position 4."), "{error:?}");
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
