//! Avalonia XAML preview through Avalonia's remote designer, as Rider, AvaloniaVS and the
//! Avalonia VS Code extension do: the IDE listens on a local TCP port and starts
//! `Avalonia.Designer.HostApp.dll` (shipped in the Avalonia NuGet package) inside the built
//! application with
//! `dotnet exec --runtimeconfig <app>.runtimeconfig.json --depsfile <app>.deps.json
//! <HostApp.dll> --transport tcp-bson://127.0.0.1:<port>/ --method avalonia-remote <app.dll>`.
//! The host connects back, the IDE sends the XAML ([`protocol::Message::UpdateXaml`]) and the
//! host answers with the result and rendered frames (BSON messages, see [`protocol`]).

pub mod bson;
pub mod protocol;

use crate::sdk::Dotnet;
use protocol::{Frame, Message, PixelFormat};
use rc_core::inbox::Outbox;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// MSBuild properties read for a preview.
const PROPERTIES: &[&str] = &["TargetPath", "ProjectRuntimeConfigFilePath", "ProjectDepsFilePath", "AvaloniaPreviewerNetCoreToolPath", "TargetFramework", "TargetFrameworks", "OutputType"];

/// What the previewer needs from the projects involved.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PreviewProject {
    /// The project that contains the XAML file.
    pub project: PathBuf,
    /// Its built assembly (where the XAML's types and resources live).
    pub assembly: PathBuf,
    /// The executable project the previewer runs in: `project` itself, or an application that
    /// references it when `project` is a library.
    pub app_project: PathBuf,
    pub app_assembly: PathBuf,
    pub runtime_config: PathBuf,
    pub deps_file: PathBuf,
    /// `Avalonia.Designer.HostApp.dll` of the app's Avalonia package.
    pub host_app: PathBuf,
}

impl PreviewProject {
    /// The files that must exist before the previewer can start.
    pub fn missing_outputs(&self) -> bool {
        ![&self.assembly, &self.app_assembly, &self.runtime_config, &self.deps_file].iter().all(|p| p.is_file())
    }

    /// The newest write time of the built assemblies (a rebuild restarts the previewer).
    pub fn outputs_stamp(&self) -> Option<std::time::SystemTime> {
        [&self.assembly, &self.app_assembly].iter().filter_map(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok()).max()
    }

    /// The XAML file's path inside its project, as the previewer wants it (`/Views/Main.axaml`).
    pub fn project_path_of(&self, xaml: &Path) -> String {
        let dir = self.project.parent().unwrap_or(Path::new(""));
        let rel = xaml.strip_prefix(dir).unwrap_or(xaml);
        let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
        format!("/{}", parts.join("/"))
    }
}

/// The framework to preview with when a project multi-targets: a plain `netX.Y` first (the
/// platform ones, `net8.0-android`, can't run on the desktop).
pub fn preview_framework(frameworks: &[String]) -> Option<&str> {
    frameworks.iter().find(|f| !f.contains('-')).or(frameworks.first()).map(String::as_str)
}

async fn properties(dotnet: &Dotnet, project: &Path) -> anyhow::Result<std::collections::BTreeMap<String, String>> {
    let global = [("Configuration", "Debug")];
    let props = crate::msbuild::get_properties(dotnet, project, PROPERTIES, &global).await?;
    let get = |k: &str| props.get(k).map(|v| v.trim().to_string()).unwrap_or_default();
    if get("TargetFramework").is_empty() && !get("TargetFrameworks").is_empty() {
        let all: Vec<String> = get("TargetFrameworks").split(';').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect();
        if let Some(tf) = preview_framework(&all) {
            let global = [("Configuration", "Debug"), ("TargetFramework", tf)];
            return crate::msbuild::get_properties(dotnet, project, PROPERTIES, &global).await;
        }
    }
    Ok(props)
}

/// Evaluates `project` (the XAML file's) and `app_project` (the executable that hosts the
/// preview; the same path for an application) for the Debug configuration.
pub async fn resolve(dotnet: &Dotnet, project: &Path, app_project: &Path) -> anyhow::Result<PreviewProject> {
    let app = properties(dotnet, app_project).await?;
    let lib = if app_project == project { app.clone() } else { properties(dotnet, project).await? };
    let path = |m: &std::collections::BTreeMap<String, String>, k: &str| PathBuf::from(m.get(k).map(|v| v.trim()).unwrap_or_default());
    let host_app = path(&app, "AvaloniaPreviewerNetCoreToolPath");
    anyhow::ensure!(!host_app.as_os_str().is_empty(), "{} does not reference Avalonia", app_project.display());
    // The props write the path with Windows separators (`build\..\tools\…`).
    let host_app = PathBuf::from(host_app.to_string_lossy().replace(if cfg!(windows) { '/' } else { '\\' }, std::path::MAIN_SEPARATOR_STR));
    Ok(PreviewProject {
        project: project.to_path_buf(),
        assembly: path(&lib, "TargetPath"),
        app_project: app_project.to_path_buf(),
        app_assembly: path(&app, "TargetPath"),
        runtime_config: path(&app, "ProjectRuntimeConfigFilePath"),
        deps_file: path(&app, "ProjectDepsFilePath"),
        host_app: dunce::simplified(&host_app).to_path_buf(),
    })
}

/// What a running previewer reports.
#[derive(Clone, Debug, PartialEq)]
pub enum HostEvent {
    /// The host connected and started its designer session.
    Connected,
    /// A rendered frame of the current XAML.
    Frame(Frame),
    /// The result of the last XAML update: `None` when it loaded.
    Result(Option<XamlError>),
    /// The host stopped (crashed, failed to start or was killed from outside). `log` holds its
    /// last output lines.
    Exited { code: Option<i32>, log: String },
}

/// Why the previewer could not show the XAML. `line` and `column` are 1-based.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct XamlError {
    pub message: String,
    pub line: Option<u32>,
    pub column: Option<u32>,
    /// The full exception text (`Exception.ToString()`).
    pub details: String,
}

impl XamlError {
    /// From an `UpdateXamlResultMessage` that carries an error.
    pub fn from_result(error: &str, exception: Option<&protocol::ExceptionDetails>) -> XamlError {
        let first = error.lines().next().unwrap_or_default().trim();
        // `Avalonia.Markup.Xaml.XamlLoadException: No precompiled XAML found…` → the message.
        let fallback = first.split_once(": ").map_or(first, |(_, m)| m).to_string();
        let aggregate = first.starts_with("System.AggregateException") || exception.is_some_and(|e| e.exception_type == "AggregateException");
        let inner = inner_messages(error);
        let message = if aggregate && !inner.is_empty() {
            // XamlX reports every problem as an inner exception (often the same one twice).
            inner.join("\n")
        } else {
            exception.map(|e| e.message.trim().to_string()).filter(|m| !m.is_empty()).unwrap_or(fallback)
        };
        let (mut line, mut column) = exception.map_or((None, None), |e| (e.line, e.column));
        if line.is_none() {
            (line, column) = position_in(message.lines().next().unwrap_or_default()).map_or((None, None), |(l, c)| (Some(l), c));
        }
        XamlError { message, line: line.filter(|&l| l > 0), column: column.filter(|&c| c > 0), details: error.to_string() }
    }
}

/// The messages of the inner exceptions in `Exception.ToString()` text, each once:
/// ` ---> XamlX.XamlTransformException: Unable to resolve type…` and
/// ` ---> (Inner Exception #1) XamlX.XamlTransformException: …<---`.
fn inner_messages(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        let Some((_, rest)) = line.split_once("---> ") else { continue };
        let rest = rest.trim_end().trim_end_matches("<---");
        let rest = match rest.strip_prefix("(Inner Exception #") {
            Some(r) => r.split_once(") ").map_or(r, |(_, m)| m),
            None => rest,
        };
        let message = rest.split_once(": ").map_or(rest, |(_, m)| m).trim().to_string();
        if !message.is_empty() && !out.contains(&message) {
            out.push(message);
        }
    }
    out
}

/// `Line 5, position 12` (XmlException, XamlX) or `(line 5 position 12)` in a message.
fn position_in(text: &str) -> Option<(u32, Option<u32>)> {
    // The last one: XmlException messages end with the position of the error.
    let lower = text.to_ascii_lowercase();
    let at = lower.rfind("line ")?;
    let rest = &lower[at + 5..];
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let line = digits.parse().ok()?;
    let after = &rest[digits.len()..];
    let column = after.find("position ").filter(|&p| p <= 3).and_then(|p| after[p + 9..].chars().take_while(char::is_ascii_digit).collect::<String>().parse().ok());
    Some((line, column))
}

/// A running `Avalonia.Designer.HostApp`. Dropping it kills the process.
pub struct DesignerHost {
    writer: Arc<Mutex<Option<TcpStream>>>,
    /// Messages sent before the host connected.
    queued: Arc<Mutex<Vec<Message>>>,
    stopped: Arc<AtomicBool>,
    pid: u32,
}

/// How long the host may take to connect (the first start JIT-compiles Avalonia and Skia).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(60);

impl DesignerHost {
    /// Starts the previewer for `project`, rendering at `scale` (physical pixels per point).
    pub fn start(dotnet: &Dotnet, project: &PreviewProject, scale: f32, outbox: Outbox<HostEvent>) -> std::io::Result<DesignerHost> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        let mut cmd = dotnet.std_command();
        cmd.arg("exec")
            .arg("--runtimeconfig")
            .arg(&project.runtime_config)
            .arg("--depsfile")
            .arg(&project.deps_file)
            .arg(&project.host_app)
            .arg("--transport")
            .arg(format!("tcp-bson://127.0.0.1:{port}/"))
            .arg("--method")
            .arg("avalonia-remote")
            .arg(&project.app_assembly)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(dir) = project.app_assembly.parent() {
            cmd.current_dir(dir);
        }
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        #[cfg(windows)]
        std::os::windows::process::CommandExt::creation_flags(&mut cmd, 0x0800_0000);
        let mut child = cmd.spawn()?;
        let pid = child.id();
        let log = Arc::new(Mutex::new(VecDeque::new()));
        for pipe in [child.stdout.take().map(|p| Box::new(p) as Box<dyn std::io::Read + Send>), child.stderr.take().map(|p| Box::new(p) as Box<dyn std::io::Read + Send>)].into_iter().flatten() {
            let log = log.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                    let mut log = log.lock().unwrap();
                    log.push_back(line);
                    if log.len() > 40 {
                        log.pop_front();
                    }
                }
            });
        }
        let host = DesignerHost { writer: Default::default(), queued: Default::default(), stopped: Default::default(), pid };
        let dpi = 96.0 * f64::from(scale.max(0.5));
        host.send(Message::ClientSupportedPixelFormats { formats: vec![PixelFormat::Rgba8888] });
        host.send(Message::ClientRenderInfo { dpi_x: dpi, dpi_y: dpi });
        let (writer, queued, stopped) = (host.writer.clone(), host.queued.clone(), host.stopped.clone());
        std::thread::Builder::new().name("avalonia-designer".into()).spawn(move || {
            serve(listener, &mut child, &writer, &queued, &stopped, &outbox);
            // Whatever ended the session, the process goes too.
            crate::process::kill_tree(child.id());
            let code = child.wait().ok().and_then(|s| s.code());
            *writer.lock().unwrap() = None;
            if !stopped.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(100)); // let the pipes drain
                let log = log.lock().unwrap().iter().cloned().collect::<Vec<_>>().join("\n");
                outbox.send(HostEvent::Exited { code, log });
            }
        })?;
        Ok(host)
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Sends a message now, or once the host has connected.
    pub fn send(&self, msg: Message) {
        let mut writer = self.writer.lock().unwrap();
        match writer.as_mut() {
            Some(stream) => {
                if stream.write_all(&msg.encode()).is_err() {
                    let _ = stream.shutdown(std::net::Shutdown::Both);
                    *writer = None;
                }
            }
            None => self.queued.lock().unwrap().push(msg),
        }
    }

    /// Shows `xaml` (the text of `file`, from the project's assembly).
    pub fn update_xaml(&self, project: &PreviewProject, file: &Path, xaml: String) {
        // Only the last update matters while the host is starting.
        self.queued.lock().unwrap().retain(|m| !matches!(m, Message::UpdateXaml { .. }));
        self.send(Message::UpdateXaml { xaml, assembly_path: project.assembly.to_string_lossy().into_owned(), xaml_file_project_path: project.project_path_of(file) });
    }

    /// The display scale changed (moved to another monitor).
    pub fn set_scale(&self, scale: f32) {
        let dpi = 96.0 * f64::from(scale.max(0.5));
        self.send(Message::ClientRenderInfo { dpi_x: dpi, dpi_y: dpi });
    }

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        if let Some(stream) = self.writer.lock().unwrap().take() {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        crate::process::kill_tree(self.pid);
    }
}

impl Drop for DesignerHost {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Accepts the host's connection and pumps its messages until it disconnects.
fn serve(listener: TcpListener, child: &mut Child, writer: &Mutex<Option<TcpStream>>, queued: &Mutex<Vec<Message>>, stopped: &AtomicBool, outbox: &Outbox<HostEvent>) {
    if listener.set_nonblocking(true).is_err() {
        return;
    }
    let started = Instant::now();
    let stream = loop {
        if stopped.load(Ordering::SeqCst) || started.elapsed() > CONNECT_TIMEOUT || child.try_wait().ok().flatten().is_some() {
            return;
        }
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return,
        }
    };
    drop(listener);
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_nodelay(true);
    let Ok(mut reader) = stream.try_clone() else { return };
    {
        let mut w = writer.lock().unwrap();
        let mut stream = stream;
        for msg in queued.lock().unwrap().drain(..) {
            if stream.write_all(&msg.encode()).is_err() {
                return;
            }
        }
        *w = Some(stream);
    }
    let mut reader = BufReader::with_capacity(1 << 16, &mut reader);
    while let Ok(Some(msg)) = Message::read(&mut reader) {
        if stopped.load(Ordering::SeqCst) {
            return;
        }
        match msg {
            Message::StartDesignerSession { .. } => {
                outbox.send(HostEvent::Connected);
            }
            Message::Frame(frame) => {
                // The host renders the next frame only after this acknowledgement.
                if let Some(w) = writer.lock().unwrap().as_mut() {
                    let _ = w.write_all(&Message::FrameReceived { sequence_id: frame.sequence_id }.encode());
                }
                outbox.send(HostEvent::Frame(frame));
            }
            Message::UpdateXamlResult { error, exception, .. } => {
                outbox.send(HostEvent::Result(error.filter(|e| !e.is_empty()).map(|e| XamlError::from_result(&e, exception.as_ref()))));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_paths_start_at_the_project_folder() {
        let p = PreviewProject { project: PathBuf::from("/src/App/App.csproj"), ..Default::default() };
        assert_eq!(p.project_path_of(Path::new("/src/App/Views/MainWindow.axaml")), "/Views/MainWindow.axaml");
        assert_eq!(p.project_path_of(Path::new("/src/App/App.axaml")), "/App.axaml");
    }

    #[test]
    fn previews_with_the_desktop_framework() {
        let fws = |s: &[&str]| s.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(preview_framework(&fws(&["net8.0-android", "net8.0", "net8.0-ios"])), Some("net8.0"));
        assert_eq!(preview_framework(&fws(&["net8.0-windows"])), Some("net8.0-windows"));
        assert_eq!(preview_framework(&[]), None);
    }

    #[test]
    fn xaml_errors_keep_the_message_and_position() {
        let details = protocol::ExceptionDetails { exception_type: "XamlLoadException".into(), message: "Unable to resolve type Buton".into(), line: Some(5), column: Some(6) };
        let e = XamlError::from_result("Avalonia.Markup.Xaml.XamlLoadException: Unable to resolve type Buton\n   at …", Some(&details));
        assert_eq!((e.message.as_str(), e.line, e.column), ("Unable to resolve type Buton", Some(5), Some(6)));
        // Without details: the first line, the (last) position from the text.
        let e = XamlError::from_result("System.Xml.XmlException: The 'Button' start tag on line 4 position 6 does not match the end tag of 'StackPanel'. Line 9, position 5.\n   at …", None);
        assert_eq!(e.message, "The 'Button' start tag on line 4 position 6 does not match the end tag of 'StackPanel'. Line 9, position 5.");
        assert_eq!((e.line, e.column), (Some(9), Some(5)));
        let aggregate = "System.AggregateException: One or more errors occurred. (Unable to resolve type TextBlok Line 2, position 4.) (Unable to resolve type TextBlok Line 2, position 4.)\n ---> XamlX.XamlTransformException: Unable to resolve type TextBlok Line 2, position 4.\n   at XamlX.Transform…\n   --- End of inner exception stack trace ---\n ---> (Inner Exception #1) XamlX.XamlTransformException: Unable to resolve type TextBlok Line 2, position 4.<---\n\n ---> (Inner Exception #2) XamlX.XamlTransformException: Unable to resolve suitable regular or attached property Text on type :.{Unknown type} Line 2, position 4.<---\n";
        let e = XamlError::from_result(aggregate, Some(&protocol::ExceptionDetails { exception_type: "AggregateException".into(), message: "One or more errors occurred.".into(), line: None, column: None }));
        assert_eq!(e.message, "Unable to resolve type TextBlok Line 2, position 4.\nUnable to resolve suitable regular or attached property Text on type :.{Unknown type} Line 2, position 4.");
        assert_eq!((e.line, e.column), (Some(2), Some(4)));
        let e = XamlError::from_result("System.InvalidOperationException: No entry point", Some(&protocol::ExceptionDetails { line: Some(0), column: Some(0), ..Default::default() }));
        assert_eq!((e.message.as_str(), e.line, e.column), ("No entry point", None, None));
    }
}
