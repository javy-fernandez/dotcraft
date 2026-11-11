//! Package insight in project files through the whole app (offscreen harness), against a local
//! NuGet v3 feed: completion, hover, update markers and fixes. No network.

use crate::app::DotCraftApp;
use crate::screens::{HOME_LOCK, fixtures, harness, test_app};
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// A NuGet v3 feed on localhost with a few packages; records the requests it gets.
pub struct FakeFeed {
    pub url: String,
    pub requests: Arc<Mutex<Vec<String>>>,
}

const PACKAGES: &[(&str, &str, &[&str])] = &[
    ("Newtonsoft.Json", "Json.NET is a popular high-performance JSON framework for .NET", &["12.0.3", "13.0.1", "13.0.3", "13.0.4", "14.0.0-beta1"]),
    ("Humanizer.Core", "Humanizer core: turns strings, enums, dates and numbers into readable text", &["2.14.1", "3.0.0-rc.30", "3.0.1"]),
    ("Dapper", "A high performance Micro-ORM supporting SQL Server, MySQL, Sqlite, SqlCE, Firebird etc.", &["2.1.66"]),
    ("Serilog", "Simple .NET logging with fully-structured events", &["3.1.1", "4.0.0", "4.3.0", "4.3.1-dev-02390"]),
    ("Serilog.Sinks.Console", "A Serilog sink that writes log events to the console", &["6.0.0"]),
    ("Serilog.Extensions.Logging", "Low-level Serilog provider for Microsoft.Extensions.Logging", &["9.0.2"]),
];

pub fn fake_feed() -> FakeFeed {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let (log, b) = (requests.clone(), base.clone());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (log, b) = (log.clone(), b.clone());
            std::thread::spawn(move || serve(stream, &b, &log));
        }
    });
    FakeFeed { url: format!("{base}/v3/index.json"), requests }
}

fn serve(mut stream: std::net::TcpStream, base: &str, log: &Mutex<Vec<String>>) {
    // BSD sockets inherit the listener's mode.
    let _ = stream.set_nonblocking(false);
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut first = String::new();
    if reader.read_line(&mut first).is_err() {
        return;
    }
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
            break;
        }
    }
    let target = first.split_whitespace().nth(1).unwrap_or("/").to_string();
    log.lock().unwrap().push(target.clone());
    let (path, query) = target.split_once('?').unwrap_or((&target, ""));
    let param = |k: &str| query.split('&').find_map(|kv| kv.strip_prefix(&format!("{k}="))).map(decode).unwrap_or_default();
    let summary = |(id, desc, versions): &(&str, &str, &[&str])| {
        let latest = versions.iter().rfind(|v| !v.contains('-')).unwrap();
        serde_json::json!({ "id": id, "version": latest, "description": desc, "authors": ["Someone"], "totalDownloads": 1000, "versions": versions.iter().map(|v| serde_json::json!({ "version": v })).collect::<Vec<_>>() })
    };
    let (status, body) = if path == "/v3/index.json" {
        ("200 OK", serde_json::json!({ "version": "3.0.0", "resources": [
            { "@id": format!("{base}/query"), "@type": "SearchQueryService/3.5.0" },
            { "@id": format!("{base}/flat/"), "@type": "PackageBaseAddress/3.0.0" },
        ] }).to_string())
    } else if path == "/query" {
        let q = param("q").to_ascii_lowercase();
        let data: Vec<_> = match q.strip_prefix("packageid:") {
            Some(id) => PACKAGES.iter().filter(|p| p.0.eq_ignore_ascii_case(id)).map(summary).collect(),
            None => PACKAGES.iter().filter(|p| p.0.to_ascii_lowercase().contains(&q)).map(summary).collect(),
        };
        ("200 OK", serde_json::json!({ "totalHits": data.len(), "data": data }).to_string())
    } else if let Some(id) = path.strip_prefix("/flat/").and_then(|p| p.strip_suffix("/index.json")) {
        match PACKAGES.iter().find(|p| p.0.eq_ignore_ascii_case(id)) {
            Some(p) => ("200 OK", serde_json::json!({ "versions": p.2 }).to_string()),
            None => ("404 Not Found", String::new()),
        }
    } else {
        ("404 Not Found", String::new())
    };
    let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 3 <= b.len() => {
                out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap_or(b'?'));
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A copy of CentralShop using `feed` (temp path canonicalized for macOS).
pub fn central_shop(feed: &FakeFeed) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::Builder::new().prefix("dotcraft-packages").tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap().join("CentralShop");
    copy(&fixtures().join("CentralShop"), &root);
    let config = format!("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<configuration>\n  <packageSources>\n    <clear />\n    <add key=\"local\" value=\"{}\" />\n  </packageSources>\n</configuration>\n", feed.url);
    std::fs::write(root.join("nuget.config"), config).unwrap();
    (tmp, root)
}

fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        if e.file_name() == "bin" || e.file_name() == "obj" {
            continue;
        }
        if e.file_type().unwrap().is_dir() {
            copy(&e.path(), &to.join(e.file_name()));
        } else {
            std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
        }
    }
}

/// The app on CentralShop with a project file open.
pub fn app_with(root: &Path, home: &Path, dark: bool, file: &Path) -> Harness<'static, DotCraftApp> {
    let mut app = test_app(home, dark);
    app.settings.roslyn_path = Some("/nonexistent".into());
    app.open_workspace(&root.join("CentralShop.slnx"));
    app.open_file(file);
    let mut h = harness(app, vec2(1280.0, 800.0));
    settle(&mut h, "the first analysis", |_| true);
    h
}

/// Runs frames until `done` (and the package insight is idle), at most 20 s.
pub fn settle(h: &mut Harness<'static, DotCraftApp>, what: &str, mut done: impl FnMut(&DotCraftApp) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        h.run_steps(1);
        if h.state().packages.idle_for_test() && done(h.state()) {
            h.run_steps(2);
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {what} (idle: {}; notices: {:?})",
            h.state().packages.idle_for_test(),
            h.state().notices.iter().map(|n| n.text.clone()).collect::<Vec<_>>()
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

pub fn text(app: &DotCraftApp) -> String {
    app.docs.active_doc().unwrap().editor.buffer.rope().to_string()
}

/// Puts the caret at the char offset of `needle`'s end (or start) in the active editor.
pub fn caret_at(h: &mut Harness<'static, DotCraftApp>, needle: &str, end: bool) {
    let t = text(h.state());
    let b = t.find(needle).unwrap() + if end { needle.len() } else { 0 };
    let pos = t[..b].chars().count();
    let app = h.state_mut();
    app.docs.active_doc_mut().unwrap().editor.set_selection(rc_text::Selection::point(pos));
    app.focus_editor = true;
    h.run_steps(1);
}

pub fn type_text(h: &mut Harness<'static, DotCraftApp>, s: &str) {
    for ch in s.chars() {
        if ch == '\n' {
            h.key_press(egui::Key::Enter);
        } else {
            h.event(egui::Event::Text(ch.to_string()));
        }
        h.run_steps(1);
    }
}

/// Chooses an entry of the Context Actions popup (shown with that label), as its Enter does.
fn choose_action(h: &mut Harness<'static, DotCraftApp>, title: &str) {
    h.get_by_label(title);
    let app = h.state_mut();
    let Some(crate::ui::dialogs::Dialog::CodeActions { list, .. }) = app.dialog.take() else { panic!("no context actions") };
    let action = list.into_iter().find(|a| matches!(a, rc_lsp::lsp_types::CodeActionOrCommand::CodeAction(c) if c.title == title)).unwrap();
    app.apply_package_fix(crate::package_insight::fix_of(&action).unwrap());
    h.run_steps(1);
}

fn labels(app: &DotCraftApp) -> Vec<String> {
    let c = app.docs.active_doc().unwrap().editor.popups.completion.as_ref().unwrap();
    c.filtered.iter().map(|&i| c.items[i].label.clone()).collect()
}

fn searches(feed: &FakeFeed) -> Vec<String> {
    feed.requests.lock().unwrap().iter().filter(|r| r.starts_with("/query") && !r.contains("packageid")).cloned().collect()
}

#[test]
fn package_ids_complete_from_the_feed_without_a_version_under_cpm() {
    let _lock = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    let feed = fake_feed();
    let (_tmp, root) = central_shop(&feed);
    let core = root.join("src/CentralShop.Core/CentralShop.Core.csproj");
    let mut h = app_with(&root, home.path(), true, &core);
    // A new reference, typed: one search after the pause, not one per key.
    caret_at(&mut h, "<PackageReference Include=\"Newtonsoft.Json\" />", true);
    type_text(&mut h, "\n<PackageReference Include=");
    assert!(text(h.state()).contains("<PackageReference Include=\"\""), "{}", text(h.state()));
    type_text(&mut h, "Seri");
    settle(&mut h, "search results", |a| a.docs.active_doc().unwrap().editor.popups.completion.as_ref().is_some_and(|c| !c.loading && !c.items.is_empty()));
    assert_eq!(labels(h.state()), ["Serilog", "Serilog.Sinks.Console", "Serilog.Extensions.Logging"]);
    assert_eq!(searches(&feed).len(), 1, "{:?}", searches(&feed));
    // A dot closes the list in other languages; here the id goes on.
    type_text(&mut h, "log.S");
    settle(&mut h, "narrower results", |a| a.docs.active_doc().unwrap().editor.popups.completion.as_ref().is_some_and(|c| !c.loading));
    assert_eq!(labels(h.state()), ["Serilog.Sinks.Console"]);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    let t = text(h.state());
    assert!(t.contains("<PackageReference Include=\"Serilog.Sinks.Console\""), "{t}");
    assert!(!t.contains("Serilog.Sinks.Console\" Version"), "{t}");
    // Close the element.
    {
        let doc = h.state_mut().docs.active_doc_mut().unwrap();
        let at = t[..t.find("Serilog.Sinks.Console\"").unwrap() + "Serilog.Sinks.Console\"".len()].chars().count();
        doc.editor.apply_text_edits(vec![rc_text::Change::insert(at, " />")]);
    }
    // No central version yet: NU1010 on the id, with its fix.
    settle(&mut h, "NU1010", |a| a.docs.active_doc().unwrap().editor.diagnostics.iter().any(|d| d.code.as_deref() == Some("NU1010")));
    assert!(h.state().insight.problems.get(&core).is_some_and(|p| p.iter().any(|d| d.code.as_deref() == Some("NU1010"))));
    caret_at(&mut h, "Serilog.Sinks.Console", false);
    h.state_mut().run_insight_action(rc_editor::Action::ShowIntentions, egui::pos2(300.0, 300.0));
    h.run_steps(2);
    choose_action(&mut h, "Add the latest version of Serilog.Sinks.Console to Directory.Packages.props");
    settle(&mut h, "the central version", |a| a.docs.active_doc().unwrap().editor.diagnostics.iter().all(|d| d.code.as_deref() != Some("NU1010")));
    let central = std::fs::read_to_string(root.join("Directory.Packages.props")).unwrap();
    assert!(central.contains("<PackageVersion Include=\"Serilog.Sinks.Console\" Version=\"6.0.0\" />"), "{central}");
}

#[test]
fn versions_complete_newest_stable_first() {
    let _lock = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    let feed = fake_feed();
    let (_tmp, root) = central_shop(&feed);
    let cli = root.join("src/CentralShop.Cli/CentralShop.Cli.csproj");
    let mut h = app_with(&root, home.path(), true, &cli);
    caret_at(&mut h, "VersionOverride=\"", true);
    h.state_mut().run_insight_action(rc_editor::Action::Complete, egui::pos2(300.0, 300.0));
    settle(&mut h, "versions", |a| a.docs.active_doc().unwrap().editor.popups.completion.as_ref().is_some_and(|c| !c.items.is_empty()));
    assert_eq!(labels(h.state()), ["13.0.4", "13.0.3", "13.0.1", "12.0.3", "14.0.0-beta1"]);
    let c = h.state().docs.active_doc().unwrap().editor.popups.completion.clone().unwrap();
    assert_eq!((c.items[0].type_text.as_str(), c.items[4].type_text.as_str()), ("latest", "prerelease"));
    // Accepting replaces the whole value, also the part after the caret.
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert!(text(h.state()).contains("<PackageReference Include=\"Newtonsoft.Json\" VersionOverride=\"13.0.4\" />"), "{}", text(h.state()));
}

#[test]
fn hover_and_update_marker_use_the_central_file() {
    let _lock = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    let feed = fake_feed();
    let (_tmp, root) = central_shop(&feed);
    let core = root.join("src/CentralShop.Core/CentralShop.Core.csproj");
    let mut h = app_with(&root, home.path(), true, &core);
    // Humanizer.Core 2.14.1 (central) has 3.0.1; Newtonsoft.Json 13.0.1 has 13.0.4.
    let line = text(h.state()).lines().position(|l| l.contains("Humanizer.Core")).unwrap();
    settle(&mut h, "markers", |a| a.docs.active_doc().unwrap().editor.gutter_icons.get(&line) == Some(&rc_editor::GutterIcon::PackageUpdate));
    let doc = h.state().docs.active_doc().unwrap();
    assert!(doc.editor.inlays.iter().any(|i| i.label == "3.0.1 available"), "{:?}", doc.editor.inlays);
    // The gutter icon offers the update.
    let outs = {
        let app = h.state_mut();
        let doc = &mut app.docs.list[app.docs.active.unwrap()];
        app.packages.on_editor_events(doc, &[rc_editor::EditorEvent::GutterIconClicked { line, icon: rc_editor::GutterIcon::PackageUpdate }])
    };
    let titles: Vec<String> = match outs.as_slice() {
        [crate::package_insight::PackageOutput::Actions(list)] => list.iter().map(|a| match a {
            rc_lsp::lsp_types::CodeActionOrCommand::CodeAction(c) => c.title.clone(),
            rc_lsp::lsp_types::CodeActionOrCommand::Command(c) => c.title.clone(),
        }).collect(),
        other => panic!("{other:?}"),
    };
    assert_eq!(titles, ["Update Humanizer.Core to 3.0.1", "Open Humanizer.Core in NuGet Window"]);
    // Hover: description, versions and where the version lives.
    let pos = {
        let t = text(h.state());
        t[..t.find("Humanizer.Core").unwrap() + 3].chars().count()
    };
    {
        let app = h.state_mut();
        let doc = &mut app.docs.list[app.docs.active.unwrap()];
        app.packages.on_editor_events(doc, &[rc_editor::EditorEvent::HoverAt { pos, screen: egui::pos2(300.0, 200.0) }]);
    }
    settle(&mut h, "the hover", |a| a.docs.active_doc().unwrap().editor.popups.hover.is_some());
    let hover = h.state().docs.active_doc().unwrap().editor.popups.hover.clone().unwrap();
    for part in ["`Humanizer.Core`", "readable text", "Version in effect: `2.14.1`", "Central version from `Directory.Packages.props`", "Latest stable version: `3.0.1`"] {
        assert!(hover.text.contains(part), "{part}: {}", hover.text);
    }
    h.get_by_label("Update to 3.0.1").click();
    settle(&mut h, "the update", |a| !a.docs.active_doc().unwrap().editor.gutter_icons.contains_key(&line));
    let central = std::fs::read_to_string(root.join("Directory.Packages.props")).unwrap();
    assert!(central.contains("<PackageVersion Include=\"Humanizer.Core\" Version=\"3.0.1\" />"), "{central}");
    // The project file itself is untouched.
    assert!(!h.state().docs.active_doc().unwrap().is_modified());
}

#[test]
fn version_on_a_reference_is_moved_to_the_central_file() {
    let _lock = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    let feed = fake_feed();
    let (_tmp, root) = central_shop(&feed);
    let core = root.join("src/CentralShop.Core/CentralShop.Core.csproj");
    let on_disk = std::fs::read_to_string(&core).unwrap().replace("Include=\"Humanizer.Core\" />", "Include=\"Humanizer.Core\" Version=\"2.14.1\" />");
    std::fs::write(&core, on_disk).unwrap();
    // Closed files show their problems too.
    let central = root.join("Directory.Packages.props");
    let mut h = app_with(&root, home.path(), true, &central);
    settle(&mut h, "problems of closed files", |a| a.insight.problems.get(&core).is_some_and(|p| p.iter().any(|d| d.code.as_deref() == Some("NU1008"))));
    // The unused Dapper version, dimmed in the central file.
    let doc = h.state().docs.active_doc().unwrap();
    let unused = doc.editor.diagnostics.iter().find(|d| d.unnecessary).unwrap();
    assert_eq!(doc.editor.buffer.rope().slice(unused.start..unused.end).to_string(), "Dapper");
    h.state_mut().open_file(&core);
    settle(&mut h, "NU1008", |a| a.docs.active_doc().unwrap().editor.diagnostics.iter().any(|d| d.code.as_deref() == Some("NU1008")));
    let doc = h.state().docs.active_doc().unwrap();
    let d = doc.editor.diagnostics.iter().find(|d| d.code.as_deref() == Some("NU1008")).unwrap();
    assert_eq!(doc.editor.buffer.rope().slice(d.start..d.end).to_string(), "Version=\"2.14.1\"");
    caret_at(&mut h, "Humanizer.Core", false);
    h.state_mut().run_insight_action(rc_editor::Action::ShowIntentions, egui::pos2(300.0, 300.0));
    h.run_steps(2);
    choose_action(&mut h, "Move the version of Humanizer.Core to Directory.Packages.props");
    settle(&mut h, "the fix", |a| a.insight.problems.get(&core).is_none_or(|p| p.iter().all(|d| d.code.as_deref() != Some("NU1008"))));
    assert!(std::fs::read_to_string(&core).unwrap().contains("<PackageReference Include=\"Humanizer.Core\" />"));
    assert_eq!(std::fs::read_to_string(&central).unwrap().matches("Humanizer.Core").count(), 1);
}

#[test]
fn nuget_build_errors_point_at_the_package() {
    let _lock = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    let feed = fake_feed();
    let (_tmp, root) = central_shop(&feed);
    let core = root.join("src/CentralShop.Core/CentralShop.Core.csproj");
    let mut h = app_with(&root, home.path(), true, &core);
    let output = format!(
        "{p} : error NU1101: Unable to find package Humanizer.Core. No packages exist with this id in source(s): local\n/usr/local/share/dotnet/sdk/8.0.100/NuGet.targets(170,5): error NU1301: Unable to load the service index for source http://x/v3/index.json. [{p}]",
        p = core.display()
    );
    h.state_mut().set_build_problems(rc_dotnet::diagnostics::parse_all(&output));
    h.run_steps(2);
    let app = h.state();
    let list = &app.insight.problems[&core];
    let nu1101 = list.iter().find(|d| d.code.as_deref() == Some("NU1101")).unwrap();
    let rope = &app.docs.active_doc().unwrap().editor.buffer;
    assert_eq!(rope.rope().slice(nu1101.start..nu1101.end).to_string(), "Humanizer.Core");
    // Raised in the SDK's targets, shown on the project.
    assert!(list.iter().any(|d| d.code.as_deref() == Some("NU1301")));
    // And in the editor, next to ours.
    h.run_steps(2);
    assert!(h.state().docs.active_doc().unwrap().editor.diagnostics.iter().any(|d| d.code.as_deref() == Some("NU1101")));
}
