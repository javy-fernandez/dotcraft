//! Offscreen captures of the whole window (wgpu, no input sent to any real window), for
//! visual review and the README. Run with `cargo test -p rc-app screens -- --ignored`;
//! images go to `target/snapshots/`.

use crate::app::DotCraftApp;
use egui::vec2;
use egui_kittest::Harness;
use rc_core::{Settings, Waker};
use std::path::{Path, PathBuf};

pub fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures").canonicalize().unwrap()
}

/// Tests that point `DOTCRAFT_HOME` somewhere hold this lock: the variable is process-wide.
pub static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// An app with its own data folder (never the user's settings or sessions).
pub fn test_app(home: &Path, dark: bool) -> DotCraftApp {
    // SAFETY: tests that read DOTCRAFT_HOME set it to the same kind of temp folder first.
    unsafe { std::env::set_var("DOTCRAFT_HOME", home) };
    let mut settings = Settings::default();
    settings.theme = if dark { rc_core::settings::ThemeChoice::Dark } else { rc_core::settings::ThemeChoice::Light };
    DotCraftApp::with_settings(settings, Waker::noop(), None)
}

pub fn harness(app: DotCraftApp, size: egui::Vec2) -> Harness<'static, DotCraftApp> {
    let mut first = true;
    let mut frame = eframe::Frame::_new_kittest();
    let mut h = Harness::builder().with_size(size).build_ui_state(
        move |ui, app: &mut DotCraftApp| {
            if std::mem::take(&mut first) {
                rc_kit::theme::install_fonts(ui.ctx());
                return;
            }
            let ctx = ui.ctx().clone();
            eframe::App::logic(app, &ctx, &mut frame);
            let bounds = ctx.content_rect();
            let mut root = ui.new_child(egui::UiBuilder::new().max_rect(bounds));
            root.set_clip_rect(bounds);
            eframe::App::ui(app, &mut root, &mut frame);
        },
        app,
    );
    h.run_steps(4);
    h
}

#[test]
#[ignore = "Requires a GPU; writes window captures to target/snapshots"]
fn screens() {
    let _home = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/snapshots");
    std::fs::create_dir_all(&out).unwrap();
    for dark in [true, false] {
        let home = tempfile::tempdir().unwrap();
        let suffix = if dark { "dark" } else { "light" };
        // Welcome screen with a couple of recent projects.
        let mut app = test_app(home.path(), dark);
        app.settings.recent_workspaces = vec![fixtures().join("Shop/Shop.sln"), fixtures().join("TenProjects/TenProjects.slnx")];
        let mut h = harness(app, vec2(1280.0, 800.0));
        h.render().unwrap().save(out.join(format!("welcome-{suffix}.png"))).unwrap();

        // A solution with files open.
        let mut app = test_app(home.path(), dark);
        app.open_workspace(&fixtures().join("Shop/Shop.sln"));
        app.explorer.expanded.insert(format!("proj:{}", fixtures().join("Shop/src/Shop.Api/Shop.Api.csproj").display()));
        app.explorer.expanded.insert(format!("deps:{}", fixtures().join("Shop/src/Shop.Api/Shop.Api.csproj").display()));
        app.open_file(&fixtures().join("Shop/src/Shop.Api/Program.cs"));
        app.open_file(&fixtures().join("Shop/src/Shop.Api/Controllers/WeatherForecastController.cs"));
        app.open_file(&fixtures().join("Shop/src/Shop.Api/Shop.Api.csproj"));
        app.docs.activate(1);
        app.locate_active();
        let mut h = harness(app, vec2(1440.0, 900.0));
        h.run_steps(4);
        h.render().unwrap().save(out.join(format!("workspace-{suffix}.png"))).unwrap();
    }
}

/// Phase 1 closure: a 10-project solution opens with every project in the explorer, and the
/// session (open tabs, caret, folds, tool windows, expanded nodes) is restored on reopen.
#[test]
fn ten_project_solution_opens_and_its_session_is_restored() {
    use egui_kittest::kittest::Queryable;
    let _home = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    let sln = fixtures().join("TenProjects/TenProjects.slnx");
    let file_a = fixtures().join("TenProjects/src/Module03/Class1.cs");
    let file_b = fixtures().join("TenProjects/src/Module07/Class1.cs");

    let mut app = test_app(home.path(), true);
    app.open_workspace(&sln);
    let w = app.workspace.as_ref().unwrap();
    assert_eq!(w.solution.as_ref().unwrap().projects().len(), 10);
    app.open_file(&file_a);
    app.open_file(&file_b);
    app.docs.list[0].editor.goto(2, 8);
    app.dock.toggle(crate::toolwindow::ToolWindow::Problems);
    app.docs.activate(0);
    let mut h = harness(app, vec2(1280.0, 800.0));
    // Every project is listed under the expanded "src" solution folder.
    for i in 1..=10 {
        assert!(h.query_all_by_label(&format!("Module{i:02}")).count() >= 1, "Module{i:02} not in the explorer");
    }
    h.run_steps(2);
    let mut app = h.into_state();
    app.close_workspace();

    let mut app = test_app(home.path(), true);
    app.open_workspace(&sln);
    assert_eq!(app.docs.list.len(), 2);
    assert_eq!(app.docs.active, Some(0));
    assert_eq!(app.docs.list[0].path, file_a.canonicalize().unwrap());
    assert_eq!(app.docs.list[0].editor.caret_line_col(), (2, 8));
    assert!(app.dock.is_open(crate::toolwindow::ToolWindow::Problems));
    assert!(app.explorer.expanded.contains("sf:/src"));
}

fn run_frames(h: &mut Harness<'static, DotCraftApp>, what: &str, mut done: impl FnMut(&DotCraftApp) -> bool) {
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(240) {
        h.run_steps(1);
        if done(h.state()) {
            h.run_steps(3);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    panic!("timed out waiting for {what}");
}

#[test]
#[ignore = "Requires a GPU and the .NET SDK; writes tool window captures to target/snapshots"]
fn screens_tools() {
    let _home = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/snapshots");
    std::fs::create_dir_all(&out).unwrap();
    let home = tempfile::tempdir().unwrap();
    let mut app = test_app(home.path(), true);
    app.settings.roslyn_path = Some("/nonexistent".into());
    app.open_workspace(&fixtures().join("Shop/Shop.sln"));
    let test_file = std::fs::read_dir(fixtures().join("Shop/tests/Shop.Core.XunitTests")).unwrap().flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|e| e == "cs")).unwrap();
    app.open_file(&test_file);
    let mut h = harness(app, vec2(1440.0, 900.0));
    // Tests.
    {
        let app = h.state_mut();
        app.runner.selected = app.runner.configs.iter().position(|c| c.name == "All Tests").unwrap();
        let outs = app.runner.run_selected();
        crate::ui::run_windows::handle_outputs(app, outs);
    }
    run_frames(&mut h, "tests", |a| a.runner.tests.as_ref().is_some_and(|t| !t.running));
    h.render().unwrap().save(out.join("tests-dark.png")).unwrap();
    // Build.
    {
        let app = h.state_mut();
        let target = app.workspace.as_ref().unwrap().ws.key_path().to_path_buf();
        let outs = app.runner.build(&target, rc_dotnet::BuildAction::Build);
        crate::ui::run_windows::handle_outputs(app, outs);
    }
    run_frames(&mut h, "build", |a| a.runner.build.as_ref().is_some_and(|b| b.result.is_some()));
    h.render().unwrap().save(out.join("build-dark.png")).unwrap();
    // Run Shop.Cli.
    {
        let app = h.state_mut();
        app.runner.selected = app.runner.configs.iter().position(|c| c.name.starts_with("Shop.Cli")).unwrap();
        let outs = app.runner.run_selected();
        crate::ui::run_windows::handle_outputs(app, outs);
    }
    run_frames(&mut h, "run", |a| a.runner.runs.first().is_some_and(|r| r.exit.is_some()));
    h.render().unwrap().save(out.join("run-dark.png")).unwrap();
    // Search Everywhere and the find bar.
    {
        let app = h.state_mut();
        app.pending.push(rc_editor::Action::GotoFile);
    }
    h.run_steps(2);
    h.state_mut().search_everywhere.as_mut().unwrap().text = "pricalc".into();
    h.run_steps(4);
    h.render().unwrap().save(out.join("search-dark.png")).unwrap();
    {
        let app = h.state_mut();
        app.search_everywhere = None;
        app.find_bar = Some(crate::search::FindBar { query: "Assert".into(), replace: Some("Check".into()), focus: true, ..Default::default() });
    }
    h.run_steps(4);
    h.render().unwrap().save(out.join("findbar-dark.png")).unwrap();
    // Terminal.
    {
        let app = h.state_mut();
        app.find_bar = None;
        app.open_terminal(None);
    }
    run_frames(&mut h, "terminal", |a| a.terminals.active().is_some_and(|t| t.session.screen_lines().iter().any(|l| !l.trim().is_empty())));
    h.render().unwrap().save(out.join("terminal-dark.png")).unwrap();
}

#[test]
#[ignore = "Requires a GPU; writes Git captures to target/snapshots"]
fn screens_git() {
    let _home = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/snapshots");
    let home = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    let root = repo.path().canonicalize().unwrap().join("Shop");
    let copy = std::process::Command::new("cp").args(["-R", fixtures().join("Shop").to_str().unwrap(), root.to_str().unwrap()]).status().unwrap();
    assert!(copy.success());
    let git = |args: &[&str]| assert!(std::process::Command::new("git").args(args).current_dir(&root).output().unwrap().status.success());
    git(&["init", "-q", "-b", "main"]);
    git(&["config", "user.name", "Ada Lovelace"]);
    git(&["config", "user.email", "ada@example.com"]);
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "Initial shop"]);
    let calc = root.join("src/Shop.Core/PriceCalculator.cs");
    let text = std::fs::read_to_string(&calc).unwrap();
    std::fs::write(&calc, text.replacen("public", "/// <summary>Prices orders.</summary>\npublic", 1)).unwrap();
    git(&["commit", "-qam", "Document the price calculator"]);
    git(&["checkout", "-qb", "feature/coupons"]);
    std::fs::write(root.join("src/Shop.Core/Coupon.cs"), "namespace Shop.Core;\n\npublic record Coupon(string Code, decimal Percent);\n").unwrap();

    let mut app = test_app(home.path(), true);
    app.settings.roslyn_path = Some("/nonexistent".into());
    app.open_workspace(&root.join("Shop.sln"));
    app.open_file(&calc);
    let doc = app.docs.active_doc_mut().unwrap();
    let t = doc.editor.buffer.rope().to_string();
    let at = doc.editor.buffer.rope().byte_to_char(t.find("{").unwrap() + 1);
    doc.editor.replace(at, at, "\n    private const decimal FreeShippingFrom = 50m;");
    app.save_all_docs();
    app.dock.show(crate::toolwindow::ToolWindow::Commit);
    app.dock.show(crate::toolwindow::ToolWindow::Git);
    app.vcs.message = "Add coupons".into();
    let mut h = harness(app, vec2(1440.0, 900.0));
    run_frames(&mut h, "status", |a| a.vcs.changes().len() >= 2 && !a.vcs.log.is_empty() && !a.docs.active_doc().unwrap().editor.line_changes.is_empty());
    let first = h.state().vcs.log[0].hash.clone();
    h.state_mut().vcs.select_commit(&first);
    run_frames(&mut h, "commit files", |a| !a.vcs.commit_files.is_empty());
    h.render().unwrap().save(out.join("git-dark.png")).unwrap();
    {
        let app = h.state_mut();
        let (path, text) = (calc.clone(), app.docs.active_doc().unwrap().editor.buffer.rope().to_string());
        app.vcs.toggle_blame(&path, text, &mut app.docs);
    }
    run_frames(&mut h, "blame", |a| a.docs.active_doc().unwrap().editor.annotations.is_some());
    h.render().unwrap().save(out.join("blame-dark.png")).unwrap();
    h.state_mut().vcs.open_diff("src/Shop.Core/PriceCalculator.cs");
    run_frames(&mut h, "diff", |a| a.vcs.diff.is_some());
    h.render().unwrap().save(out.join("diff-dark.png")).unwrap();
}
