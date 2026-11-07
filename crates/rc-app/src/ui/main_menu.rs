//! Scrollable native-egui menus, selectively adapted from CrabCraft's main_menu.rs.
//! Only implemented DotCraft actions are offered; command dispatch matches shortcuts.
//! Copyright (c) 2026 javy-fernandez and the DotCraft contributors.
//! Copyright (c) 2026 the CrabCraft contributors.
//! The retained MIT notice is in docs/implementation/scrollable-menu.md.

use crate::app::DotCraftApp;
use crate::toolwindow::ToolWindow;
use crate::ui::dialogs::{Dialog, SettingsPage};
use egui::Ui;
use rc_editor::Action;
use rc_kit::{ico, theme, i18n::tr};

fn groups() -> Vec<(&'static str, Vec<(&'static str, Action)>)> {
    use Action::*;
    vec![
        (tr("File"), vec![(tr("Save"), Save), (tr("Save All"), SaveAll), (tr("Close Tab"), CloseTab), (tr("Settings…"), ShowSettings)]),
        (tr("Edit"), vec![(tr("Undo"), Undo), (tr("Redo"), Redo), (tr("Cut"), Cut), (tr("Copy"), Copy), (tr("Paste"), Paste), (tr("Select All"), SelectAll), (tr("Find…"), Find), (tr("Replace…"), Replace), (tr("Duplicate Line or Selection"), DuplicateLine), (tr("Comment with Line Comment"), CommentLine), (tr("Comment with Block Comment"), CommentBlock), (tr("Join Lines"), JoinLines), (tr("Toggle Case"), ToggleCase), (tr("Move Line Up"), MoveLineUp), (tr("Move Line Down"), MoveLineDown)]),
        (tr("View"), vec![(tr("Zoom In"), ZoomIn), (tr("Zoom Out"), ZoomOut), (tr("Reset Zoom"), ZoomReset), (tr("Hide All Tool Windows"), HideToolWindows)]),
        (tr("Navigate"), vec![(tr("Search Everywhere"), SearchEverywhere), (tr("Go to File…"), GotoFile), (tr("Go to Type…"), GotoClass), (tr("Go to Symbol…"), GotoSymbol), (tr("Go to Line…"), GotoLine), (tr("Back"), NavigateBack), (tr("Forward"), NavigateForward), (tr("Recent Files"), RecentFiles), (tr("Recent Locations"), RecentLocations), (tr("Go to Declaration"), GotoDeclaration), (tr("Go to Implementation"), GotoImplementation), (tr("Go to Type Declaration"), GotoTypeDeclaration), (tr("Find Usages"), FindUsages), (tr("File Structure"), FileStructure)]),
        (tr("Code"), vec![(tr("Code Completion"), Complete), (tr("Parameter Info"), ParameterInfo), (tr("Quick Documentation"), QuickDoc), (tr("Show Context Actions"), ShowIntentions), (tr("Rename…"), Rename), (tr("Reformat Code"), Reformat)]),
        (tr("Build"), vec![(tr("Build Solution"), Build)]),
        (tr("Run"), vec![(tr("Run"), Run), (tr("Debug"), Debug), (tr("Stop"), StopRun), (tr("Run Tests"), RunTests), (tr("Step Over"), StepOver), (tr("Step Into"), StepInto), (tr("Step Out"), StepOut), (tr("Run to Cursor"), RunToCursor), (tr("Resume Program"), Resume), (tr("Evaluate Expression…"), Evaluate), (tr("Attach to Process…"), AttachProcess)]),
        (tr("Tools"), vec![(tr("Settings…"), ShowSettings)]),
        (tr("Git"), vec![(tr("Commit…"), Commit)]),
        (tr("Window"), vec![(tr("Next Tab"), NextTab), (tr("Previous Tab"), PreviousTab)]),
        (tr("Help"), vec![]),
    ]
}

fn enabled(app: &DotCraftApp, action: Action) -> bool {
    let document = app.docs.active_doc().is_some();
    match action {
        Action::ShowSettings | Action::ZoomIn | Action::ZoomOut | Action::ZoomReset => true,
        Action::AttachProcess => app.workspace.is_some() && !app.debugger.active() && app.debugger.downloading.is_none(),
        Action::Run => app.runner.selected_config().is_some(),
        Action::Debug => app.runner.selected_config().is_some_and(|config| matches!(config.kind, crate::runner::ConfigKind::App { .. })),
        Action::StopRun => app.runner.busy() || app.debugger.active() || app.debugger.downloading.is_some(),
        Action::StepOver | Action::StepInto | Action::StepOut | Action::Resume | Action::Evaluate => app.debugger.stopped(),
        Action::RunToCursor => app.debugger.stopped() && document,
        Action::Build | Action::RunTests | Action::HideToolWindows | Action::SearchEverywhere | Action::GotoFile | Action::GotoClass | Action::GotoSymbol => app.workspace.is_some(),
        Action::Commit => app.vcs.repo.is_some(),
        Action::SaveAll => !app.docs.list.is_empty(),
        Action::Cut | Action::Paste => app.docs.active_doc().is_some_and(|doc| !doc.editor.read_only),
        _ => document,
    }
}

/// Anchor-independent bounds avoid popup placement feeding a shrinking height back into
/// the next frame. Each submenu owns its own scroll offset and viewport-height limit.
fn scroll_menu(ui: &mut Ui, width: f32, id: impl std::hash::Hash + std::fmt::Debug, contents: impl FnOnce(&mut Ui)) {
    let viewport = ui.ctx().content_rect();
    ui.set_width(width.min((viewport.width() - 32.0).max(80.0)));
    let height = (viewport.height() - 64.0).max(40.0);
    ui.set_max_height(height);
    egui::ScrollArea::vertical().id_salt(id).max_height(height).show(ui, contents);
}

fn command(ui: &mut Ui, label: &str, icon: Option<rc_kit::Ico>, enabled: bool) -> bool {
    let clicked = theme::menu_entry(ui, icon, label, None, false, enabled).clicked();
    if clicked { ui.close(); }
    clicked
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
enum Page {
    #[default]
    Categories,
    Commands(usize),
    ToolWindows,
}

pub fn reset(ctx: &egui::Context) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new("main_menu_page"), Page::Categories));
}

pub fn show(app: &mut DotCraftApp, ui: &mut Ui) {
    let id = egui::Id::new("main_menu_page");
    let mut page = ui.ctx().data_mut(|data| data.get_temp::<Page>(id)).unwrap_or_default();
    let width = if page == Page::Categories { 180.0 } else { 320.0 };
    scroll_menu(ui, width, ("main_menu_page", page), |ui| {
        if page != Page::Categories {
            if theme::menu_entry(ui, Some(ico::CHEVRON_LEFT), tr("Main Menu"), None, false, true).clicked() {
                page = Page::Categories;
            }
            ui.separator();
        }
        match page {
            Page::Categories => {
                for (index, (category, _)) in groups().iter().enumerate() {
                    if theme::menu_entry(ui, None, category, Some("›"), false, true).clicked() {
                        page = Page::Commands(index);
                    }
                }
            }
            Page::ToolWindows => {
                for tool in ToolWindow::ALL {
                    if command(ui, tool.title(), Some(tool.icon()), app.workspace.is_some()) { app.toggle_tool(tool); }
                }
            }
            Page::Commands(index) => {
                if let Some((category, actions)) = groups().into_iter().nth(index) {
                    category_commands(app, ui, category, actions, &mut page);
                }
            }
        }
    });
    ui.ctx().data_mut(|data| data.insert_temp(id, page));
}

fn category_commands(app: &mut DotCraftApp, ui: &mut Ui, category: &str, actions: Vec<(&str, Action)>, page: &mut Page) {
    if category == tr("File") {
        if command(ui, tr("New Solution…"), Some(ico::ADD), true) { app.new_project_dialog(None); }
        if command(ui, tr("New File…"), Some(ico::NEW_FILE), app.workspace.is_some() || app.docs.active_doc().is_some()) { app.new_file_dialog(); }
        if command(ui, tr("Open…"), Some(ico::OPEN), true) { app.pick_and_open(); }
        if command(ui, tr("Close Project"), Some(ico::CLOSE), app.workspace.is_some()) { app.close_project(); }
        ui.separator();
    }
    for (label, action) in actions {
        let shortcut = app.keymap.shortcut_text(action);
        if theme::menu_entry(ui, crate::search::action_icon(action), label, shortcut.as_deref(), false, enabled(app, action)).clicked() {
            app.focus_editor = true;
            let clipboard = match action {
                Action::Copy => Some(crate::native_menu::MenuCommand::Copy),
                Action::Cut => Some(crate::native_menu::MenuCommand::Cut),
                Action::Paste => Some(crate::native_menu::MenuCommand::Paste),
                _ => None,
            };
            if let Some(command) = clipboard {
                crate::native_menu::forward_edit(ui.ctx(), command);
            } else {
                app.pending.push(action);
            }
            ui.close();
        }
    }
    if category == tr("View") {
        ui.separator();
        if theme::menu_entry(ui, None, tr("Tool Windows"), Some("›"), false, true).clicked() { *page = Page::ToolWindows; }
    }
    if category == tr("Build") {
        for (label, icon, action) in [(tr("Rebuild Solution"), ico::REBUILD, rc_dotnet::BuildAction::Rebuild), (tr("Clean Solution"), ico::DELETE, rc_dotnet::BuildAction::Clean), (tr("Restore NuGet Packages"), ico::DOWNLOAD, rc_dotnet::BuildAction::Restore)] {
            if command(ui, label, Some(icon), app.workspace.is_some())
                && let Some(target) = app.workspace.as_ref().map(|workspace| workspace.ws.key_path().to_path_buf())
            {
                let outputs = app.runner.build(&target, action);
                crate::ui::run_windows::handle_outputs(app, outputs);
            }
        }
    }
    if category == tr("Tools") {
        if command(ui, tr(".NET CLI"), Some(ico::SETTINGS), true) {
            app.dialog = Some(Dialog::Settings { draft: Box::new(app.settings.clone()), page: SettingsPage::Tools });
            app.focus_editor = false;
        }
        if command(ui, "NuGet", Some(ico::PACKAGE), app.workspace.is_some()) { app.dock.show(ToolWindow::Nuget); }
    }
    if category == tr("Help") && command(ui, tr("About DotCraft"), Some(ico::INFO), true) { app.dialog = Some(Dialog::About); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    fn harness(home: &std::path::Path, size: egui::Vec2) -> Harness<'static, DotCraftApp> {
        let app = crate::screens::test_app(home, true);
        harness_app(app, size)
    }

    fn harness_app(app: DotCraftApp, size: egui::Vec2) -> Harness<'static, DotCraftApp> {
        let mut first = true;
        let mut harness = Harness::builder().with_size(size).build_ui_state(
            move |ui, app: &mut DotCraftApp| {
                if std::mem::take(&mut first) {
                    rc_kit::theme::install_fonts(ui.ctx());
                    return;
                }
                let rect = egui::Rect::from_min_size(ui.max_rect().min, egui::vec2(ui.available_width(), 40.0));
                let mut toolbar = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                crate::ui::header::show(app, &mut toolbar);
            }, app,
        );
        harness.run_steps(3);
        harness.get_by_role_and_label(egui::accesskit::Role::Button, tr("Main Menu")).click();
        harness.run_steps(5);
        harness
    }

    fn scroll(harness: &mut Harness<'static, DotCraftApp>, point: egui::Pos2) {
        harness.input_mut().events.push(egui::Event::PointerMoved(point));
        harness.input_mut().events.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -1000.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        });
        harness.run_steps(12);
    }

    #[test]
    fn compact_main_menu_scrolls_to_help_and_opens_about() {
        let _home = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        for size in [egui::vec2(480.0, 240.0), egui::vec2(640.0, 320.0)] {
            let mut harness = harness(home.path(), size);
            let initial = harness.get_by_label(tr("File")).rect().center();
            scroll(&mut harness, initial);
            let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
            let help_label = tr("Help");
            let help = harness.get_by_label(help_label);
            assert!(viewport.contains_rect(help.rect()), "Help category must fit {size:?}: {:?}", help.rect());
            help.click();
            harness.run_steps(4);
            let about = harness.get_by_label(tr("About DotCraft"));
            assert!(viewport.contains_rect(about.rect()), "About action must fit {size:?}: {:?}", about.rect());
            about.click();
            harness.run_steps(2);
            assert!(matches!(harness.state().dialog, Some(Dialog::About)));
        }
    }

    #[test]
    fn compact_run_submenu_scrolls_to_attach_and_dispatches_its_action() {
        let _home = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        let size = egui::vec2(480.0, 240.0);
        let root = home.path().join("Demo");
        std::fs::create_dir(&root).unwrap();
        let mut app = crate::screens::test_app(home.path(), true);
        app.workspace = Some(crate::app::WorkspaceState { ws: rc_core::Workspace::open(&root).unwrap(), solution: None });
        let mut harness = harness_app(app, size);
        let initial = harness.get_by_label(tr("File")).rect().center();
        scroll(&mut harness, initial);
        harness.get_by_label(tr("Run")).click();
        harness.run_steps(4);
        let first = harness.get_by_label(tr("Debug")).rect().center();
        scroll(&mut harness, first);
        let attach = harness.get_by_label(tr("Attach to Process…"));
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        assert!(viewport.contains_rect(attach.rect()), "Attach must fit after submenu scrolling: {:?}", attach.rect());
        attach.click();
        harness.run_steps(2);
        assert_eq!(harness.state().pending, vec![Action::AttachProcess]);
        harness.state_mut().run_ide_action(&egui::Context::default(), Action::AttachProcess);
        assert!(matches!(harness.state().dialog, Some(Dialog::AttachProcess { .. })));
        assert!(harness.state().debugger.session.is_none());
    }

    #[test]
    fn attach_command_is_disabled_without_a_workspace() {
        use egui_kittest::kittest::NodeT;
        let _home = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        let mut harness = harness(home.path(), egui::vec2(640.0, 320.0));
        harness.get_by_label(tr("Run")).click();
        harness.run_steps(3);
        let initial = harness.get_by_label(tr("Debug")).rect().center();
        scroll(&mut harness, initial);
        let attach = harness.get_by_label(tr("Attach to Process…"));
        assert!(attach.accesskit_node().is_disabled());
        attach.click();
        harness.run_steps(2);
        assert!(harness.state().pending.is_empty());
        assert!(harness.state().dialog.is_none());
    }

    #[test]
    fn compact_tool_catalog_scrolls_to_and_opens_the_last_window() {
        let _home = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        let fixture = crate::screens::fixtures().join("TenProjects/TenProjects.slnx");
        let solution = rc_dotnet::Solution::load(&fixture).unwrap();
        assert_eq!(solution.projects().len(), 10);
        let mut app = crate::screens::test_app(home.path(), true);
        // Load the real .NET solution model without starting SDK or language-server work.
        app.workspace = Some(crate::app::WorkspaceState { ws: rc_core::Workspace::open(&fixture).unwrap(), solution: Some(solution) });
        let size = egui::vec2(480.0, 240.0);
        let mut harness = harness_app(app, size);
        harness.get_by_label(tr("View")).click();
        harness.run_steps(3);
        harness.get_by_label(tr("Tool Windows")).click();
        harness.run_steps(3);
        let initial = harness.get_by_label(ToolWindow::Explorer.title()).rect().center();
        scroll(&mut harness, initial);
        let last = *ToolWindow::ALL.last().unwrap();
        let target = harness.get_by_label(last.title());
        assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(target.rect()));
        target.click();
        harness.run_steps(2);
        assert!(harness.state().dock.is_open(last));
        assert_eq!(harness.state().workspace.as_ref().unwrap().solution.as_ref().unwrap().projects().len(), 10);
    }

    #[test]
    fn tools_entries_open_nuget_and_the_existing_cli_settings_page() {
        let _home = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join("Demo");
        std::fs::create_dir(&root).unwrap();
        let mut app = crate::screens::test_app(home.path(), true);
        app.workspace = Some(crate::app::WorkspaceState { ws: rc_core::Workspace::open(&root).unwrap(), solution: None });
        let mut harness = harness_app(app, egui::vec2(640.0, 320.0));
        harness.get_by_label(tr("Tools")).click();
        harness.run_steps(3);
        harness.get_by_label("NuGet").click();
        harness.run_steps(2);
        assert!(harness.state().dock.is_open(ToolWindow::Nuget));
        harness.get_by_role_and_label(egui::accesskit::Role::Button, tr("Main Menu")).click();
        harness.run_steps(3);
        harness.get_by_label(tr("Tools")).click();
        harness.run_steps(3);
        harness.get_by_label(tr(".NET CLI")).click();
        harness.run_steps(2);
        assert!(matches!(harness.state().dialog, Some(Dialog::Settings { page: SettingsPage::Tools, .. })));
    }

    #[test]
    fn menu_clipboard_actions_use_platform_events_and_edit_the_selected_document() {
        let _home = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("clipboard.txt");
        std::fs::write(&path, "hello world").unwrap();
        let mut app = crate::screens::test_app(home.path(), true);
        app.open_file(&path);
        app.docs.active_doc_mut().unwrap().editor.selection = rc_text::Selection::single(rc_text::Range::new(0, 5));
        let mut harness = crate::screens::harness(app, egui::vec2(900.0, 600.0));
        for label in [tr("Copy"), tr("Cut"), tr("Paste")] {
            harness.get_by_role_and_label(egui::accesskit::Role::Button, tr("Main Menu")).click();
            harness.run_steps(3);
            harness.get_by_label(tr("Edit")).click();
            harness.run_steps(3);
            harness.get_by_label(label).click();
            let mut copied = Vec::new();
            let mut paste_requested = false;
            for _ in 0..4 {
                harness.run_steps(1);
                copied.extend(harness.output().platform_output.commands.iter().filter_map(|command| {
                    if let egui::OutputCommand::CopyText(text) = command { Some(text.clone()) } else { None }
                }));
                paste_requested |= harness.output().viewport_output.values().any(|viewport| viewport.commands.iter().any(|command| matches!(command, egui::ViewportCommand::RequestPaste)));
            }
            let doc = harness.state().docs.active_doc().unwrap();
            if label == tr("Copy") {
                assert!(copied.iter().any(|text| text == "hello"), "Copy must emit the selected text: {copied:?}");
                assert_eq!(doc.editor.buffer.rope().to_string(), "hello world");
                assert_eq!(doc.editor.caret(), 5);
            } else if label == tr("Cut") {
                assert!(copied.iter().any(|text| text == "hello"), "Cut must emit the selected text: {copied:?}");
                assert_eq!(doc.editor.buffer.rope().to_string(), " world");
                assert_eq!(doc.editor.caret(), 0);
            } else {
                assert!(paste_requested, "Paste must request clipboard contents from the platform");
            }
            assert!(harness.state().pending.iter().all(|action| !matches!(action, Action::Copy | Action::Cut | Action::Paste)));
        }
    }
}
