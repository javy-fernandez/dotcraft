//! The C# pipeline end to end inside the app (no window): Roslyn starts for a solution, an
//! opened file gets semantic colours and diagnostics, typing `.` opens a completion list with
//! the right members, and a rename edits both projects. Uses the server in `target/test-home`
//! (downloaded by the rc-lsp tests).

use crate::app::DotCraftApp;
use crate::insight::InsightStatus;
use rc_core::{Settings, Waker};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn write(p: &Path, s: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}

fn demo_solution(dir: &Path) -> PathBuf {
    write(&dir.join("Lib/Lib.csproj"), "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup><TargetFramework>net8.0</TargetFramework></PropertyGroup>\n</Project>\n");
    write(&dir.join("Lib/Greeter.cs"), "namespace Lib;\n\npublic class Greeter\n{\n    public string Hello(string name) => $\"Hello {name}\";\n}\n");
    write(&dir.join("App/App.csproj"), "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup><OutputType>Exe</OutputType><TargetFramework>net8.0</TargetFramework></PropertyGroup>\n  <ItemGroup><ProjectReference Include=\"..\\Lib\\Lib.csproj\" /></ItemGroup>\n</Project>\n");
    write(&dir.join("App/Program.cs"), "using Lib;\n\nvar greeter = new Greeter();\nSystem.Console.WriteLine(greeter.Hello(\"world\"));\nint unused = undefinedThing;\n");
    let sln = dir.join("Demo.slnx");
    write(&sln, "<Solution>\n  <Project Path=\"App/App.csproj\" />\n  <Project Path=\"Lib/Lib.csproj\" />\n</Solution>\n");
    assert!(std::process::Command::new("dotnet").args(["restore", sln.to_str().unwrap()]).output().unwrap().status.success());
    sln
}

/// Runs the app's per-frame logic until `done` holds.
fn pump(app: &mut DotCraftApp, what: &str, mut done: impl FnMut(&mut DotCraftApp) -> bool) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(120) {
        let out = app.insight.update(&mut app.docs, true, false);
        app.handle_insight(out);
        app.docs.poll_parses();
        if let InsightStatus::Failed(why) = &app.insight.status {
            panic!("server failed: {why}");
        }
        if done(app) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for {what}; status {:?}", app.insight.status);
}

#[test]
#[ignore = "Requires DOTCRAFT_TEST_ROSLYN and DOTCRAFT_TEST_DOTNET for a live Roslyn session"]
fn live_code_vision_labels_and_navigation() {
    let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let roslyn = PathBuf::from(std::env::var_os("DOTCRAFT_TEST_ROSLYN").expect("Roslyn server path"));
    let dotnet = PathBuf::from(std::env::var_os("DOTCRAFT_TEST_DOTNET").expect("dotnet SDK path"));
    let home = tempfile::tempdir().unwrap();
    // SAFETY: this test holds HOME_LOCK while the app reads the process-wide override.
    unsafe { std::env::set_var("DOTCRAFT_HOME", home.path()) };
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().canonicalize().unwrap();
    let project = root.join("Vision.csproj");
    let source = root.join("Vision.cs");
    write(&project, "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>net10.0</TargetFramework></PropertyGroup></Project>\n");
    write(&source, "public interface IGreeter { string Hello(); }\npublic class Greeter : IGreeter { public virtual string Hello() => \"hello\"; }\npublic class LoudGreeter : Greeter { public override string Hello() => \"LOUD\"; }\npublic class Consumer { public string Read(IGreeter greeter) => greeter.Hello(); }\n");
    let restored = std::process::Command::new(&dotnet).args(["restore", "--ignore-failed-sources"]).arg(&project).output().unwrap();
    assert!(restored.status.success(), "{}", String::from_utf8_lossy(&restored.stderr));
    let mut settings = Settings::default();
    settings.roslyn_path = Some(roslyn);
    settings.dotnet_path = Some(dotnet);
    settings.show_code_vision = true;
    let mut app = DotCraftApp::with_settings(settings, Waker::noop(), None);
    app.open_workspace(&project);
    app.open_file(&source);
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(120) {
        let outputs = app.insight.update(&mut app.docs, true, true);
        app.handle_insight(outputs);
        app.docs.poll_parses();
        if let InsightStatus::Failed(why) = &app.insight.status {
            panic!("server failed: {why}");
        }
        let ready = app.docs.active_doc().is_some_and(|doc| {
            let labels = &doc.editor.code_vision;
            labels.iter().any(|label| label.kind == rc_editor::CodeVisionKind::Usages)
                && labels.iter().any(|label| label.kind == rc_editor::CodeVisionKind::Implementations)
                && labels.iter().any(|label| label.kind == rc_editor::CodeVisionKind::Overrides)
        });
        if ready {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let labels = app.docs.active_doc().unwrap().editor.code_vision.clone();
    assert!(labels.iter().any(|label| label.kind == rc_editor::CodeVisionKind::Usages), "{labels:?}");
    assert!(labels.iter().any(|label| label.kind == rc_editor::CodeVisionKind::Implementations), "{labels:?}");
    assert!(labels.iter().any(|label| label.kind == rc_editor::CodeVisionKind::Overrides), "{labels:?}");
    app.insight.update(&mut app.docs, true, false);
    assert!(app.docs.active_doc().unwrap().editor.code_vision.is_empty());
    let enabled = Instant::now();
    while enabled.elapsed() < Duration::from_secs(30) {
        app.insight.update(&mut app.docs, true, true);
        if app.docs.active_doc().unwrap().editor.code_vision.iter().any(|label| label.kind == rc_editor::CodeVisionKind::Overrides && label.title.starts_with('1')) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(app.docs.active_doc().unwrap().editor.code_vision.iter().any(|label| label.kind == rc_editor::CodeVisionKind::Overrides && label.title.starts_with('1')));
    let doc = app.docs.active_doc_mut().unwrap();
    let end = doc.editor.buffer.len_chars();
    doc.editor.replace(end, end, "\n");
    app.docs.reparse_edited();
    app.insight.update(&mut app.docs, true, true);
    assert!(app.docs.active_doc().unwrap().editor.code_vision.is_empty());
    let edited = Instant::now();
    while edited.elapsed() < Duration::from_secs(30) {
        let outputs = app.insight.update(&mut app.docs, true, true);
        app.handle_insight(outputs);
        app.docs.poll_parses();
        if app.docs.active_doc().unwrap().editor.code_vision.iter().any(|label| label.kind == rc_editor::CodeVisionKind::Overrides && label.title.starts_with('1')) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(app.docs.active_doc().unwrap().editor.code_vision.iter().any(|label| label.kind == rc_editor::CodeVisionKind::Overrides && label.title.starts_with('1')));
    let labels = app.docs.active_doc().unwrap().editor.code_vision.clone();
    let target = labels.iter().find(|label| label.kind == rc_editor::CodeVisionKind::Overrides && label.title.starts_with('1')).unwrap();
    let position = target.position;
    let active = app.docs.active.unwrap();
    app.insight.navigate(&app.docs.list[active], position, crate::insight::NavKind::Implementation);
    let navigation_start = Instant::now();
    while navigation_start.elapsed() < Duration::from_secs(30) {
        let outputs = app.insight.update(&mut app.docs, true, true);
        if let Some(target) = outputs.iter().find_map(|output| match output {
            crate::insight::InsightOutput::Navigate(target) => Some(target),
            _ => None,
        }) {
            assert_eq!(target.path, source);
            assert_eq!(target.line, 2);
            if let Some(path) = std::env::var_os("DOTCRAFT_CODE_VISION_SCREENSHOT") {
                let mut harness = crate::screens::harness(app, egui::vec2(1400.0, 900.0));
                harness.run_steps(4);
                harness.render().unwrap().save(path).unwrap();
                let at = egui::pos2(620.0, 173.0);
                harness.event(egui::Event::PointerMoved(at));
                harness.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
                harness.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
                let clicked = Instant::now();
                while clicked.elapsed() < Duration::from_secs(10) {
                    harness.run_steps(2);
                    if harness.state().docs.active_doc().is_some_and(|doc| doc.editor.caret_line_col().0 == 2) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                panic!("Code Vision label click did not navigate to the overriding method");
            } else {
                app.close_workspace();
            }
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for Code Vision implementation navigation");
}

#[test]
fn csharp_insight_end_to_end() {
    let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-home").canonicalize().expect("run the rc-lsp tests first");
    // SAFETY: set before the app reads it; other tests use their own temp homes the same way.
    unsafe { std::env::set_var("DOTCRAFT_HOME", &home) };
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let sln = demo_solution(&root);
    let mut app = DotCraftApp::with_settings(Settings::default(), Waker::noop(), None);
    app.open_workspace(&sln);
    pump(&mut app, "projects", |a| a.insight.status == InsightStatus::Ready);

    let program = root.join("App/Program.cs");
    app.open_file(&program);
    // Diagnostics: the undefined name is an error, with the right range.
    pump(&mut app, "diagnostics", |a| a.docs.active_doc().is_some_and(|d| d.editor.diagnostics.iter().any(|x| x.code.as_deref() == Some("CS0103"))));
    let d = app.docs.active_doc().unwrap();
    let diag = d.editor.diagnostics.iter().find(|x| x.code.as_deref() == Some("CS0103")).unwrap();
    assert_eq!(d.editor.buffer.rope().slice(diag.start..diag.end).to_string(), "undefinedThing");
    assert!(app.insight.problems.contains_key(&program));
    // Semantic colours: `Greeter` is coloured as a class by the server.
    pump(&mut app, "semantic tokens", |a| a.docs.active_doc().is_some_and(|d| d.editor.semantic.get(2).is_some_and(|l| l.iter().any(|s| s.hl == rc_syntax::Hl::Class))));

    // Typing "greeter." on a new line opens the completion with Hello.
    let doc = app.docs.active_doc_mut().unwrap();
    let end = doc.editor.buffer.len_chars();
    doc.editor.replace(end, end, "greeter.");
    doc.editor.selection = rc_text::Selection::point(end + 8);
    let i = app.docs.active.unwrap();
    let t = Instant::now();
    app.insight.on_editor_events(&mut app.docs.list[i], &[rc_editor::EditorEvent::Typed('.')]);
    pump(&mut app, "completion", |a| a.docs.active_doc().and_then(|d| d.editor.popups.completion.as_ref()).is_some_and(|c| !c.filtered.is_empty()));
    eprintln!("completion shown {:?} after typing", t.elapsed());
    let c = app.docs.active_doc().unwrap().editor.popups.completion.clone().unwrap();
    assert!(c.filtered.iter().any(|&k| c.items[k].label == "Hello"));

    // Rename Hello → Greet from App: both files change (Greeter.cs on disk, Program.cs in the editor).
    let doc = app.docs.active_doc_mut().unwrap();
    doc.editor.popups.close_all();
    let pos = doc.editor.buffer.rope().to_string().find("greeter.Hello").unwrap() + "greeter.".len() + 1;
    let i = app.docs.active.unwrap();
    app.insight.rename(&app.docs.list[i], pos, "Greet");
    pump(&mut app, "rename", |a| a.docs.active_doc().is_some_and(|d| d.editor.buffer.rope().to_string().contains("greeter.Greet(")));
    let lib = std::fs::read_to_string(root.join("Lib/Greeter.cs")).unwrap();
    assert!(lib.contains("public string Greet("), "{lib}");

    // Pasting code with an unresolved type adds its using.
    app.open_file(&program);
    let doc = app.docs.active_doc_mut().unwrap();
    let end = doc.editor.buffer.len_chars();
    doc.editor.replace(end, end, "\nvar sb = new StringBuilder();\n");
    let i = app.docs.active.unwrap();
    app.insight.on_editor_events(&mut app.docs.list[i], &[rc_editor::EditorEvent::Changed, rc_editor::EditorEvent::Pasted]);
    pump(&mut app, "the using", |a| a.docs.active_doc().is_some_and(|d| d.editor.buffer.rope().to_string().contains("using System.Text;")));
    let text = app.docs.active_doc().unwrap().editor.buffer.rope().to_string();
    assert!(text.starts_with("using Lib;\nusing System.Text;\n") || text.starts_with("using System.Text;\nusing Lib;\n"), "{text}");

    // `///` above a method: Roslyn inserts the doc comment skeleton with its parameter.
    app.open_file(&root.join("Lib/Greeter.cs"));
    pump(&mut app, "Greeter.cs to load", |a| a.docs.active_doc().is_some_and(|d| !d.editor.semantic.is_empty()));
    let doc = app.docs.active_doc_mut().unwrap();
    let text = doc.editor.buffer.rope().to_string();
    let at = text.find("    public string Greet(").unwrap();
    let pos = doc.editor.buffer.rope().byte_to_char(at);
    doc.editor.replace(pos, pos, "    ///\n");
    doc.editor.selection = rc_text::Selection::point(pos + 7);
    let i = app.docs.active.unwrap();
    app.insight.on_editor_events(&mut app.docs.list[i], &[rc_editor::EditorEvent::Typed('/')]);
    pump(&mut app, "the doc comment", |a| a.docs.active_doc().is_some_and(|d| d.editor.buffer.rope().to_string().contains("<summary>")));
    let d = app.docs.active_doc().unwrap();
    let text = d.editor.buffer.rope().to_string();
    assert!(text.contains("/// <param name=\"name\">") && text.contains("/// <returns>"), "{text}");
    // The caret waits inside the summary.
    let caret_line = d.editor.buffer.rope().char_to_line(d.editor.caret());
    assert!(text.lines().nth(caret_line - 1).unwrap().contains("<summary>"), "{text}");
    app.close_workspace();
}

/// ⌘+click: to the declaration in another file and back, its usages from the declaration
/// itself, and a framework type.
#[test]
fn go_to_declaration_and_back() {
    let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-home").canonicalize().expect("run the rc-lsp tests first");
    // SAFETY: as above.
    unsafe { std::env::set_var("DOTCRAFT_HOME", &home) };
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let sln = demo_solution(&root);
    let (program, greeter) = (root.join("App/Program.cs"), root.join("Lib/Greeter.cs"));
    let mut app = DotCraftApp::with_settings(Settings::default(), Waker::noop(), None);
    app.open_workspace(&sln);
    app.open_file(&program);
    // Clicked while the projects still load: it goes there once they have.
    pump(&mut app, "the server", |a| a.insight.ready());
    let click = |app: &mut DotCraftApp, word: &str| {
        let i = app.docs.active.unwrap();
        let d = &mut app.docs.list[i];
        let pos = d.editor.buffer.rope().to_string().find(word).unwrap() + 2;
        d.editor.set_selection(rc_text::Selection::point(pos));
        let path = d.path.clone();
        app.insight.on_editor_events(&mut app.docs.list[i], &[rc_editor::EditorEvent::NavigateAt(pos)]);
        app.push_nav_at(path, pos);
        pos
    };
    let at = |a: &DotCraftApp| a.docs.active_doc().map(|d| (d.path.clone(), d.editor.caret_line_col()));

    // To the method in the other project's file, on its name.
    let from = click(&mut app, "Hello(\"world\")");
    pump(&mut app, "Greeter.cs", |a| a.docs.active_doc().is_some_and(|d| d.path == greeter));
    assert_eq!(at(&app).unwrap().1, (4, 18));
    // Back to the call, forward to the method again.
    app.navigate(true);
    assert_eq!(at(&app).map(|(p, _)| p), Some(program.clone()));
    assert_eq!(app.docs.active_doc().unwrap().editor.caret(), from);
    app.navigate(false);
    assert_eq!(at(&app), Some((greeter.clone(), (4, 18))));

    // On the declaration itself: its usages (one, so it goes there).
    click(&mut app, "Hello(string");
    pump(&mut app, "the usage", |a| a.docs.active_doc().is_some_and(|d| d.path == program));
    assert_eq!(at(&app).unwrap().1.0, 3);

    // A framework type opens its decompiled or metadata source.
    click(&mut app, "Console.");
    pump(&mut app, "Console's source", |a| a.docs.active_doc().is_some_and(|d| d.path != program));
    let d = app.docs.active_doc().unwrap();
    assert!(d.path.ends_with("Console.cs") && d.editor.read_only && d.editor.buffer.rope().to_string().contains("class Console"), "{}", d.path.display());
    app.close_workspace();
}

/// ⌃⌥H on `Hello`: its caller in Program.cs; ⌃H on a derived class: its base type.
#[test]
fn call_and_type_hierarchy() {
    let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-home").canonicalize().expect("run the rc-lsp tests first");
    // SAFETY: as above.
    unsafe { std::env::set_var("DOTCRAFT_HOME", &home) };
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let sln = demo_solution(&root);
    write(&root.join("Lib/LoudGreeter.cs"), "namespace Lib;\n\npublic class LoudGreeter : Greeter\n{\n}\n");
    let greeter = root.join("Lib/Greeter.cs");
    let mut app = DotCraftApp::with_settings(Settings::default(), Waker::noop(), None);
    app.open_workspace(&sln);
    pump(&mut app, "projects", |a| a.insight.status == InsightStatus::Ready);
    app.open_file(&greeter);
    let i = app.docs.active.unwrap();
    let pos = app.docs.list[i].editor.buffer.rope().to_string().find("Hello(").unwrap() + 1;
    app.docs.list[i].editor.selection = rc_text::Selection::point(pos);
    app.run_insight_action(rc_editor::Action::CallHierarchy, egui::Pos2::ZERO);
    pump(&mut app, "callers", |a| a.hierarchy.as_ref().is_some_and(|h| h.nodes[0].children.as_ref().is_some_and(|c| !c.is_empty())));
    let h = app.hierarchy.clone().unwrap();
    assert!(h.nodes[0].name.starts_with("Greeter.Hello"), "{:?}", h.nodes[0].name);
    let callers: Vec<_> = h.nodes[0].children.as_ref().unwrap().iter().map(|&c| h.nodes[c].clone()).collect();
    assert!(callers.iter().any(|c| c.path.as_deref() == Some(root.join("App/Program.cs").as_path())), "{callers:?}");

    // Type hierarchy of Greeter: LoudGreeter derives from it.
    let pos = app.docs.list[i].editor.buffer.rope().to_string().find("class Greeter").unwrap() + 7;
    app.docs.list[i].editor.selection = rc_text::Selection::point(pos);
    app.run_insight_action(rc_editor::Action::TypeHierarchy, egui::Pos2::ZERO);
    pump(&mut app, "derived types", |a| a.hierarchy.as_ref().is_some_and(|h| h.kind == crate::hierarchy::HierarchyKind::Types && h.nodes[0].children.as_ref().is_some_and(|c| !c.is_empty())));
    let h = app.hierarchy.clone().unwrap();
    let derived: Vec<String> = h.nodes[0].children.as_ref().unwrap().iter().map(|&c| h.nodes[c].name.clone()).collect();
    assert!(derived.iter().any(|n| n.contains("LoudGreeter")), "{derived:?}");
    app.close_workspace();
}
