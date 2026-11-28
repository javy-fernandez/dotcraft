//! XAML preview (Rider): `.axaml` files, and `.xaml` files of Avalonia projects, show the
//! rendered view beside the editor and follow it as you type. Avalonia's own previewer
//! (`rc_dotnet::avalonia_preview`) renders the XAML inside the built application, one host
//! process per previewed file; it is started when the preview is first shown, restarted after
//! a build or a crash, stopped after a while out of sight and killed when the file closes.
//!
//! .NET MAUI, WPF and WinUI have no design-time previewer outside Visual Studio's (Windows)
//! designers, so their XAML files show a note instead of a picture.

use crate::docs::{Doc, Docs};
use rc_core::Inbox;
use rc_dotnet::Dotnet;
use rc_dotnet::avalonia_preview::{DesignerHost, HostEvent, PreviewProject, XamlError};
use rc_kit::i18n::{tr, trf};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

/// Typing pauses this long before the previewer gets the new text.
const DEBOUNCE: Duration = Duration::from_millis(300);
/// A preview out of sight this long stops its host (it starts again when shown).
const IDLE_AFTER: Duration = Duration::from_secs(120);
/// Crashes tolerated in a minute before the preview gives up and offers Restart.
const MAX_RESTARTS: usize = 3;

/// Which UI framework a XAML file belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XamlKind {
    Avalonia,
    Maui,
    Wpf,
    WinUi,
    Unknown,
}

/// `.axaml` and `.xaml` files open beside their preview.
pub fn is_xaml(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("axaml") || e.eq_ignore_ascii_case("xaml"))
}

/// The framework of a XAML file, from its extension and its root element's default namespace.
pub fn kind(path: &Path, text: &str) -> XamlKind {
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("axaml")) {
        return XamlKind::Avalonia;
    }
    match root_namespace(text) {
        Some("https://github.com/avaloniaui") => XamlKind::Avalonia,
        Some("http://xamarin.com/schemas/2014/forms") => XamlKind::Maui,
        Some(ns) if ns.starts_with("http://schemas.microsoft.com/dotnet/") && ns.ends_with("/maui") => XamlKind::Maui,
        // WPF and WinUI share the namespace; the project tells them apart.
        Some("http://schemas.microsoft.com/winfx/2006/xaml/presentation") => {
            let winui = project_of(path).and_then(|p| std::fs::read_to_string(p).ok()).is_some_and(|t| {
                let t = t.to_ascii_lowercase();
                t.contains("<usewinui>true") || t.contains("microsoft.windowsappsdk")
            });
            if winui { XamlKind::WinUi } else { XamlKind::Wpf }
        }
        _ => XamlKind::Unknown,
    }
}

/// The `xmlns="…"` of the first element (comments and the XML declaration skipped).
fn root_namespace(text: &str) -> Option<&str> {
    let mut rest = text;
    loop {
        let at = rest.find('<')?;
        rest = &rest[at..];
        if rest.starts_with("<!--") {
            rest = &rest[rest.find("-->")? + 3..];
            continue;
        }
        if rest.starts_with("<?") || rest.starts_with("<!") {
            rest = &rest[rest.find('>')? + 1..];
            continue;
        }
        let tag = &rest[..rest.find('>').unwrap_or(rest.len())];
        let mut search = tag;
        while let Some(i) = search.find("xmlns") {
            let after = &search[i + 5..];
            let value = after.trim_start().strip_prefix('=').map(str::trim_start);
            if let Some(v) = value.and_then(|v| v.strip_prefix('"').or_else(|| v.strip_prefix('\''))) {
                return v.find(['"', '\'']).map(|end| &v[..end]);
            }
            search = after;
        }
        return None;
    }
}

/// The project file in the nearest folder above `file` that has one.
pub fn project_of(file: &Path) -> Option<PathBuf> {
    file.ancestors().skip(1).find_map(|dir| {
        let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
            .ok()?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e, "csproj" | "fsproj" | "vbproj")))
            .collect();
        found.sort();
        // `Views/App.csproj` next to `App.Tests.csproj`: the one named like the folder first.
        let named = found.iter().position(|p| p.file_stem() == dir.file_name());
        named.map(|i| found.swap_remove(i)).or_else(|| found.into_iter().next())
    })
}

/// The application the preview of `project`'s XAML runs in: the project itself when it is an
/// executable, else the first of `executables` that references it (directly or not).
pub fn app_project_for(project: &Path, executables: &[PathBuf]) -> Option<PathBuf> {
    let info = rc_dotnet::ProjectInfo::load(project).ok()?;
    if info.is_executable() {
        return Some(project.to_path_buf());
    }
    executables.iter().find(|exe| references(exe, project)).cloned()
}

fn references(from: &Path, to: &Path) -> bool {
    let same = |a: &Path, b: &Path| dunce::canonicalize(a).ok().zip(dunce::canonicalize(b).ok()).is_some_and(|(a, b)| a == b) || a == b;
    let mut seen = Vec::new();
    let mut stack = vec![from.to_path_buf()];
    while let Some(p) = stack.pop() {
        if seen.iter().any(|s: &PathBuf| same(s, &p)) || seen.len() > 200 {
            continue;
        }
        let Ok(info) = rc_dotnet::ProjectInfo::load(&p) else { continue };
        if info.project_refs.iter().any(|r| same(r, to)) {
            return true;
        }
        stack.extend(info.project_refs);
        seen.push(p);
    }
    false
}

/// How big the rendered view is shown.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Zoom {
    /// As large as fits the pane, never above 100%.
    Fit,
    Scale(f32),
}

const STEPS: [f32; 11] = [0.25, 0.33, 0.5, 0.67, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0];

pub fn zoom_in(current: f32) -> f32 {
    STEPS.iter().copied().find(|s| *s > current + 0.005).unwrap_or(STEPS[STEPS.len() - 1])
}

pub fn zoom_out(current: f32) -> f32 {
    STEPS.iter().rev().copied().find(|s| *s < current - 0.005).unwrap_or(STEPS[0])
}

/// The scale that shows `frame` (points) whole in `area`, at most 100%.
pub fn fit_scale(frame: egui::Vec2, area: egui::Vec2) -> f32 {
    if frame.x <= 0.0 || frame.y <= 0.0 {
        return 1.0;
    }
    (area.x / frame.x).min(area.y / frame.y).clamp(0.05, 1.0)
}

/// Where a preview is.
pub enum Stage {
    /// Not an Avalonia file: the pane explains why there is no preview.
    Unsupported(XamlKind),
    /// Finding the project and evaluating it with MSBuild.
    Resolving(Inbox<Result<PreviewProject, String>>),
    /// No preview possible (no project, no Avalonia, MSBuild failed): the reason.
    Unavailable(String),
    /// The application has not been built.
    NeedsBuild(PreviewProject),
    Running(Box<Running>),
    /// The host stopped too often (or could not start): why, and its last output.
    Stopped { project: PreviewProject, message: String, log: String },
    /// Out of sight for a while: the host was stopped; it starts again when shown.
    Idle(PreviewProject),
}

pub struct Running {
    pub project: PreviewProject,
    pub host: DesignerHost,
    inbox: Inbox<HostEvent>,
    /// The built assemblies' time when the host started (a newer build restarts it).
    stamp: Option<SystemTime>,
    pub connected: bool,
    scale: f32,
}

/// The preview of one document.
pub struct Preview {
    pub kind: XamlKind,
    pub stage: Stage,
    pub texture: Option<egui::TextureHandle>,
    /// The rendered view's size in points.
    pub frame_size: egui::Vec2,
    /// Why the current text could not be shown (the last good frame stays).
    pub error: Option<XamlError>,
    pub zoom: Zoom,
    /// The scale the view was last shown at (Fit resolved).
    pub shown_scale: f32,
    /// Code changed since the last build: the preview may miss new types or properties.
    pub stale: bool,
    /// The buffer version last sent, and when the text changed after it.
    sent: Option<u64>,
    changed: Option<Instant>,
    /// An update was sent and its result has not come back yet.
    pub updating: bool,
    restarts: Vec<Instant>,
    polled: Instant,
    stale_check: Option<(mpsc::Receiver<bool>, Instant)>,
    shown: Instant,
}

impl Preview {
    fn new(kind: XamlKind) -> Self {
        let now = Instant::now();
        Preview {
            kind,
            stage: Stage::Unsupported(kind),
            texture: None,
            frame_size: egui::Vec2::ZERO,
            error: None,
            zoom: Zoom::Fit,
            shown_scale: 1.0,
            stale: false,
            sent: None,
            changed: None,
            updating: false,
            restarts: Vec::new(),
            polled: now,
            stale_check: None,
            shown: now,
        }
    }

    pub fn project(&self) -> Option<&PreviewProject> {
        match &self.stage {
            Stage::NeedsBuild(p) | Stage::Idle(p) | Stage::Stopped { project: p, .. } => Some(p),
            Stage::Running(r) => Some(&r.project),
            _ => None,
        }
    }
}

/// What the previews need from the rest of the IDE each frame.
pub struct Env {
    pub dotnet: Option<Dotnet>,
    /// Executable projects of the workspace (hosts for XAML in libraries).
    pub executables: Vec<PathBuf>,
    /// A build is running (outputs are being written).
    pub building: bool,
    /// Physical pixels per point.
    pub scale: f32,
}

#[derive(Default)]
pub struct XamlPreviews {
    pub map: HashMap<u64, Preview>,
    /// The previewer's error published per file (editor diagnostics and Problems).
    published: HashMap<PathBuf, rc_editor::Diagnostic>,
}

impl XamlPreviews {
    /// Once per frame for a shown preview: starts, feeds and drains its previewer.
    pub fn update(&mut self, ctx: &egui::Context, doc: &Doc, env: &Env) {
        let id = doc.editor.id;
        let version = doc.editor.buffer.version();
        let p = self.map.entry(id).or_insert_with(|| Preview::new(XamlKind::Unknown));
        p.shown = Instant::now();
        // The framework can change while typing the root element of a `.xaml` file.
        if p.sent != Some(version) && matches!(p.stage, Stage::Unsupported(_)) {
            let k = kind(&doc.path, &doc.editor.buffer.rope().to_string());
            p.kind = k;
            p.stage = if k == XamlKind::Avalonia { resolve(&doc.path, env, ctx) } else { Stage::Unsupported(k) };
            if k != XamlKind::Avalonia {
                p.sent = Some(version);
                return;
            }
        }
        match &mut p.stage {
            Stage::Unsupported(_) | Stage::Unavailable(_) | Stage::Stopped { .. } => return,
            Stage::Resolving(inbox) => {
                let Some(result) = inbox.drain().pop() else { return };
                p.stage = match result {
                    // Avalonia's props (and the previewer's path) only exist once restored.
                    Ok(project) if !project.references_avalonia() && !restored(&project.app_project) => Stage::NeedsBuild(project),
                    Ok(project) if !project.references_avalonia() => Stage::Unavailable(trf("{project} does not reference Avalonia.", &[("project", &file_name(&project.app_project))])),
                    Ok(project) => Stage::NeedsBuild(project),
                    Err(e) => Stage::Unavailable(e),
                };
                p.polled = Instant::now() - Duration::from_secs(5);
            }
            _ => {}
        }
        // Start when the build is there; restart after a newer build.
        if p.polled.elapsed() >= Duration::from_secs(1) {
            p.polled = Instant::now();
            let restart = match &p.stage {
                Stage::NeedsBuild(project) => !env.building && !project.missing_outputs(),
                Stage::Idle(_) => true,
                Stage::Running(r) => !env.building && r.project.outputs_stamp() != r.stamp,
                _ => false,
            };
            if restart && let Some(project) = p.project().cloned() {
                if !project.references_avalonia() {
                    // Built for the first time: evaluate again, now with the packages restored.
                    p.stage = resolve(&doc.path, env, ctx);
                } else if project.missing_outputs() {
                    p.stage = Stage::NeedsBuild(project);
                } else {
                    start(p, project, env, ctx);
                }
            }
            check_stale(p);
        }
        ctx.request_repaint_after(Duration::from_secs(1));
        let Stage::Running(run) = &mut p.stage else { return };
        // Messages from the host.
        let mut exited = None;
        for ev in run.inbox.drain() {
            match ev {
                HostEvent::Connected => run.connected = true,
                HostEvent::FrameReady => {
                    if let Some(frame) = run.host.take_frame().filter(|f| f.width > 0 && f.height > 0) {
                        let image = egui::ColorImage::from_rgba_premultiplied([frame.width, frame.height], &frame.to_rgba());
                        match &mut p.texture {
                            Some(t) => t.set(image, egui::TextureOptions::LINEAR),
                            None => p.texture = Some(ctx.load_texture(format!("xaml-preview-{id}"), image, egui::TextureOptions::LINEAR)),
                        }
                        let ratio = (frame.dpi_x / 96.0).max(0.1) as f32;
                        p.frame_size = egui::vec2(frame.width as f32 / ratio, frame.height as f32 / ratio);
                    }
                }
                HostEvent::Result(error) => {
                    p.error = error;
                    p.updating = false;
                }
                HostEvent::Exited { code, log } => exited = Some((code, log)),
            }
        }
        if let Some((code, log)) = exited {
            let project = run.project.clone();
            p.restarts.retain(|t| t.elapsed() < Duration::from_secs(60));
            if p.restarts.len() < MAX_RESTARTS && !project.missing_outputs() {
                p.restarts.push(Instant::now());
                start(p, project, env, ctx);
            } else {
                let message = match code {
                    Some(code) => trf("The previewer stopped (exit code {code}).", &[("code", &code)]),
                    None => tr("The previewer stopped.").to_string(),
                };
                p.stage = Stage::Stopped { project, message, log };
                p.updating = false;
            }
            return;
        }
        if (run.scale - env.scale).abs() > 0.01 {
            run.scale = env.scale;
            run.host.set_scale(env.scale);
        }
        // The text, once typing pauses.
        if p.sent != Some(version) {
            let due = match (p.sent, p.changed) {
                (None, _) => Instant::now(),
                (_, Some(t)) => t + DEBOUNCE,
                (_, None) => {
                    p.changed = Some(Instant::now());
                    Instant::now() + DEBOUNCE
                }
            };
            let now = Instant::now();
            if now >= due {
                run.host.update_xaml(&run.project, &doc.path, doc.editor.buffer.rope().to_string());
                p.sent = Some(version);
                p.changed = None;
                p.updating = true;
            } else {
                ctx.request_repaint_after(due - now);
            }
        }
    }

    /// Starts the previewer again after it stopped (the Restart button).
    pub fn restart(&mut self, id: u64, env: &Env, ctx: &egui::Context) {
        let Some(p) = self.map.get_mut(&id) else { return };
        p.restarts.clear();
        match p.project().cloned() {
            Some(project) if project.missing_outputs() => p.stage = Stage::NeedsBuild(project),
            Some(project) => start(p, project, env, ctx),
            None => p.stage = Stage::Unsupported(XamlKind::Unknown),
        }
        p.sent = None;
    }

    /// Once per frame: closes the previews of closed documents, stops the hosts of previews
    /// out of sight, and publishes the previewer's errors in the editors and Problems.
    pub fn prune(&mut self, docs: &mut Docs, problems: &mut BTreeMap<PathBuf, Vec<rc_editor::Diagnostic>>) {
        self.map.retain(|id, _| docs.list.iter().any(|d| d.editor.id == *id));
        for p in self.map.values_mut() {
            if p.shown.elapsed() > IDLE_AFTER
                && let Stage::Running(r) = &p.stage
            {
                p.stage = Stage::Idle(r.project.clone());
                p.sent = None;
            }
        }
        let mut want: HashMap<PathBuf, rc_editor::Diagnostic> = HashMap::new();
        for d in &docs.list {
            let Some(p) = self.map.get(&d.editor.id) else { continue };
            if let (Some(e), Stage::Running(_)) = (&p.error, &p.stage) {
                want.insert(d.path.clone(), diagnostic(d, e));
            }
        }
        if want == self.published {
            return;
        }
        let mut touched: Vec<PathBuf> = self.published.keys().chain(want.keys()).cloned().collect();
        touched.sort();
        touched.dedup();
        for path in touched {
            let (old, new) = (self.published.get(&path), want.get(&path));
            if old == new {
                continue;
            }
            let entry = problems.entry(path.clone()).or_default();
            if let Some(old) = old {
                entry.retain(|d| d != old);
            }
            if let Some(new) = new {
                entry.push(new.clone());
            }
            let list = entry.clone();
            if list.is_empty() {
                problems.remove(&path);
            }
            if let Some(i) = docs.index_of(&path) {
                docs.list[i].editor.set_diagnostics(list);
            }
        }
        self.published = want;
    }

    /// Kills every previewer (quitting).
    pub fn stop_all(&mut self) {
        self.map.clear();
    }
}

/// Whether `project`'s packages were restored (`obj/project.assets.json`).
fn restored(project: &Path) -> bool {
    project.parent().is_some_and(|d| d.join("obj/project.assets.json").is_file())
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// The previewer's error at its position (the word there), for the editor.
fn diagnostic(doc: &Doc, e: &XamlError) -> rc_editor::Diagnostic {
    let buffer = &doc.editor.buffer;
    let line = e.line.unwrap_or(1).saturating_sub(1) as usize;
    let start = buffer.pos_at(line, e.column.unwrap_or(1).saturating_sub(1) as usize);
    let rope = buffer.rope();
    let mut end = start;
    while end < rope.len_chars() && rope.char(end).is_alphanumeric() {
        end += 1;
    }
    let message = e.message.lines().next().unwrap_or_default().to_string();
    rc_editor::Diagnostic { start, end: end.max(start), severity: rc_editor::Severity::Error, message, code: Some("XAML".into()), unnecessary: false, deprecated: false }
}

/// Finds the project of `file` and evaluates it in the background.
fn resolve(file: &Path, env: &Env, ctx: &egui::Context) -> Stage {
    let Some(dotnet) = env.dotnet.clone() else {
        return Stage::Unavailable(tr("The .NET SDK was not found. Install it or set its path in Settings › Tools.").into());
    };
    let ctx = ctx.clone();
    let inbox = Inbox::new(rc_core::Waker::new(move || ctx.request_repaint()));
    let tx = inbox.sender();
    let (file, executables) = (file.to_path_buf(), env.executables.clone());
    rc_core::Runtime::spawn(async move {
        let result = async {
            let project = project_of(&file).ok_or_else(|| tr("This file is not part of a project.").to_string())?;
            let app = app_project_for(&project, &executables)
                .ok_or_else(|| trf("No application references {project}. The previewer runs the XAML inside an Avalonia application of the solution.", &[("project", &file_name(&project))]))?;
            rc_dotnet::avalonia_preview::resolve(&dotnet, &project, &app).await.map_err(|e| e.to_string())
        }
        .await;
        tx.send(result);
    });
    Stage::Resolving(inbox)
}

fn start(p: &mut Preview, project: PreviewProject, env: &Env, ctx: &egui::Context) {
    let Some(dotnet) = env.dotnet.clone() else { return };
    let ctx = ctx.clone();
    let inbox = Inbox::new(rc_core::Waker::new(move || ctx.request_repaint()));
    p.sent = None;
    p.updating = false;
    p.stage = match DesignerHost::start(&dotnet, &project, env.scale, inbox.sender()) {
        Ok(host) => Stage::Running(Box::new(Running { stamp: project.outputs_stamp(), project, host, inbox, connected: false, scale: env.scale })),
        Err(e) => Stage::Stopped { project, message: trf("The previewer could not start: {error}", &[("error", &e)]), log: String::new() },
    };
}

/// Every few seconds, in the background: whether code changed after the last build.
fn check_stale(p: &mut Preview) {
    if let Some((rx, _)) = &p.stale_check
        && let Ok(stale) = rx.try_recv()
    {
        p.stale = stale;
    }
    if p.stale_check.as_ref().is_some_and(|(_, at)| at.elapsed() < Duration::from_secs(5)) {
        return;
    }
    let Some(project) = p.project().cloned() else { return };
    let (tx, rx) = mpsc::channel();
    p.stale_check = Some((rx, Instant::now()));
    std::thread::spawn(move || {
        let Some(built) = project.outputs_stamp() else { return };
        let dirs: Vec<PathBuf> = [&project.project, &project.app_project].iter().filter_map(|p| p.parent().map(Path::to_path_buf)).collect();
        let _ = tx.send(newest_source(&dirs).is_some_and(|t| t > built));
    });
}

/// The newest write time of the code and project files under `dirs` (bin, obj and hidden
/// folders skipped). XAML is left out: the preview shows the edited text itself.
pub fn newest_source(dirs: &[PathBuf]) -> Option<SystemTime> {
    let mut newest = None;
    let mut stack: Vec<PathBuf> = dirs.to_vec();
    let mut visited = 0;
    while let Some(dir) = stack.pop() {
        visited += 1;
        if visited > 2000 {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                if !name.starts_with('.') && name != "bin" && name != "obj" && name != "node_modules" {
                    stack.push(path);
                }
            } else if [".cs", ".fs", ".vb", ".csproj", ".fsproj", ".vbproj", ".props", ".targets"].iter().any(|x| name.ends_with(x))
                && let Ok(t) = e.metadata().and_then(|m| m.modified())
            {
                newest = newest.max(Some(t));
            }
        }
    }
    newest
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_the_frameworks_apart() {
        let p = Path::new("/nowhere/View.xaml");
        assert_eq!(kind(Path::new("/x/Main.axaml"), ""), XamlKind::Avalonia);
        assert_eq!(kind(p, "<?xml version=\"1.0\"?>\n<!-- <Window xmlns=\"x\"> -->\n<UserControl xmlns=\"https://github.com/avaloniaui\" xmlns:x=\"…\"/>"), XamlKind::Avalonia);
        assert_eq!(kind(p, "<ContentPage xmlns:x=\"http://schemas.microsoft.com/winfx/2009/xaml\"\n  xmlns=\"http://schemas.microsoft.com/dotnet/2021/maui\">"), XamlKind::Maui);
        assert_eq!(kind(p, "<ContentPage xmlns='http://xamarin.com/schemas/2014/forms'>"), XamlKind::Maui);
        assert_eq!(kind(p, "<Window xmlns=\"http://schemas.microsoft.com/winfx/2006/xaml/presentation\">"), XamlKind::Wpf);
        assert_eq!(kind(p, "<Window>"), XamlKind::Unknown);
        assert_eq!(kind(p, ""), XamlKind::Unknown);
        assert!(is_xaml(Path::new("A.AXAML")) && is_xaml(Path::new("B.xaml")) && !is_xaml(Path::new("C.xml")));
    }

    #[test]
    fn winui_is_told_from_wpf_by_its_project() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("App.csproj"), "<Project><PropertyGroup><UseWinUI>true</UseWinUI></PropertyGroup></Project>").unwrap();
        std::fs::create_dir(dir.path().join("Views")).unwrap();
        let page = dir.path().join("Views/MainPage.xaml");
        assert_eq!(kind(&page, "<Page xmlns=\"http://schemas.microsoft.com/winfx/2006/xaml/presentation\">"), XamlKind::WinUi);
    }

    #[test]
    fn libraries_preview_inside_an_application_that_references_them() {
        let dir = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        for (name, body) in [
            ("Controls/Controls.csproj", "<Project Sdk=\"Microsoft.NET.Sdk\"></Project>"),
            ("Ui/Ui.csproj", "<Project Sdk=\"Microsoft.NET.Sdk\"><ItemGroup><ProjectReference Include=\"..\\Controls\\Controls.csproj\" /></ItemGroup></Project>"),
            ("App/App.csproj", "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><OutputType>WinExe</OutputType></PropertyGroup><ItemGroup><ProjectReference Include=\"../Ui/Ui.csproj\" /></ItemGroup></Project>"),
            ("Other/Other.csproj", "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><OutputType>Exe</OutputType></PropertyGroup></Project>"),
        ] {
            let path = root.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        }
        std::fs::create_dir_all(root.join("Controls/Themes")).unwrap();
        let control = root.join("Controls/Themes/Button.axaml");
        let controls = project_of(&control).unwrap();
        assert_eq!(controls, root.join("Controls/Controls.csproj"));
        let exes = [root.join("Other/Other.csproj"), root.join("App/App.csproj")];
        assert_eq!(app_project_for(&controls, &exes), Some(root.join("App/App.csproj")));
        assert_eq!(app_project_for(&controls, &exes[..1]), None);
        let app = root.join("App/App.csproj");
        assert_eq!(app_project_for(&app, &[]), Some(app));
    }

    #[test]
    fn zoom_steps_and_fit() {
        assert_eq!(zoom_in(1.0), 1.25);
        assert_eq!(zoom_out(1.0), 0.75);
        assert_eq!(zoom_in(0.8), 1.0);
        assert_eq!(zoom_out(0.25), 0.25);
        assert_eq!(zoom_in(4.0), 4.0);
        assert_eq!(fit_scale(egui::vec2(800.0, 600.0), egui::vec2(400.0, 600.0)), 0.5);
        // Small views are not blown up.
        assert_eq!(fit_scale(egui::vec2(100.0, 50.0), egui::vec2(400.0, 600.0)), 1.0);
    }

    #[test]
    fn code_changes_after_the_build_are_found_but_not_build_output() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        std::fs::create_dir_all(root.join("obj")).unwrap();
        std::fs::write(root.join("obj/Generated.cs"), "").unwrap();
        assert_eq!(newest_source(std::slice::from_ref(&root)), None);
        std::fs::write(root.join("View.axaml.cs"), "").unwrap();
        std::fs::write(root.join("View.axaml"), "").unwrap();
        let t = std::fs::metadata(root.join("View.axaml.cs")).unwrap().modified().unwrap();
        assert_eq!(newest_source(&[root]), Some(t));
    }
}
