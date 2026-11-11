//! Offscreen captures of the whole window (wgpu, no input sent to any real window), for
//! visual review and the README. Run with `cargo test -p rc-app screens -- --ignored`;
//! images go to `target/snapshots/`.

use crate::app::DotCraftApp;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
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
        // Long Spanish labels wrap inside the buttons.
        rc_kit::i18n::set_lang(rc_kit::i18n::Lang::Es);
        h.run_steps(3);
        h.render().unwrap().save(out.join(format!("welcome-es-{suffix}.png"))).unwrap();
        rc_kit::i18n::set_lang(rc_kit::i18n::Lang::En);
        // New Solution.
        h.state_mut().new_project_dialog(None);
        h.run_steps(4);
        h.render().unwrap().save(out.join(format!("new-solution-{suffix}.png"))).unwrap();

        // A file on its own, without a workspace (Finder's "Open With").
        let mut app = test_app(home.path(), dark);
        app.open_path(&fixtures().join("Shop/src/Shop.Api/appsettings.json"));
        assert!(app.workspace.is_none());
        let mut h = harness(app, vec2(1280.0, 800.0));
        h.run_steps(4);
        h.render().unwrap().save(out.join(format!("loose-file-{suffix}.png"))).unwrap();

        // Razor and solution files.
        let samples = tempfile::Builder::new().prefix("dotcraft-razor").tempdir().unwrap();
        let razor = samples.path().join("Orders.razor");
        std::fs::write(&razor, "@page \"/orders\"\n@using Shop.Core\n@inject OrderService Orders\n\n<PageTitle>Orders</PageTitle>\n\n<h1 class=\"title\">Orders &amp; quotes</h1>\n\n@if (orders is null)\n{\n    <p><em>Loading…</em></p>\n}\nelse\n{\n    <table class=\"table\">\n        @foreach (var order in orders)\n        {\n            <tr @key=\"order.Id\">\n                <td>@order.Customer</td>\n                <td>@order.Total.ToString(\"0.00\")</td>\n                <td><button class=\"btn\" @onclick=\"() => Remove(order)\">Remove</button></td>\n            </tr>\n        }\n    </table>\n}\n\n@code {\n    private List<Order>? orders;\n\n    protected override async Task OnInitializedAsync()\n    {\n        // Load the sample orders.\n        orders = await Orders.GetAllAsync();\n    }\n\n    private void Remove(Order order) => orders?.Remove(order);\n}\n").unwrap();
        let mut app = test_app(home.path(), dark);
        let sln = fixtures().join("Shop/Shop.sln");
        app.open_path(&sln);
        app.open_file(&sln);
        app.open_path(&razor);
        let mut h = harness(app, vec2(1280.0, 800.0));
        h.run_steps(6);
        h.render().unwrap().save(out.join(format!("razor-{suffix}.png"))).unwrap();
        let i = h.state().docs.index_of(&sln).unwrap();
        h.state_mut().docs.activate(i);
        h.run_steps(6);
        h.render().unwrap().save(out.join(format!("sln-{suffix}.png"))).unwrap();

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
        // Settings (appearance page, with the zoom slider).
        let s = h.state().settings.clone();
        h.state_mut().dialog = Some(crate::ui::dialogs::Dialog::settings(&s));
        h.run_steps(4);
        h.render().unwrap().save(out.join(format!("settings-{suffix}.png"))).unwrap();
        // The Tests window before any run: it offers to run them all.
        h.state_mut().dialog = None;
        h.state_mut().dock.show(crate::toolwindow::ToolWindow::Tests);
        h.run_steps(4);
        h.render().unwrap().save(out.join(format!("tests-empty-{suffix}.png"))).unwrap();
        // A compound run configuration.
        let members = h.state().runner.app_names().into_iter().map(|n| (n, true)).collect();
        h.state_mut().dialog = Some(crate::ui::dialogs::Dialog::Compound { old: None, name: "All Applications".into(), members });
        h.run_steps(4);
        h.render().unwrap().save(out.join(format!("compound-{suffix}.png"))).unwrap();
        h.state_mut().dialog = None;
        // Editing an application run configuration.
        let api = h.state().runner.configs.iter().find(|c| c.name.starts_with("Shop.Api")).cloned().unwrap();
        if let crate::runner::ConfigKind::App { project, .. } = api.kind {
            let options = rc_core::session::RunOptions { args: "--seed 42".into(), env: "ASPNETCORE_ENVIRONMENT=Staging".into(), ..Default::default() };
            h.state_mut().dialog = Some(crate::ui::dialogs::Dialog::RunOptions { name: api.name.clone(), project, profiles: vec!["http".into(), "https".into()], options, web: true });
            h.run_steps(4);
            h.render().unwrap().save(out.join(format!("run-options-{suffix}.png"))).unwrap();
            h.state_mut().dialog = None;
        }
        // Add › API Controller.
        h.state_mut().dialog = Some(crate::ui::dialogs::Dialog::NewFile { dir: fixtures().join("Shop/src/Shop.Api/Controllers"), name: "Customers".into(), kind: crate::item_templates::ItemKind::ApiController, error: None });
        h.run_steps(4);
        h.render().unwrap().save(out.join(format!("add-item-{suffix}.png"))).unwrap();
        h.state_mut().dialog = None;
        // File › New File.
        h.state_mut().dialog = Some(crate::ui::dialogs::Dialog::NewFile { dir: fixtures().join("Shop/src/Shop.Api"), name: "Properties/notes.md".into(), kind: crate::item_templates::ItemKind::File, error: None });
        h.run_steps(4);
        h.render().unwrap().save(out.join(format!("new-file-{suffix}.png"))).unwrap();
        h.state_mut().dialog = None;
        // Many tabs: one scrolling row, then several rows, with a pinned and a preview tab.
        {
            let app = h.state_mut();
            for f in ["Shop/src/Shop.Core/Order.cs", "Shop/src/Shop.Core/OrderLine.cs", "Shop/src/Shop.Core/PriceCalculator.cs", "Shop/src/Shop.Api/Controllers/OrdersController.cs", "Shop/src/Shop.Api/appsettings.json", "Shop/tests/Shop.Core.XunitTests/PriceCalculatorTests.cs", "Shop/src/Shop.Cli/Program.cs", "Shop/Shop.sln"] {
                app.open_file(&fixtures().join(f));
            }
            let i = app.docs.index_of(&fixtures().join("Shop/src/Shop.Core/Order.cs").canonicalize().unwrap()).unwrap();
            app.docs.list[i].pinned = true;
            app.docs.move_tab(i, 0);
            app.docs.open_preview(&fixtures().join("Shop/src/Shop.Core/Shop.Core.csproj")).unwrap();
        }
        h.run_steps(4);
        h.render().unwrap().save(out.join(format!("tabs-{suffix}.png"))).unwrap();
        h.state_mut().settings.tabs_multirow = true;
        h.run_steps(4);
        h.render().unwrap().save(out.join(format!("tabs-rows-{suffix}.png"))).unwrap();
        h.state_mut().settings.tabs_multirow = false;
        // Right-click in the editor: the context menu.
        let calc = fixtures().join("Shop/src/Shop.Core/PriceCalculator.cs");
        h.state_mut().open_file(&calc);
        h.run_steps(3);
        let at = egui::pos2(760.0, 300.0);
        h.event(egui::Event::PointerMoved(at));
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed: true, modifiers: Default::default() });
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed: false, modifiers: Default::default() });
        h.run_steps(4);
        h.render().unwrap().save(out.join(format!("editor-menu-{suffix}.png"))).unwrap();
        h.event(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
        h.run_steps(2);
        // ⌘ over a name: a link; Back is enabled in the toolbar.
        h.state_mut().push_nav_at(calc.canonicalize().unwrap(), 0);
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::COMMAND));
        h.event(egui::Event::PointerMoved(egui::pos2(640.0, 320.0)));
        h.run_steps(3);
        h.render().unwrap().save(out.join(format!("navigation-{suffix}.png"))).unwrap();
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
        h.state_mut().nav_back.clear();
        h.run_steps(2);
        // Reformat Code on a JSON file.
        let json = samples.path().join("settings.json");
        std::fs::write(&json, "{\"Logging\":{\"LogLevel\":{\"Default\":\"Information\"}},\"AllowedHosts\":\"*\",\"Ports\":[5000,5001]}\n").unwrap();
        h.state_mut().open_file(&json);
        h.state_mut().pending.push(rc_editor::Action::Reformat);
        h.run_steps(4);
        let formatted = h.state().docs.active_doc().unwrap().editor.buffer.rope().to_string();
        assert!(formatted.contains("\"LogLevel\": {\n"), "{formatted}");
        h.render().unwrap().save(out.join(format!("json-format-{suffix}.png"))).unwrap();
        // Markdown: editor and preview side by side, then the preview alone.
        let md = samples.path().join("README.md");
        std::fs::create_dir_all(samples.path().join("docs")).unwrap();
        std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/icon-64.png"), samples.path().join("docs/icon.png")).unwrap();
        std::fs::write(
            &md,
            "# Shop\n\nSample solution for **DotCraft**: an API, a CLI and *three* test projects.\n\n![icon](docs/icon.png)\n\n## Projects\n\n- `Shop.Core`: orders and prices\n- `Shop.Api`: ASP.NET Core minimal API\n  - Swagger at `/swagger`\n- [ ] Blazor front end\n- [x] Tests\n\n## Run\n\n```bash\ndotnet run --project src/Shop.Api\n```\n\n| Project | Framework |\n| --- | --- |\n| Shop.Core | net8.0 |\n| Shop.Api | net8.0 |\n\n> Prices include VAT.\n\nSee the [.NET docs](https://learn.microsoft.com/dotnet/).\n",
        )
        .unwrap();
        h.state_mut().open_file(&md);
        h.run_steps(6);
        h.render().unwrap().save(out.join(format!("markdown-{suffix}.png"))).unwrap();
        if let Some(d) = h.state_mut().docs.active_doc_mut() {
            d.md_view = Some(crate::docs::MdView::Preview);
        }
        h.run_steps(4);
        h.render().unwrap().save(out.join(format!("markdown-preview-{suffix}.png"))).unwrap();
        // NuGet: installed packages with updates from nuget.org, one selected.
        h.state_mut().dock.show(crate::toolwindow::ToolWindow::Nuget);
        h.state_mut().dock.sizes.insert(crate::toolwindow::ToolWindow::Nuget, 420.0);
        h.run_steps(3);
        h.state_mut().nuget.select_for_test("xunit");
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(25) && !h.state().nuget.ready_for_test() {
            h.run_steps(1);
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        h.run_steps(3);
        h.render().unwrap().save(out.join(format!("nuget-{suffix}.png"))).unwrap();
        h.state_mut().dock.bottom = None;
        // Split right, then a tab in its own window.
        {
            let app = h.state_mut();
            let calc = app.docs.index_of(&fixtures().join("Shop/src/Shop.Core/PriceCalculator.cs").canonicalize().unwrap()).unwrap();
            let program = app.docs.index_of(&fixtures().join("Shop/src/Shop.Cli/Program.cs").canonicalize().unwrap()).unwrap();
            app.docs.activate(calc);
            let (main, other) = (app.docs.list[calc].editor.id, app.docs.list[program].editor.id);
            app.split = Some(crate::app::Split { vertical: true, main, other, focus_second: true, ratio: 0.5 });
            app.docs.activate(program);
        }
        h.run_steps(5);
        h.render().unwrap().save(out.join(format!("split-{suffix}.png"))).unwrap();
        {
            let app = h.state_mut();
            app.split = None;
            let json = app.docs.list.iter().position(|d| d.name() == "appsettings.json").unwrap();
            let id = app.docs.list[json].editor.id;
            app.floating.push(id);
        }
        h.run_steps(5);
        h.render().unwrap().save(out.join(format!("floating-{suffix}.png"))).unwrap();
        h.state_mut().floating.clear();
        // The explorer moved to the right side.
        h.state_mut().dock.move_to(crate::toolwindow::ToolWindow::Explorer, crate::toolwindow::Side::Right);
        h.run_steps(4);
        h.render().unwrap().save(out.join(format!("explorer-right-{suffix}.png"))).unwrap();
    }
}

/// Phase 1 closure: a 10-project solution opens with every project in the explorer, and the
/// session (open tabs, caret, folds, tool windows, expanded nodes) is restored on reopen.
/// Close Project, quit, start: the welcome screen. Quitting with a project open reopens it.
#[test]
fn closed_projects_are_not_reopened_on_startup() {
    let _home = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    let sln = fixtures().join("Shop/Shop.sln");
    let mut app = test_app(home.path(), true);
    app.open_workspace(&sln);
    assert_eq!(crate::app::startup_paths(&app.settings, Vec::new()), vec![sln.clone()]);
    app.close_project();
    assert!(crate::app::startup_paths(&rc_core::Settings::load(), Vec::new()).is_empty());
    assert_eq!(rc_core::Settings::load().recent_workspaces.first(), Some(&sln));
    app.open_workspace(&sln);
    let mut settings = rc_core::Settings::load();
    assert_eq!(crate::app::startup_paths(&settings, Vec::new()), vec![sln.clone()]);
    settings.reopen_last = false;
    assert!(crate::app::startup_paths(&settings, Vec::new()).is_empty());
    // Files from Finder win.
    let file = fixtures().join("Shop/README.md");
    assert_eq!(crate::app::startup_paths(&settings, vec![file.clone()]), vec![file]);
}

#[test]
fn files_dropped_on_the_welcome_screen_open_in_the_editor() {
    let _home = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    let file = home.path().join("cosa.txt");
    std::fs::write(&file, "hola").unwrap();
    let mut h = harness(test_app(home.path(), true), vec2(1280.0, 800.0));
    #[derive(Debug)]
    struct Dropped(PathBuf);
    impl egui::DroppedFile for Dropped {
        fn path(&self) -> &Path {
            &self.0
        }
        fn bytes(&self) -> Result<Vec<u8>, String> {
            std::fs::read(&self.0).map_err(|e| e.to_string())
        }
    }
    h.input_mut().dropped_files.push(std::sync::Arc::new(Dropped(file.clone())));
    h.run_steps(3);
    let app = h.state();
    assert!(app.workspace.is_none());
    assert_eq!(app.docs.active_doc().map(|d| d.path.clone()), Some(file.canonicalize().unwrap()));
    assert!(app.notices.iter().all(|n| !n.error), "{:?}", app.notices.iter().map(|n| &n.text).collect::<Vec<_>>());
}

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
#[ignore = "Requires a GPU; writes recent-files captures to target/snapshots"]
fn screens_recent_and_reload() {
    let _home = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/snapshots");
    std::fs::create_dir_all(&out).unwrap();
    let home = tempfile::tempdir().unwrap();
    let notes = tempfile::Builder::new().prefix("dotcraft-notes").tempdir().unwrap();
    let md = notes.path().canonicalize().unwrap().join("notes.md");
    std::fs::write(&md, "# Notes\n\nFirst version.\n").unwrap();
    // A solution, and a Markdown file from another folder (outside the watched workspace).
    let mut app = test_app(home.path(), true);
    app.open_workspace(&fixtures().join("Shop/Shop.sln"));
    app.open_file(&fixtures().join("Shop/src/Shop.Core/Order.cs"));
    app.open_path(&md);
    assert_eq!(app.settings.recent_files.first(), Some(&md));
    assert_eq!(app.settings.recent_files.get(1), Some(&fixtures().join("Shop/src/Shop.Core/Order.cs")));
    let mut h = harness(app, vec2(1280.0, 800.0));
    // Another app changes the file: it reloads without closing it.
    std::fs::write(&md, "# Notes\n\nChanged by another app.\n").unwrap();
    let i = h.state().docs.index_of(&md).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !h.state().docs.list[i].editor.buffer.rope().to_string().contains("another app") && std::time::Instant::now() < deadline {
        h.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(h.state().docs.list[i].editor.buffer.rope().to_string().contains("another app"));
    // The project menu lists the recent files (Windows and Linux have no native File menu).
    h.state_mut().docs.close(0);
    h.run_steps(2);
    h.get_all_by_label("Shop").next().unwrap().click();
    h.run_steps(4);
    h.render().unwrap().save(out.join("recent-files-dark.png")).unwrap();
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

#[test]
#[ignore = "Requires a GPU, the .NET SDK and the Roslyn server in target/test-home; writes captures"]
fn screens_insight() {
    let _home = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/snapshots");
    let home = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-home").canonicalize().unwrap();
    let mut app = test_app(&home, true);
    let repo = tempfile::tempdir().unwrap();
    let root = repo.path().canonicalize().unwrap().join("Shop");
    assert!(std::process::Command::new("cp").args(["-R", fixtures().join("Shop").to_str().unwrap(), root.to_str().unwrap()]).status().unwrap().success());
    app.open_workspace(&root.join("Shop.sln"));
    let program = root.join("src/Shop.Cli/Program.cs");
    app.open_file(&program);
    let mut h = harness(app, vec2(1440.0, 900.0));
    run_frames(&mut h, "Roslyn", |a| a.insight.status == crate::insight::InsightStatus::Ready && a.docs.active_doc().is_some_and(|d| !d.editor.semantic.is_empty()));
    if std::env::var_os("DOTCRAFT_DEBUG_DIAGS").is_some() {
        let d = h.state().docs.active_doc().unwrap();
        for x in &d.editor.diagnostics {
            eprintln!("DIAG {:?} {:?} {} {:?} unnecessary={}", d.editor.buffer.line_col(x.start), x.severity, x.message, x.code, x.unnecessary);
        }
        eprintln!("SEM0 {:?}", d.editor.semantic.first());
    }
    // Completion after typing "calculator." on a new line inside the file.
    {
        let app = h.state_mut();
        let doc = app.docs.active_doc_mut().unwrap();
        let text = doc.editor.buffer.rope().to_string();
        let line = text.lines().position(|l| l.contains("Calculate(")).unwrap_or(5);
        let pos = doc.editor.buffer.pos_at(line + 1, 0);
        doc.editor.replace(pos, pos, "var c = new PriceCalculator();\nc.\n");
        doc.editor.selection = rc_text::Selection::point(pos + "var c = new PriceCalculator();\nc.".len());
        let i = app.docs.active.unwrap();
        app.insight.on_editor_events(&mut app.docs.list[i], &[rc_editor::EditorEvent::Typed('.')]);
    }
    run_frames(&mut h, "completion", |a| a.docs.active_doc().and_then(|d| d.editor.popups.completion.as_ref()).is_some_and(|c| !c.filtered.is_empty() && c.doc.is_some()));
    h.render().unwrap().save(out.join("completion-dark.png")).unwrap();
    // Parameter info inside the call.
    {
        let app = h.state_mut();
        let doc = app.docs.active_doc_mut().unwrap();
        doc.editor.popups.close_all();
        let caret = doc.editor.caret();
        doc.editor.replace(caret, caret, "Calculate(");
        let i = app.docs.active.unwrap();
        app.insight.on_editor_events(&mut app.docs.list[i], &[rc_editor::EditorEvent::Typed('(')]);
    }
    run_frames(&mut h, "signature", |a| a.docs.active_doc().is_some_and(|d| d.editor.popups.signature.is_some()));
    h.render().unwrap().save(out.join("signature-dark.png")).unwrap();
    // Hover over a type, with the broken call's diagnostic.
    {
        let app = h.state_mut();
        let doc = app.docs.active_doc_mut().unwrap();
        doc.editor.popups.close_all();
        let text = doc.editor.buffer.rope().to_string();
        let pos = doc.editor.buffer.rope().byte_to_char(text.find("PriceCalculator()").unwrap() + 3);
        let i = app.docs.active.unwrap();
        app.insight.on_editor_events(&mut app.docs.list[i], &[rc_editor::EditorEvent::HoverAt { pos, screen: egui::pos2(700.0, 300.0) }]);
    }
    run_frames(&mut h, "hover", |a| a.docs.active_doc().is_some_and(|d| d.editor.popups.hover.is_some()));
    h.render().unwrap().save(out.join("hover-dark.png")).unwrap();
    // Context actions.
    {
        let app = h.state_mut();
        app.docs.active_doc_mut().unwrap().editor.popups.close_all();
        app.caret_screen = Some(egui::pos2(500.0, 320.0));
        app.run_insight_action(rc_editor::Action::ShowIntentions, egui::pos2(500.0, 320.0));
    }
    run_frames(&mut h, "actions", |a| matches!(a.dialog, Some(crate::ui::dialogs::Dialog::CodeActions { .. })) || a.notices.iter().any(|n| n.text.contains("context")));
    h.render().unwrap().save(out.join("actions-dark.png")).unwrap();
    h.state_mut().insight.stop();
}

/// The README's main capture: Roslyn colours and inlay hints, the explorer and a finished run.
#[test]
#[ignore = "Requires a GPU, the .NET SDK and Roslyn in target/test-home; writes target/snapshots/readme-*.png"]
fn screens_readme() {
    let _home = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/snapshots");
    let roslyn = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-home/tools/roslyn").canonicalize().expect("run the rc-lsp tests first");
    let roslyn = std::fs::read_dir(&roslyn).unwrap().flatten().map(|e| e.path().join("Microsoft.CodeAnalysis.LanguageServer")).find(|p| p.exists()).unwrap();
    let shop = fixtures().join("Shop");
    let calc = shop.join("src/Shop.Core/PriceCalculator.cs");
    for dark in [true, false] {
        let home = tempfile::tempdir().unwrap();
        let mut app = test_app(home.path(), dark);
        app.settings.roslyn_path = Some(roslyn.clone());
        app.open_workspace(&shop.join("Shop.sln"));
        app.explorer.expanded.insert(format!("proj:{}", shop.join("src/Shop.Core/Shop.Core.csproj").display()));
        app.open_file(&shop.join("src/Shop.Core/Order.cs"));
        app.open_file(&calc);
        app.locate_active();
        app.docs.active_doc_mut().unwrap().editor.goto(26, 12);
        let mut h = harness(app, vec2(1440.0, 900.0));
        run_frames(&mut h, "Roslyn", |a| {
            a.insight.status == crate::insight::InsightStatus::Ready && a.docs.active_doc().is_some_and(|d| !d.editor.semantic.is_empty() && !d.editor.inlays.is_empty()) && a.vcs.branch_name().is_some() && !a.runner.configs.is_empty()
        });
        {
            let app = h.state_mut();
            app.runner.selected = app.runner.configs.iter().position(|c| c.name.starts_with("Shop.Cli")).unwrap();
            app.pending.push(rc_editor::Action::Run);
        }
        run_frames(&mut h, "Shop.Cli", |a| a.runner.runs.first().is_some_and(|r| r.exit.is_some()));
        h.run_steps(10);
        let suffix = if dark { "dark" } else { "light" };
        h.render().unwrap().save(out.join(format!("readme-workspace-{suffix}.png"))).unwrap();
        h.state_mut().insight.stop();
    }
}

#[test]
fn solution_filter_explorer_shows_only_selected_real_projects() {
    let _home = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    let filter = home.path().join("Selected.slnf");
    let root = fixtures().join("TenProjects");
    let json = serde_json::json!({"solution": {
        "path": root.join("TenProjects.slnx"),
        "projects": ["src/Module01/Module01.csproj", "src/Module10/Module10.csproj"]
    }});
    std::fs::write(&filter, json.to_string()).unwrap();
    let solution = rc_dotnet::Solution::load(&filter).unwrap();
    let mut app = test_app(home.path(), true);
    // Exercise the real explorer without starting SDK downloads or project processes.
    app.explorer.expanded.extend(["sln".to_string(), "sf:/src".to_string()]);
    app.workspace = Some(crate::app::WorkspaceState {
        ws: rc_core::Workspace::open(&filter).unwrap(),
        solution: Some(solution),
    });
    let mut h = harness(app, vec2(1000.0, 700.0));
    h.run_steps(3);
    assert!(h.query_by_label("Module01").is_some());
    assert!(h.query_by_label("Module10").is_some());
    assert!(h.query_by_label("Module02").is_none());
    assert!(h.query_by_label("Module09").is_none());
}

#[test]
fn solution_filter_open_route_uses_source_root_and_watches_both_locations() {
    let _home = HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = tempfile::tempdir().unwrap();
    // Canonical, as the app opens paths (/private/var on macOS).
    let base = home.path().canonicalize().unwrap();
    let source = base.join("source");
    let filters = base.join("filters");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::create_dir_all(&filters).unwrap();
    assert!(std::process::Command::new("git").args(["init", "-q"]).arg(&source).status().unwrap().success());
    for name in ["Selected", "Other"] {
        let directory = source.join(name);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join(format!("{name}.csproj")), "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>net8.0</TargetFramework><OutputType>Exe</OutputType></PropertyGroup></Project>").unwrap();
        std::fs::write(directory.join("Program.cs"), "System.Console.WriteLine(42);\n").unwrap();
    }
    let solution = source.join("Demo.slnx");
    std::fs::write(&solution, "<Solution><Project Path=\"Selected/Selected.csproj\"/><Project Path=\"Other/Other.csproj\"/></Solution>").unwrap();
    let filter = filters.join("Subset.slnf");
    let select = |name: &str| serde_json::json!({ "solution": { "path": "../source/Demo.slnx", "projects": [format!("{name}/{name}.csproj")] } }).to_string();
    std::fs::write(&filter, select("Selected")).unwrap();
    let mut app = test_app(home.path(), true);
    // An existing non-executable dummy prevents fallback to a real installed server.
    let dummy = home.path().join("no-language-server");
    std::fs::write(&dummy, "not an executable").unwrap();
    app.settings.roslyn_path = Some(dummy);
    app.open_path(&filter);
    let state = app.workspace.as_ref().unwrap();
    assert_eq!(state.ws.root, source.canonicalize().unwrap());
    assert_eq!(state.ws.key_path(), filter.canonicalize().unwrap());
    assert_eq!(state.solution.as_ref().unwrap().source_path, solution);
    assert_eq!(app.settings.last_workspace.as_deref(), Some(filter.as_path()));
    assert_eq!(app.solution_projects(), vec![source.join("Selected/Selected.csproj")]);
    assert!(app.runner.configs.iter().any(|config| config.name == "Selected"));
    assert!(!app.runner.configs.iter().any(|config| config.name == "Other"));
    app.explorer.expanded.insert("sln".into());
    let mut harness = harness(app, vec2(1000.0, 700.0));
    let wait = |harness: &mut Harness<'_, DotCraftApp>, done: &dyn Fn(&DotCraftApp) -> bool| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !done(harness.state()) {
            harness.run_steps(1);
            assert!(std::time::Instant::now() < deadline, "solution filter update timed out");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    };
    wait(&mut harness, &|app| app.files.index.as_ref().unwrap().snapshot().get("Selected/Program.cs").is_some());
    wait(&mut harness, &|app| app.vcs.repo.as_ref().is_some_and(|repo| repo.root() == source));
    // The source-root watcher patches the index even before a source file is opened.
    let added = source.join("Selected/Added.cs");
    std::fs::write(&added, "class Added {}\n").unwrap();
    wait(&mut harness, &|app| app.files.index.as_ref().unwrap().snapshot().get("Selected/Added.cs").is_some());
    let open = source.join("Selected/Program.cs");
    harness.state_mut().open_file(&open);
    harness.state_mut().docs.active_doc_mut().unwrap().editor.replace(0, 0, "// unsaved\n");
    // The external filter watcher changes both explorer selection and run configurations.
    std::fs::write(&filter, select("Other")).unwrap();
    wait(&mut harness, &|app| app.solution_projects() == vec![source.join("Other/Other.csproj")] && app.runner.configs.iter().any(|config| config.name == "Other"));
    assert!(!harness.state().runner.configs.iter().any(|config| config.name == "Selected"));
    harness.run_steps(2);
    assert!(harness.query_by_label("Other").is_some());
    // The dirty editor's breadcrumbs may still name Selected; inspect explorer rows only.
    {
        let app = harness.state_mut();
        let workspace = app.workspace.as_ref().unwrap();
        let rows = app.explorer.rows(workspace.solution.as_ref(), &workspace.ws.root, &workspace.ws.name());
        assert!(rows.iter().any(|row| row.label == "Other"));
        assert!(!rows.iter().any(|row| row.label == "Selected"));
    }
    assert!(harness.state().docs.active_doc().unwrap().is_modified());
    assert!(harness.state().docs.active_doc().unwrap().editor.buffer.rope().to_string().starts_with("// unsaved\n"));
    // The underlying solution file is observed too; adding a folder reloads its structure.
    std::fs::write(&solution, "<Solution><Project Path=\"Selected/Selected.csproj\"/><Folder Name=\"/Changed/\"><Project Path=\"Other/Other.csproj\"/></Folder></Solution>").unwrap();
    wait(&mut harness, &|app| matches!(app.workspace.as_ref().unwrap().solution.as_ref().unwrap().items.first(), Some(rc_dotnet::solution::SolutionItem::Folder { name, .. }) if name == "Changed"));
    assert_eq!(harness.state().workspace.as_ref().unwrap().ws.key_path(), filter);
    let alternate = base.join("alternate");
    std::fs::create_dir_all(alternate.join("Moved")).unwrap();
    std::fs::write(alternate.join("Moved/Moved.csproj"), "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>net8.0</TargetFramework></PropertyGroup></Project>").unwrap();
    std::fs::write(alternate.join("Moved/Program.cs"), "class Moved {}\n").unwrap();
    std::fs::write(alternate.join("Demo.slnx"), "<Solution><Project Path=\"Moved/Moved.csproj\"/></Solution>").unwrap();
    std::fs::write(&filter, serde_json::json!({ "solution": { "path": "../alternate/Demo.slnx", "projects": ["Moved/Moved.csproj"] } }).to_string()).unwrap();
    wait(&mut harness, &|app| app.workspace.as_ref().unwrap().ws.root == alternate && app.files.index.as_ref().unwrap().snapshot().get("Moved/Program.cs").is_some());
    assert!(harness.state().files.index.as_ref().unwrap().snapshot().get("Selected/Program.cs").is_none());
    assert_eq!(harness.state().workspace.as_ref().unwrap().ws.key_path(), filter);
    assert!(harness.state().docs.active_doc().unwrap().is_modified(), "changing source roots must preserve unsaved editors");
    harness.state_mut().close_workspace();
    assert!(harness.state().files.index.is_none());
}

#[test]
#[ignore = "Requires a GPU; writes target/snapshots/show-all-files-dark.png"]
fn screens_show_all_files() {
    let _home = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/snapshots");
    std::fs::create_dir_all(&out).unwrap();
    let home = tempfile::tempdir().unwrap();
    let shop = fixtures().join("Shop");
    std::fs::create_dir_all(shop.join("src/Shop.Core/bin/Debug")).unwrap();
    std::fs::create_dir_all(shop.join("src/Shop.Core/obj")).unwrap();
    let mut app = test_app(home.path(), true);
    app.open_workspace(&shop.join("Shop.sln"));
    app.explorer.expanded.insert(format!("proj:{}", shop.join("src/Shop.Core/Shop.Core.csproj").display()));
    let mut h = harness(app, vec2(1280.0, 800.0));
    h.run_steps(2);
    h.get_by_label("Show All Files").click();
    h.run_steps(3);
    assert!(h.state().explorer.show_all);
    h.render().unwrap().save(out.join("show-all-files-dark.png")).unwrap();
}

#[test]
#[ignore = "Requires a GPU; writes target/snapshots/update-notice-dark.png"]
fn screens_update_notice() {
    let _home = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/snapshots");
    std::fs::create_dir_all(&out).unwrap();
    let home = tempfile::tempdir().unwrap();
    let mut app = test_app(home.path(), true);
    app.update = Some(crate::update::Release { version: "9.0.0".into(), url: "https://github.com/javy-fernandez/dotcraft/releases/tag/v9.0.0".into() });
    let mut h = harness(app, vec2(1280.0, 800.0));
    h.run_steps(3);
    h.render().unwrap().save(out.join("update-notice-dark.png")).unwrap();
    h.get_by_label("Skip This Version").click();
    h.run_steps(2);
    assert!(h.state().update.is_none());
    assert_eq!(h.state().settings.skipped_version.as_deref(), Some("9.0.0"));
    h.state_mut().dialog = Some(crate::ui::dialogs::Dialog::About);
    h.run_steps(3);
    h.render().unwrap().save(out.join("about-dark.png")).unwrap();
}

#[test]
#[ignore = "Requires a GPU; writes target/snapshots/monitoring-*.png"]
fn screens_monitoring() {
    let _home = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/snapshots");
    std::fs::create_dir_all(&out).unwrap();
    let home = tempfile::tempdir().unwrap();
    for (dark, width) in [(true, 1440.0), (false, 900.0)] {
        let mut app = test_app(home.path(), dark);
        app.open_workspace(&fixtures().join("Shop/Shop.sln"));
        app.dock.show(crate::toolwindow::ToolWindow::Monitoring);
        app.dock.sizes.insert(crate::toolwindow::ToolWindow::Monitoring, 300.0);
        // A minute and a half of a web app warming up, then allocating steadily.
        app.monitor.name = "Shop.Api".into();
        app.monitor.pid = Some(4242);
        app.monitor.running = true;
        for i in 0..90 {
            let t = i as f64;
            let rt = (i % 2 == 0).then(|| crate::monitor::Runtime {
                gc_heap_mb: Some(12.0 + (t * 0.4) % 18.0),
                gen0: Some((t / 3.0).floor()),
                gen1: Some((t / 20.0).floor()),
                gen2: Some((t / 60.0).floor()),
                time_in_gc: Some(if i % 7 == 0 { 3.0 } else { 0.0 }),
                alloc_rate: Some(2.4 * 1024.0 * 1024.0),
                threadpool_threads: Some(6.0),
                exceptions: Some(0.0),
            });
            app.monitor.samples.push_back(crate::monitor::Sample { t, cpu: if i < 10 { 60.0 - t * 4.0 } else { 8.0 + (t * 0.7).sin().abs() * 14.0 }, memory_mb: 60.0 + t.min(30.0) * 1.5 + (t * 0.4) % 18.0, runtime: rt });
        }
        let mut h = harness(app, vec2(width, 800.0));
        h.run_steps(3);
        h.render().unwrap().save(out.join(format!("monitoring-{}.png", if dark { "dark" } else { "light-narrow" }))).unwrap();
    }
}

#[test]
#[ignore = "Requires a GPU and the .NET SDK; runs Shop.Api and writes target/snapshots/monitoring-live-dark.png"]
fn screens_monitoring_live() {
    let _home = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/snapshots");
    std::fs::create_dir_all(&out).unwrap();
    let home = tempfile::tempdir().unwrap();
    let mut app = test_app(home.path(), true);
    app.settings.roslyn_path = Some("/nonexistent".into());
    app.open_workspace(&fixtures().join("Shop/Shop.sln"));
    app.dock.show(crate::toolwindow::ToolWindow::Monitoring);
    app.dock.sizes.insert(crate::toolwindow::ToolWindow::Monitoring, 320.0);
    app.runner.selected = app.runner.configs.iter().position(|c| c.name.starts_with("Shop.Api")).unwrap();
    let outs = app.runner.run_selected();
    crate::ui::run_windows::handle_outputs(&mut app, outs);
    let mut h = harness(app, vec2(1440.0, 900.0));
    // The process, then the runtime's counters through its diagnostic port.
    run_frames(&mut h, "runtime counters", |a| a.monitor.runtime().is_some_and(|r| r.gc_heap_mb.is_some()) && a.monitor.samples.len() >= 6);
    // Running opened the Run window in the same place.
    h.state_mut().dock.show(crate::toolwindow::ToolWindow::Monitoring);
    h.run_steps(3);
    h.render().unwrap().save(out.join("monitoring-live-dark.png")).unwrap();
    let pid = h.state().monitor.app_pid.or(h.state().monitor.pid).unwrap();
    assert!(crate::monitor::has_diagnostic_port(pid));
    let root = h.state().monitor.pid.unwrap();
    // Quitting kills dotnet watch and the app at once (nothing is left running).
    h.state_mut().runner.kill_all_now();
    std::thread::sleep(std::time::Duration::from_millis(500));
    #[cfg(unix)]
    {
        // SAFETY: signal 0 only checks that the process exists.
        let alive = |p: u32| unsafe { libc::kill(p as i32, 0) == 0 };
        assert!(!alive(pid) && !alive(root), "left running: app {} watch {}", alive(pid), alive(root));
    }
    run_frames(&mut h, "exit", |a| !a.monitor.running);
}

#[test]
#[ignore = "Requires a GPU; writes target/snapshots/bookmarks-*.png"]
fn screens_bookmarks() {
    let _home = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/snapshots");
    std::fs::create_dir_all(&out).unwrap();
    let home = tempfile::tempdir().unwrap();
    let mut app = test_app(home.path(), true);
    app.settings.roslyn_path = Some("/nonexistent".into());
    app.open_workspace(&fixtures().join("Shop/Shop.sln"));
    let calc = fixtures().join("Shop/src/Shop.Core/PriceCalculator.cs");
    let order = fixtures().join("Shop/src/Shop.Core/Order.cs");
    app.docs.set_bookmark(&order, 2, Some(Some('2')));
    app.open_file(&calc);
    app.docs.set_bookmark(&calc, 4, Some(None));
    app.docs.set_bookmark(&calc, 8, Some(Some('1')));
    app.dock.show(crate::toolwindow::ToolWindow::Bookmarks);
    let mut h = harness(app, vec2(1280.0, 800.0));
    h.run_steps(3);
    h.render().unwrap().save(out.join("bookmarks-dark.png")).unwrap();
    // ⌃2 jumps to Order.cs.
    {
        let ctx = h.ctx.clone();
        h.state_mut().run_ide_action(&ctx, rc_editor::Action::GotoBookmark(2));
    }
    h.run_steps(2);
    assert_eq!(h.state().docs.active_doc().unwrap().path, order);
    // ⌥F3 picker.
    h.state_mut().dialog = Some(crate::ui::dialogs::Dialog::BookmarkMnemonic { path: order.clone(), line: 2 });
    h.run_steps(3);
    h.render().unwrap().save(out.join("bookmarks-mnemonic-dark.png")).unwrap();
    h.get_by_label("A").click();
    h.run_steps(2);
    assert!(h.state().docs.all_bookmarks().iter().any(|b| b.path == order && b.mnemonic == Some('A')));
}

#[test]
#[ignore = "Requires a GPU; writes target/snapshots/hierarchy-dark.png"]
fn screens_hierarchy() {
    let _home = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/snapshots");
    std::fs::create_dir_all(&out).unwrap();
    let home = tempfile::tempdir().unwrap();
    let mut app = test_app(home.path(), true);
    app.settings.roslyn_path = Some("/nonexistent".into());
    app.open_workspace(&fixtures().join("Shop/Shop.sln"));
    let calc = fixtures().join("Shop/src/Shop.Core/PriceCalculator.cs");
    app.open_file(&calc);
    let uri = |p: &str| rc_lsp::path_to_uri(&fixtures().join(p)).as_str().to_string();
    let item = |name: &str, detail: &str, file: &str, line: u64| serde_json::json!({ "name": name, "detail": detail, "kind": 6, "uri": uri(file), "range": {}, "selectionRange": { "start": { "line": line, "character": 4 }, "end": { "line": line, "character": 8 } } });
    let mut h = crate::hierarchy::Hierarchy::new(crate::hierarchy::HierarchyKind::Calls, vec![item("PriceCalculator.Calculate(Order)", "Shop.Core.PriceCalculator", "Shop/src/Shop.Core/PriceCalculator.cs", 16)], 1);
    h.set_children(0, vec![
        serde_json::json!({ "from": item("OrdersController.Post(QuoteRequest)", "Shop.Api.Controllers.OrdersController", "Shop/src/Shop.Api/Controllers/OrdersController.cs", 30) }),
        serde_json::json!({ "from": item("Program.<Main>$(string[])", "Program", "Shop/src/Shop.Cli/Program.cs", 12) }),
        serde_json::json!({ "from": item("PriceCalculatorTests.AppliesBulkDiscount()", "Shop.Core.XunitTests.PriceCalculatorTests", "Shop/tests/Shop.Core.XunitTests/PriceCalculatorTests.cs", 20) }),
    ]);
    h.nodes[0].expanded = true;
    h.set_children(1, vec![]);
    app.hierarchy = Some(h);
    app.dock.show(crate::toolwindow::ToolWindow::Hierarchy);
    app.dock.sizes.insert(crate::toolwindow::ToolWindow::Hierarchy, 520.0);
    let mut hh = harness(app, vec2(1440.0, 800.0));
    hh.run_steps(3);
    hh.render().unwrap().save(out.join("hierarchy-dark.png")).unwrap();
}

#[test]
#[ignore = "Requires a GPU; writes target/snapshots/local-history-*.png"]
fn screens_local_history() {
    let _home = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/snapshots");
    std::fs::create_dir_all(&out).unwrap();
    let home = tempfile::tempdir().unwrap();
    let dir = tempfile::Builder::new().prefix("dotcraft-history").tempdir().unwrap();
    let file = dunce::canonicalize(dir.path()).unwrap().join("Notes.cs");
    std::fs::write(&file, "class Notes\n{\n    int a = 1;\n}\n").unwrap();
    let mut app = test_app(home.path(), true);
    app.open_workspace(file.parent().unwrap());
    app.open_file(&file);
    // Two saves and a change by another app.
    for text in ["class Notes\n{\n    int a = 2;\n}\n", "class Notes\n{\n    int a = 2;\n    int b = 3;\n}\n"] {
        let d = app.docs.active_doc_mut().unwrap();
        let len = d.editor.buffer.len_chars();
        d.editor.replace(0, len, text);
        app.docs.save(0).unwrap();
    }
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&file, "class Notes\n{\n    int a = 2;\n    int b = 3;\n    // edited elsewhere\n}\n").unwrap();
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
    std::fs::File::options().write(true).open(&file).unwrap().set_modified(later).unwrap();
    app.docs.check_disk_changes();
    let revs = rc_core::local_history::revisions(&file);
    assert_eq!(revs.iter().map(|r| r.label.as_str()).collect::<Vec<_>>(), ["External change", "Saved", "Saved", "Original"]);
    app.show_local_history(None);
    let mut h = harness(app, vec2(1280.0, 800.0));
    h.run_steps(3);
    // The original version against the current text.
    let original = crate::ui::toolwindows::format_time(revs[3].time);
    h.get_all_by_label(&original).last().unwrap().click();
    h.run_steps(3);
    assert!(h.state().vcs.diff.is_some());
    h.render().unwrap().save(out.join("local-history-dark.png")).unwrap();
}

#[test]
#[ignore = "Requires a GPU; writes target/snapshots/templates-dark.png"]
fn screens_templates() {
    let _home = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/snapshots");
    std::fs::create_dir_all(&out).unwrap();
    let home = tempfile::tempdir().unwrap();
    let mut app = test_app(home.path(), true);
    app.settings.roslyn_path = Some("/nonexistent".into());
    let dir = tempfile::Builder::new().prefix("dotcraft-templates").tempdir().unwrap();
    let file = dunce::canonicalize(dir.path()).unwrap().join("Cart.cs");
    std::fs::write(&file, "public class Cart\n{\n    public void Checkout()\n    {\n        Pay();\n        Ship();\n    }\n}\n").unwrap();
    app.open_workspace(file.parent().unwrap());
    app.open_file(&file);
    // ⌥⌘T on the two calls: "try" puts them in a try/catch.
    let d = app.docs.active_doc_mut().unwrap();
    let text = d.editor.buffer.rope().to_string();
    let (a, b) = (text.find("        Pay").unwrap(), text.find("    }\n}").unwrap());
    d.editor.selection = rc_text::Selection::single(rc_text::Range::new(a, b));
    app.dialog = Some(crate::ui::dialogs::Dialog::Templates { surround: true, filter: String::new(), selected: 0 });
    let mut h = harness(app, vec2(1280.0, 720.0));
    h.run_steps(3);
    h.render().unwrap().save(out.join("templates-surround-dark.png")).unwrap();
    h.get_by_label("try").click();
    h.run_steps(3);
    let text = h.state().docs.active_doc().unwrap().editor.buffer.rope().to_string();
    assert!(text.contains("        try\n        {\n            Pay();\n            Ship();\n        }\n        catch (Exception e)\n"), "{text}");
    h.render().unwrap().save(out.join("templates-dark.png")).unwrap();
}

#[test]
#[ignore = "Requires a GPU and the .NET SDK; runs Shop.Api and writes target/snapshots/http-*.png"]
fn screens_http_client_and_endpoints() {
    let _home = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/snapshots");
    std::fs::create_dir_all(&out).unwrap();
    let home = tempfile::tempdir().unwrap();
    let mut app = test_app(home.path(), true);
    app.settings.roslyn_path = Some("/nonexistent".into());
    app.open_workspace(&fixtures().join("Shop/Shop.sln"));
    app.runner.selected = app.runner.configs.iter().position(|c| c.name.starts_with("Shop.Api")).unwrap();
    let outs = app.runner.run_selected();
    crate::ui::run_windows::handle_outputs(&mut app, outs);
    let http = fixtures().join("Shop/src/Shop.Api/Shop.Api.http");
    app.open_file(&http);
    let mut h = harness(app, vec2(1440.0, 900.0));
    run_frames(&mut h, "the app to listen", |a| a.runner.runs.iter().any(|r| !r.urls.is_empty()));
    // ▶ on the request: retried until the server answers (it may still be starting).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let i = h.state().docs.active.unwrap();
        h.state_mut().send_http_request(i, 2);
        run_frames(&mut h, "a response", |a| a.http.response.is_some());
        if matches!(&h.state().http.response, Some(Ok(r)) if r.status == 200) || std::time::Instant::now() > deadline {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    let resp = h.state().http.response.clone().unwrap().unwrap();
    assert_eq!(resp.status, 200);
    assert!(resp.text.contains("temperatureC"), "{}", resp.text);
    assert_eq!(h.state().docs.active_doc().unwrap().editor.gutter_icons.get(&2), Some(&rc_editor::GutterIcon::RunApp));
    h.state_mut().dock.show(crate::toolwindow::ToolWindow::HttpClient);
    h.state_mut().dock.sizes.insert(crate::toolwindow::ToolWindow::HttpClient, 380.0);
    h.run_steps(3);
    h.render().unwrap().save(out.join("http-client-dark.png")).unwrap();
    // Endpoints of Shop.Api.
    run_frames(&mut h, "endpoints", |a| !a.endpoints.scanning);
    assert!(h.state().endpoints.projects.iter().any(|(n, _, eps)| n == "Shop.Api" && eps.iter().any(|e| e.route == "/WeatherForecast")));
    h.state_mut().dock.show(crate::toolwindow::ToolWindow::Endpoints);
    h.run_steps(3);
    h.render().unwrap().save(out.join("endpoints-dark.png")).unwrap();
    h.state_mut().runner.kill_all_now();
}

#[test]
#[ignore = "Requires a GPU and the .NET SDK; runs Shop's tests with coverage and writes target/snapshots/coverage-dark.png"]
fn screens_coverage() {
    let _home = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/snapshots");
    std::fs::create_dir_all(&out).unwrap();
    let home = tempfile::tempdir().unwrap();
    let mut app = test_app(home.path(), true);
    app.settings.roslyn_path = Some("/nonexistent".into());
    app.open_workspace(&fixtures().join("Shop/Shop.sln"));
    let calc = fixtures().join("Shop/src/Shop.Core/PriceCalculator.cs");
    app.open_file(&calc);
    app.run_coverage();
    let mut h = harness(app, vec2(1440.0, 900.0));
    run_frames(&mut h, "coverage", |a| !a.coverage.running && (a.coverage.report.is_some() || a.coverage.error.is_some()));
    assert!(h.state().coverage.error.is_none(), "{:?}", h.state().coverage.error);
    let d = h.state().docs.active_doc().unwrap();
    let line = d.editor.buffer.rope().to_string().lines().position(|l| l.contains("subtotal += line.Subtotal")).unwrap();
    assert_eq!(d.editor.coverage.get(&line), Some(&rc_editor::CoverageMark::Covered));
    h.state_mut().coverage.expanded.insert("Shop.Core".into());
    h.state_mut().dock.sizes.insert(crate::toolwindow::ToolWindow::Coverage, 320.0);
    h.run_steps(3);
    h.render().unwrap().save(out.join("coverage-dark.png")).unwrap();
}

#[test]
#[ignore = "Requires a GPU and the .NET SDK; runs Shop.Api and writes target/snapshots/profiler-*.png"]
fn screens_profiler() {
    let _home = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/snapshots");
    std::fs::create_dir_all(&out).unwrap();
    let home = tempfile::tempdir().unwrap();
    let mut app = test_app(home.path(), true);
    app.settings.roslyn_path = Some("/nonexistent".into());
    app.open_workspace(&fixtures().join("Shop/Shop.sln"));
    app.runner.selected = app.runner.configs.iter().position(|c| c.name.starts_with("Shop.Api")).unwrap();
    let outs = app.runner.run_selected();
    crate::ui::run_windows::handle_outputs(&mut app, outs);
    let mut h = harness(app, vec2(1440.0, 900.0));
    run_frames(&mut h, "runtime counters", |a| a.monitor.runtime().is_some());
    h.state_mut().dock.show(crate::toolwindow::ToolWindow::Monitoring);
    h.state_mut().dock.sizes.insert(crate::toolwindow::ToolWindow::Monitoring, 420.0);
    h.run_steps(3);
    // CPU profile of a few seconds while requests come in.
    h.get_by_label("Start CPU Profiling").click();
    h.run_steps(2);
    // Load: requests from a few threads for three seconds.
    let load: Vec<_> = (0..4)
        .map(|_| {
            std::thread::spawn(|| {
                let end = std::time::Instant::now() + std::time::Duration::from_secs(3);
                while std::time::Instant::now() < end {
                    let _ = std::process::Command::new("curl").args(["-s", "-o", "/dev/null", "http://localhost:5112/weatherforecast"]).status();
                }
            })
        })
        .collect();
    while !load.iter().all(|t| t.is_finished()) {
        h.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    if h.state().monitor.profiling() {
        h.get_by_label("Stop Profiling").click();
    }
    run_frames(&mut h, "the profile", |a| a.monitor.profile.is_some());
    let prof = h.state().monitor.profile.clone().unwrap().expect("profile");
    assert!(prof.samples > 0);
    h.state_mut().monitor.show_tree = true;
    h.run_steps(3);
    h.render().unwrap().save(out.join("profiler-cpu-dark.png")).unwrap();
    h.state_mut().monitor.show_tree = false;
    h.run_steps(3);
    h.render().unwrap().save(out.join("profiler-hotspots-dark.png")).unwrap();
    // Memory snapshot.
    h.get_by_label("Get Memory Snapshot").click();
    run_frames(&mut h, "the snapshot", |a| a.monitor.snapshot.is_some());
    let snap = h.state().monitor.snapshot.clone().unwrap().expect("snapshot");
    assert!(snap.total_objects > 1000 && snap.types.iter().any(|t| t.name == "System.String"), "{} objects", snap.total_objects);
    // The web app itself, not MSBuild or dotnet watch.
    assert!(snap.types.iter().any(|t| t.name.starts_with("Microsoft.AspNetCore")), "{:?}", snap.types.iter().take(20).map(|t| &t.name).collect::<Vec<_>>());
    assert!(!snap.types.iter().any(|t| t.name.starts_with("Microsoft.Build.")), "profiled MSBuild");
    h.render().unwrap().save(out.join("profiler-memory-dark.png")).unwrap();
    h.state_mut().runner.kill_all_now();
}

#[test]
#[ignore = "Requires a GPU; captures the windows contributed in PRs 108/109 to target/snapshots"]
fn screens_contributed_windows() {
    let _home = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/snapshots");
    std::fs::create_dir_all(&out).unwrap();
    let home = tempfile::tempdir().unwrap();
    let root = home.path().canonicalize().unwrap().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    let git = |args: &[&str]| assert!(std::process::Command::new("git").args(args).current_dir(&root).output().unwrap().status.success(), "{args:?}");
    git(&["init", "-q", "-b", "main"]);
    git(&["config", "user.email", "t@t"]);
    git(&["config", "user.name", "T"]);
    std::fs::write(root.join("index.html"), "<!doctype html>\n<html lang=\"en\">\n<head>\n  <link rel=\"stylesheet\" href=\"site.css\">\n  <title>Shop</title>\n</head>\n<body>\n  <h1 class=\"title\">Orders</h1>\n  <script src=\"app.js\"></script>\n</body>\n</html>\n").unwrap();
    std::fs::write(root.join("App.tsx"), "import { useState } from \"react\";\n\ninterface Order { id: number; total: number }\n\nexport function Orders({ orders }: { orders: Order[] }) {\n  const [selected, setSelected] = useState<number | null>(null);\n  // Highlight the selected order.\n  return (\n    <ul className=\"orders\">\n      {orders.map(o => <li key={o.id} onClick={() => setSelected(o.id)}>{o.total.toFixed(2)}</li>)}\n    </ul>\n  );\n}\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "Initial"]);
    std::fs::write(root.join("index.html"), "<!doctype html>\n<p>wip</p>\n").unwrap();
    git(&["stash", "push", "-q", "-m", "WIP on the order page"]);
    // SQLite sample.
    let db = root.join("shop.db");
    {
        let c = rusqlite::Connection::open(&db).unwrap();
        c.execute_batch("CREATE TABLE customers(id INTEGER PRIMARY KEY, name TEXT NOT NULL, email TEXT UNIQUE);
            CREATE TABLE orders(id INTEGER PRIMARY KEY, customer_id INTEGER NOT NULL REFERENCES customers(id), total REAL, created TEXT);
            CREATE INDEX orders_customer ON orders(customer_id);
            INSERT INTO customers(name, email) VALUES ('Ada', 'ada@example.com'), ('Linus', 'linus@example.com');
            INSERT INTO orders(customer_id, total, created) VALUES (1, 64.85, '2026-11-01'), (2, 12.5, '2026-11-02'), (1, 99.0, '2026-11-03');").unwrap();
    }
    let mut app = test_app(home.path(), true);
    app.settings.roslyn_path = Some("/nonexistent".into());
    app.open_workspace(&root);
    app.open_file(&root.join("index.html"));
    app.open_file(&root.join("App.tsx"));
    let mut h = harness(app, vec2(1440.0, 900.0));
    h.run_steps(6);
    h.render().unwrap().save(out.join("contrib-tsx-dark.png")).unwrap();
    let i = h.state().docs.index_of(&root.join("index.html")).unwrap();
    h.state_mut().docs.activate(i);
    h.run_steps(6);
    h.render().unwrap().save(out.join("contrib-html-dark.png")).unwrap();
    // Git: stashes and repository tools.
    h.state_mut().dock.show(crate::toolwindow::ToolWindow::Git);
    h.state_mut().vcs.show_stashes = true;
    run_frames(&mut h, "stashes", |a| !a.vcs.stashes.is_empty());
    h.render().unwrap().save(out.join("contrib-stashes-dark.png")).unwrap();
    h.state_mut().vcs.tools.visible = true;
    h.run_steps(4);
    h.render().unwrap().save(out.join("contrib-repo-tools-dark.png")).unwrap();
    // SQLite.
    h.state_mut().dock.show(crate::toolwindow::ToolWindow::Database);
    h.state_mut().dock.sizes.insert(crate::toolwindow::ToolWindow::Database, 480.0);
    h.state_mut().database.path = db.display().to_string();
    let r = root.clone();
    h.state_mut().database.connect(Some(&r));
    run_frames(&mut h, "the schema", |a| a.database.connected.is_some());
    h.state_mut().database.sql = "SELECT c.name, COUNT(*) AS orders, SUM(o.total) AS total FROM orders o JOIN customers c ON c.id = o.customer_id GROUP BY c.name;".into();
    let sql = h.state().database.sql.clone();
    h.state_mut().database.execute(crate::database::Action::Query(sql));
    run_frames(&mut h, "the query", |a| a.database.grid.is_some() || a.database.error.is_some());
    assert!(h.state().database.error.is_none(), "{:?}", h.state().database.error);
    assert_eq!(h.state().database.grid.as_ref().unwrap().rows.len(), 2);
    h.render().unwrap().save(out.join("contrib-sqlite-dark.png")).unwrap();
    h.state_mut().database.execute(crate::database::Action::Relations);
    run_frames(&mut h, "relations", |a| a.database.relations.is_some() || a.database.error.is_some());
    h.state_mut().database.view = crate::database::View::Relations;
    h.run_steps(4);
    h.render().unwrap().save(out.join("contrib-sqlite-relations-dark.png")).unwrap();
    // Attach to Process.
    h.state_mut().dialog = Some(crate::ui::dialogs::Dialog::AttachProcess { pid: String::new(), focus: true });
    h.run_steps(4);
    h.render().unwrap().save(out.join("contrib-attach-dark.png")).unwrap();
    // The in-window main menu (Windows / Linux).
    h.state_mut().dialog = None;
    h.run_steps(2);
}

#[test]
#[ignore = "Requires a GPU; writes target/snapshots/nuget-sign-in-*.png"]
fn screens_nuget_private_feeds() {
    use rc_dotnet::nuget::PackageSource;
    use rc_dotnet::nuget_auth::SourceStatus;
    let _home = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/snapshots");
    std::fs::create_dir_all(&out).unwrap();
    let azure = "https://pkgs.dev.azure.com/contoso/_packaging/internal/nuget/v3/index.json";
    let source = |name: &str, url: &str| PackageSource { name: name.into(), url: url.into(), enabled: true, credentials: None };
    for dark in [true, false] {
        let suffix = if dark { "dark" } else { "light" };
        let home = tempfile::tempdir().unwrap();
        let mut app = test_app(home.path(), dark);
        app.open_workspace(&fixtures().join("Shop/Shop.sln"));
        app.dock.show(crate::toolwindow::ToolWindow::Nuget);
        app.dock.sizes.insert(crate::toolwindow::ToolWindow::Nuget, 420.0);
        let mut h = harness(app, vec2(1280.0, 800.0));
        h.run_steps(3);
        h.state_mut().nuget.set_sources_for_test(
            vec![
                (source("nuget.org", rc_dotnet::nuget::NUGET_ORG), SourceStatus::Ok),
                (source("Contoso", azure), SourceStatus::NeedsSignIn("the source needs credentials (401)".into())),
                (source("GitHub", "https://nuget.pkg.github.com/contoso/index.json"), SourceStatus::Failed("nuget.config: the password is encrypted with Windows DPAPI".into())),
            ],
            1,
        );
        h.run_steps(3);
        h.render().unwrap().save(out.join(format!("nuget-sign-in-needed-{suffix}.png"))).unwrap();
        // The source picker with each source's status.
        h.get_by_label("Contoso").click();
        h.run_steps(3);
        h.render().unwrap().save(out.join(format!("nuget-sources-{suffix}.png"))).unwrap();
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        // Device flow: the provider's page and code.
        let device = "[CredentialProvider]To sign in, use a web browser to open the page https://microsoft.com/devicelogin and enter the code HXQ7K2LMB to authenticate.";
        h.state_mut().nuget.sign_in_for_test("Contoso", azure, &[device], None);
        h.run_steps(3);
        h.render().unwrap().save(out.join(format!("nuget-sign-in-{suffix}.png"))).unwrap();
        h.state_mut().nuget.sign_in_for_test("Contoso", azure, &[], Some("CredentialProvider.Microsoft: the device code expired"));
        h.run_steps(3);
        h.render().unwrap().save(out.join(format!("nuget-sign-in-error-{suffix}.png"))).unwrap();
    }
}


#[test]
#[ignore = "Requires a GPU, the .NET SDK and nuget.org; writes NuGet CPM captures to target/snapshots"]
fn screens_nuget_cpm() {
    let _home = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/snapshots");
    std::fs::create_dir_all(&out).unwrap();
    for dark in [true, false] {
        let suffix = if dark { "dark" } else { "light" };
        let home = tempfile::tempdir().unwrap();
        let mut app = test_app(home.path(), dark);
        app.settings.roslyn_path = Some("/nonexistent".into());
        app.open_workspace(&fixtures().join("CentralShop/CentralShop.slnx"));
        app.dock.show(crate::toolwindow::ToolWindow::Nuget);
        app.dock.sizes.insert(crate::toolwindow::ToolWindow::Nuget, 440.0);
        let mut h = harness(app, vec2(1280.0, 800.0));
        run_frames(&mut h, "CPM evaluation", |a| a.nuget.cpm_ready_for_test());
        // A central package with an override in one project.
        h.state_mut().nuget.select_for_test("Newtonsoft.Json");
        run_frames(&mut h, "versions", |a| a.nuget.ready_for_test());
        h.render().unwrap().save(out.join(format!("nuget-cpm-{suffix}.png"))).unwrap();
        // The issues tab: Dapper's version is not used.
        h.state_mut().nuget.show_central_for_test();
        h.state_mut().nuget.select_for_test("Dapper");
        h.run_steps(4);
        h.render().unwrap().save(out.join(format!("nuget-cpm-issues-{suffix}.png"))).unwrap();
    }
}

#[test]
#[ignore = "Requires a GPU; writes branch management captures to target/snapshots"]
fn screens_branches() {
    let _home = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/snapshots");
    std::fs::create_dir_all(&out).unwrap();
    for dark in [true, false] {
        let suffix = if dark { "dark" } else { "light" };
        let home = tempfile::tempdir().unwrap();
        let base = home.path().canonicalize().unwrap();
        let (remote, root) = (base.join("remote.git"), base.join("Shop"));
        std::fs::create_dir(&remote).unwrap();
        let copy = std::process::Command::new("cp").args(["-R", fixtures().join("Shop").to_str().unwrap(), root.to_str().unwrap()]).status().unwrap();
        assert!(copy.success());
        let git = |dir: &Path, args: &[&str]| assert!(std::process::Command::new("git").args(args).current_dir(dir).output().unwrap().status.success(), "{args:?}");
        git(&remote, &["init", "-q", "--bare", "-b", "main"]);
        git(&root, &["init", "-q", "-b", "main"]);
        git(&root, &["config", "user.name", "Ada Lovelace"]);
        git(&root, &["config", "user.email", "ada@example.com"]);
        git(&root, &["remote", "add", "origin", &format!("file://{}", remote.display())]);
        git(&root, &["add", "."]);
        git(&root, &["commit", "-q", "-m", "Initial shop"]);
        git(&root, &["push", "-q", "-u", "origin", "main"]);
        for (branch, file, subject) in [("feature/coupons", "Coupon.cs", "Add coupons"), ("feature/gift-cards", "GiftCard.cs", "Add gift cards"), ("bugfix/rounding", "Rounding.cs", "Fix rounding of discounts")] {
            git(&root, &["checkout", "-q", "-b", branch, "main"]);
            std::fs::write(root.join("src/Shop.Core").join(file), "namespace Shop.Core;\n").unwrap();
            git(&root, &["add", "."]);
            git(&root, &["commit", "-q", "-m", subject]);
        }
        git(&root, &["push", "-q", "-u", "origin", "feature/coupons"]);
        git(&root, &["checkout", "-q", "main"]);
        git(&root, &["push", "-q", "origin", "main:release/1.0"]);
        git(&root, &["fetch", "-q", "origin"]);
        std::fs::write(root.join("README.md"), "# Shop\n").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-q", "-m", "Add a readme"]);

        let mut app = test_app(&base.join("home"), dark);
        app.settings.roslyn_path = Some("/nonexistent".into());
        app.open_workspace(&root.join("Shop.sln"));
        let mut h = harness(app, vec2(1440.0, 900.0));
        run_frames(&mut h, "branches", |a| a.vcs.branches.iter().any(|b| b.name == "origin/release/1.0"));
        let label = rc_kit::i18n::tr("Git branches and actions");
        h.get_by_role_and_label(egui::accesskit::Role::Button, label).click();
        h.run_steps(4);
        h.render().unwrap().save(out.join(format!("branches-popup-{suffix}.png"))).unwrap();
        h.get_by_role_and_label(egui::accesskit::Role::Button, "feature/coupons").click();
        h.run_steps(4);
        h.render().unwrap().save(out.join(format!("branches-actions-{suffix}.png"))).unwrap();
        if !dark {
            continue;
        }
        h.get_by_role_and_label(egui::accesskit::Role::Button, label).click();
        h.run_steps(2);
        h.state_mut().dialog = Some(crate::ui::dialogs::Dialog::DeleteBranch { name: "feature/gift-cards".into(), force: true });
        h.run_steps(4);
        h.render().unwrap().save(out.join("branches-force-delete-dark.png")).unwrap();
        h.state_mut().dialog = Some(crate::ui::dialogs::Dialog::DeleteRemoteBranch { name: "origin/release/1.0".into() });
        h.run_steps(4);
        h.render().unwrap().save(out.join("branches-delete-remote-dark.png")).unwrap();
        h.state_mut().vcs.compare_with_current("feature/coupons");
        h.state_mut().dialog = Some(crate::ui::dialogs::Dialog::CompareBranch { branch: "feature/coupons".into() });
        run_frames(&mut h, "compare", |a| a.vcs.compare.as_ref().is_some_and(|c| !c.loading));
        h.render().unwrap().save(out.join("branches-compare-dark.png")).unwrap();
        h.state_mut().dialog = Some(crate::ui::dialogs::Dialog::NewBranch { name: "feature/vouchers".into(), start: Some("feature/coupons".into()), checkout: true });
        h.run_steps(4);
        h.render().unwrap().save(out.join("branches-new-dark.png")).unwrap();
    }
}


/// Copies a folder without `bin/` and `obj/`.
fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        if e.file_name() == "bin" || e.file_name() == "obj" {
            continue;
        }
        if e.file_type().unwrap().is_dir() {
            copy_tree(&e.path(), &to.join(e.file_name()));
        } else {
            std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
        }
    }
}

#[test]
#[ignore = "Requires a GPU and the .NET SDK; writes NuGet CPM migration and audit captures to target/snapshots"]
fn screens_nuget_migrate_and_audit() {
    let _home = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/snapshots");
    std::fs::create_dir_all(&out).unwrap();
    // Shop with a version conflict and a conditional reference.
    let tmp = tempfile::Builder::new().prefix("dotcraft-screens-cpm").tempdir().unwrap();
    let shop = tmp.path().canonicalize().unwrap().join("Shop");
    copy_tree(&fixtures().join("Shop"), &shop);
    let nunit = shop.join("tests/Shop.Core.NunitTests/Shop.Core.NunitTests.csproj");
    let text = std::fs::read_to_string(&nunit).unwrap().replace(r#""coverlet.collector" Version="6.0.0""#, r#""coverlet.collector" Version="6.0.4""#);
    std::fs::write(&nunit, text).unwrap();
    let api = shop.join("src/Shop.Api/Shop.Api.csproj");
    let text = std::fs::read_to_string(&api).unwrap().replace(
        "  <ItemGroup>\n    <ProjectReference",
        "  <ItemGroup Condition=\"'$(Configuration)' == 'Debug'\">\n    <PackageReference Include=\"Swashbuckle.AspNetCore\" Version=\"6.5.0\" />\n  </ItemGroup>\n\n  <ItemGroup>\n    <ProjectReference",
    );
    std::fs::write(&api, text).unwrap();
    let central = fixtures().join("CentralShop");
    let path = |rel: &str| central.join(rel).display().to_string();
    let json = include_str!("../../rc-dotnet/tests/data/list-package-vulnerable.json").replace("/work/Audit/A/A.csproj", &path("src/CentralShop.Core/CentralShop.Core.csproj")).replace("/work/Audit/B/B.csproj", &path("src/CentralShop.Cli/CentralShop.Cli.csproj"));
    let deprecated = include_str!("../../rc-dotnet/tests/data/list-package-deprecated.json").replace("/work/Audit/A/A.csproj", &path("src/CentralShop.Core/CentralShop.Core.csproj")).replace("/work/Audit/B/B.csproj", &path("src/CentralShop.Cli/CentralShop.Cli.csproj"));
    let mut report = rc_dotnet::nuget::parse_list_packages(&json).unwrap();
    report.merge(rc_dotnet::nuget::parse_list_packages(&deprecated).unwrap());
    for dark in [true, false] {
        let suffix = if dark { "dark" } else { "light" };
        // The migration preview.
        let home = tempfile::tempdir().unwrap();
        let mut app = test_app(home.path(), dark);
        app.settings.roslyn_path = Some("/nonexistent".into());
        app.open_workspace(&shop.join("Shop.sln"));
        app.dock.show(crate::toolwindow::ToolWindow::Nuget);
        app.dock.sizes.insert(crate::toolwindow::ToolWindow::Nuget, 500.0);
        let mut h = harness(app, vec2(1280.0, 800.0));
        run_frames(&mut h, "NuGet", |a| a.nuget.cpm_ready_for_test());
        let app = h.state_mut();
        app.nuget.start_migration_for_test(&app.docs, "Shop.Core.XunitTests.csproj");
        h.run_steps(4);
        h.render().unwrap().save(out.join(format!("nuget-migrate-{suffix}.png"))).unwrap();
        // The audit, fed with sample results.
        let home = tempfile::tempdir().unwrap();
        let mut app = test_app(home.path(), dark);
        app.settings.roslyn_path = Some("/nonexistent".into());
        app.open_workspace(&central.join("CentralShop.slnx"));
        app.dock.show(crate::toolwindow::ToolWindow::Nuget);
        app.dock.sizes.insert(crate::toolwindow::ToolWindow::Nuget, 500.0);
        let mut h = harness(app, vec2(1280.0, 800.0));
        run_frames(&mut h, "CPM evaluation", |a| a.nuget.cpm_ready_for_test());
        h.state_mut().nuget.set_audit_for_test(report.clone(), &[("Newtonsoft.Json", "13.0.3"), ("System.Net.Http", "4.3.4"), ("System.Text.RegularExpressions", "4.3.1")]);
        h.run_steps(4);
        h.render().unwrap().save(out.join(format!("nuget-audit-{suffix}.png"))).unwrap();
        h.state_mut().nuget.select_audit_for_test("System.Text.RegularExpressions", true);
        h.run_steps(4);
        h.render().unwrap().save(out.join(format!("nuget-audit-pin-{suffix}.png"))).unwrap();
        h.state_mut().nuget.select_audit_for_test("WindowsAzure.Storage", false);
        h.run_steps(4);
        h.render().unwrap().save(out.join(format!("nuget-audit-deprecated-{suffix}.png"))).unwrap();
    }
}


#[test]
#[ignore = "Requires a GPU; writes package insight captures (local feed) to target/snapshots"]
fn screens_package_insight() {
    use crate::package_insight_tests::{app_with, caret_at, central_shop, fake_feed, settle, text, type_text};
    let _home = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/snapshots");
    std::fs::create_dir_all(&out).unwrap();
    let feed = fake_feed();
    for dark in [true, false] {
        let suffix = if dark { "dark" } else { "light" };
        let home = tempfile::tempdir().unwrap();
        let (_tmp, root) = central_shop(&feed);
        let core = root.join("src/CentralShop.Core/CentralShop.Core.csproj");
        let mut h = app_with(&root, home.path(), dark, &core);
        // Update markers: gutter icon and inline hint.
        let line = text(h.state()).lines().position(|l| l.contains("Humanizer.Core")).unwrap();
        settle(&mut h, "markers", |a| a.docs.active_doc().unwrap().editor.gutter_icons.contains_key(&line));
        h.render().unwrap().save(out.join(format!("package-update-{suffix}.png"))).unwrap();
        // Hover on the package id.
        caret_at(&mut h, "Humanizer", false);
        h.run_steps(2);
        let screen = h.state().caret_screen.unwrap() - egui::vec2(0.0, 10.0);
        let pos = h.state().docs.active_doc().unwrap().editor.caret() + 3;
        {
            let app = h.state_mut();
            let doc = &mut app.docs.list[app.docs.active.unwrap()];
            app.packages.on_editor_events(doc, &[rc_editor::EditorEvent::HoverAt { pos, screen }]);
        }
        settle(&mut h, "hover", |a| a.docs.active_doc().unwrap().editor.popups.hover.is_some());
        h.render().unwrap().save(out.join(format!("package-hover-{suffix}.png"))).unwrap();
        // Completion of a new reference's id.
        h.state_mut().docs.active_doc_mut().unwrap().editor.popups.hover = None;
        caret_at(&mut h, "<PackageReference Include=\"Newtonsoft.Json\" />", true);
        type_text(&mut h, "\n<PackageReference Include=");
        type_text(&mut h, "Seri");
        settle(&mut h, "completion", |a| a.docs.active_doc().unwrap().editor.popups.completion.as_ref().is_some_and(|c| !c.loading && !c.items.is_empty()));
        h.run_steps(4);
        h.render().unwrap().save(out.join(format!("package-completion-{suffix}.png"))).unwrap();    }
}

/// A solution with an EF Core data project and a web API that references it (never built:
/// the dialog's lists are set by the test).
#[cfg(test)]
pub fn ef_solution() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::Builder::new().prefix("dotcraft-ef").tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("Shop.Data")).unwrap();
    std::fs::create_dir_all(root.join("Shop.Api")).unwrap();
    std::fs::write(
        root.join("Shop.Data/Shop.Data.csproj"),
        "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup><TargetFramework>net10.0</TargetFramework></PropertyGroup>\n  <ItemGroup>\n    <PackageReference Include=\"Microsoft.EntityFrameworkCore.SqlServer\" Version=\"10.0.0\" />\n  </ItemGroup>\n</Project>\n",
    )
    .unwrap();
    std::fs::write(root.join("Shop.Data/ShopContext.cs"), "using Microsoft.EntityFrameworkCore;\n\nnamespace Shop.Data;\n\npublic class ShopContext(DbContextOptions<ShopContext> options) : DbContext(options)\n{\n    public DbSet<Order> Orders => Set<Order>();\n}\n\npublic class Order\n{\n    public int Id { get; set; }\n    public string Customer { get; set; } = \"\";\n}\n").unwrap();
    std::fs::write(
        root.join("Shop.Api/Shop.Api.csproj"),
        "<Project Sdk=\"Microsoft.NET.Sdk.Web\">\n  <PropertyGroup><TargetFramework>net10.0</TargetFramework></PropertyGroup>\n  <ItemGroup>\n    <PackageReference Include=\"Microsoft.EntityFrameworkCore.Design\" Version=\"10.0.0\" />\n    <ProjectReference Include=\"../Shop.Data/Shop.Data.csproj\" />\n  </ItemGroup>\n</Project>\n",
    )
    .unwrap();
    std::fs::write(root.join("Shop.Api/Program.cs"), "var app = WebApplication.CreateBuilder(args).Build();\napp.Run();\n").unwrap();
    std::fs::write(root.join("Shop.slnx"), "<Solution>\n  <Project Path=\"Shop.Api/Shop.Api.csproj\" />\n  <Project Path=\"Shop.Data/Shop.Data.csproj\" />\n</Solution>\n").unwrap();
    (tmp, root)
}

#[cfg(test)]
pub fn ef_samples() -> (Vec<rc_dotnet::ef::DbContextInfo>, Vec<rc_dotnet::ef::Migration>) {
    let contexts = rc_dotnet::ef::parse_contexts(r#"[{"fullName":"Shop.Data.ShopContext","safeName":"ShopContext","name":"ShopContext"},{"fullName":"Shop.Data.AuditContext","safeName":"AuditContext","name":"AuditContext"}]"#).unwrap();
    let migrations = rc_dotnet::ef::parse_migrations(
        r#"[{"id":"20260903101500_Initial","name":"Initial","safeName":"Initial","applied":true},{"id":"20261002094512_AddCustomers","name":"AddCustomers","safeName":"AddCustomers","applied":true},{"id":"20261108170230_AddOrderLines","name":"AddOrderLines","safeName":"AddOrderLines","applied":false}]"#,
    )
    .unwrap();
    (contexts, migrations)
}

#[test]
#[ignore = "Requires a GPU; writes Entity Framework Core captures to target/snapshots"]
fn screens_ef_core() {
    use crate::ui::ef_core::{EfDialog, EfOp};
    let _home = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/snapshots");
    std::fs::create_dir_all(&out).unwrap();
    let (_tmp, root) = ef_solution();
    let (contexts, migrations) = ef_samples();
    for dark in [true, false] {
        let suffix = if dark { "dark" } else { "light" };
        let home = tempfile::tempdir().unwrap();
        let mut app = test_app(home.path(), dark);
        app.settings.roslyn_path = Some("/nonexistent".into());
        app.open_workspace(&root.join("Shop.slnx"));
        let mut h = harness(app, vec2(1280.0, 800.0));
        h.run_steps(4);
        // Explorer › right-click on the data project › Entity Framework Core.
        let at = h.get_by_label("Shop.Data").rect().center();
        h.event(egui::Event::PointerMoved(at));
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed: true, modifiers: Default::default() });
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed: false, modifiers: Default::default() });
        h.run_steps(3);
        // The submenu button sits right under Manage NuGet Packages (egui's submenu buttons
        // expose no label to AccessKit).
        let ef = h.get_by_label(rc_kit::i18n::tr("Manage NuGet Packages…")).rect().center() + vec2(0.0, 28.0);
        h.event(egui::Event::PointerMoved(ef));
        h.run_steps(6);
        h.render().unwrap().save(out.join(format!("ef-menu-{suffix}.png"))).unwrap();
        let add = h.get_by_label(&format!("{}…", rc_kit::i18n::tr("Add Migration"))).rect().center();
        h.event(egui::Event::PointerMoved(add));
        h.event(egui::Event::PointerButton { pos: add, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.event(egui::Event::PointerButton { pos: add, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
        h.run_steps(2);
        assert_eq!(h.state().ef.as_ref().map(|d| d.op), Some(EfOp::AddMigration));
        // The dialog on each operation, with sample DbContexts and migrations.
        h.state_mut().ef.as_mut().unwrap().set_offline(Some("10.0.8"), contexts.clone(), migrations.clone());
        h.run_steps(2);
        h.get_by_label(rc_kit::i18n::tr("Migration name")).type_text("AddInvoices");
        h.run_steps(4);
        h.render().unwrap().save(out.join(format!("ef-add-migration-{suffix}.png"))).unwrap();
        for (op, name) in [(EfOp::RemoveMigration, "remove-migration"), (EfOp::UpdateDatabase, "update-database"), (EfOp::Script, "script"), (EfOp::Scaffold, "scaffold")] {
            h.get_by_label(rc_kit::i18n::tr(op.title())).click();
            h.run_steps(4);
            assert_eq!(h.state().ef.as_ref().map(|d| d.op), Some(op));
            h.render().unwrap().save(out.join(format!("ef-{name}-{suffix}.png"))).unwrap();
        }
        // Without the dotnet-ef tool: the install offer.
        let mut d = EfDialog::new(h.state(), EfOp::UpdateDatabase, None, Waker::noop());
        d.set_offline(None, Vec::new(), Vec::new());
        h.state_mut().ef = Some(d);
        h.run_steps(4);
        h.render().unwrap().save(out.join(format!("ef-no-tool-{suffix}.png"))).unwrap();
    }
}
