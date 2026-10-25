//! The application: workspace, open documents, tool window layout, settings and session, and
//! the frame loop that ties them together.

use crate::docs::Docs;
use crate::insight::{Insight, InsightOutput, NavKind, Target};
use crate::runner::Runner;
use crate::native_menu::{MenuCommand, NativeMenu};
use crate::toolwindow::{Dock, ToolWindow};
use crate::ui::dialogs::Dialog;
use crate::ui::explorer::{ExplorerAction, ExplorerState};
use egui::{Context, Ui};
use rc_core::session::{OpenFile, Session};
use rc_core::settings::{KeymapChoice, ThemeChoice};
use rc_core::watch::{FileWatcher, FsEvent};
use rc_core::{Inbox, Settings, Waker, Workspace};
use rc_dotnet::Solution;
use rc_editor::{Action, EditorConfig, Keymap, KeymapKind};
use rc_kit::i18n::{self, Lang, tr, trf};
use std::path::{Path, PathBuf};

/// A message in the status bar / notification balloon.
#[derive(Clone, Debug)]
pub struct Notice {
    pub text: String,
    pub error: bool,
    pub at: f64,
}

pub struct WorkspaceState {
    pub ws: Workspace,
    pub solution: Option<Solution>,
}

pub struct DotCraftApp {
    pub settings: Settings,
    pub keymap: Keymap,
    pub editor_config: EditorConfig,
    pub workspace: Option<WorkspaceState>,
    pub docs: Docs,
    pub dock: Dock,
    pub explorer: ExplorerState,
    pub dialog: Option<Dialog>,
    pub notices: Vec<Notice>,
    pub insight: Insight,
    pub runner: Runner,
    pub debugger: crate::debugger::Debugger,
    window_title: String,
    pub files: crate::search::Files,
    pub vcs: crate::vcs::Vcs,
    pub nuget: Option<crate::ui::nuget::NugetDialog>,
    pub new_project: Option<crate::ui::new_project::NewProjectDialog>,
    pub search_everywhere: Option<crate::search::SearchEverywhere>,
    pub find_bar: Option<crate::search::FindBar>,
    pub find_in_files: Option<crate::search::FindInFiles>,
    pub terminals: rc_term::Terminals,
    pub focus_terminal: bool,
    /// Results shown in the Find tool window (usages, implementations…).
    pub find_results: Option<(String, Vec<Target>)>,
    /// Screen position under the active caret (popups that belong to the editor).
    pub caret_screen: Option<egui::Pos2>,
    pub test_markers: Vec<crate::runner::TestMarker>,
    pub native_menu: Option<NativeMenu>,
    pub waker: Waker,
    fs_inbox: Inbox<FsEvent>,
    _watcher: Option<FileWatcher>,
    /// Actions waiting for the UI pass (from the native menu or global shortcuts).
    pub pending: Vec<Action>,
    /// Give the editor keyboard focus on the next frame.
    pub focus_editor: bool,
    /// Back/forward navigation: (file, char offset).
    pub nav_back: Vec<(PathBuf, usize)>,
    pub nav_forward: Vec<(PathBuf, usize)>,
    applied_dark: Option<bool>,
    applied_zoom: Option<f32>,
    pub shift_taps: (f64, u8, bool),
    pub session_dirty: bool,
    pub last_session_save: f64,
    /// Editor keeps the focus when the window regains it.
    pub had_focus: bool,
}

impl DotCraftApp {
    pub fn new(cc: &eframe::CreationContext<'_>, open: Option<PathBuf>) -> Self {
        let settings = Settings::load();
        i18n::set_lang(settings.language.as_deref().map(Lang::from_locale).unwrap_or_else(Lang::system));
        rc_kit::theme::install_fonts(&cc.egui_ctx);
        let ctx = cc.egui_ctx.clone();
        let waker = Waker::new(move || ctx.request_repaint());
        let keymap = Keymap::new(keymap_kind(settings.keymap));
        let native_menu = NativeMenu::install(&cc.egui_ctx, &keymap);
        let mut app = Self::with_settings(settings, waker, native_menu);
        crate::open_files::set_context(&cc.egui_ctx);
        // Files sent by Finder at launch arrive before the first frame: open them instead of
        // the last project.
        let mut from_system = crate::open_files::take();
        from_system.extend(open);
        let to_open = if from_system.is_empty() { app.settings.recent_workspaces.first().cloned().filter(|p| p.exists()).into_iter().collect() } else { from_system };
        for path in to_open {
            app.open_path(&path);
        }
        app
    }

    /// Builds the app without a window (tests and headless captures).
    pub fn with_settings(settings: Settings, waker: Waker, native_menu: Option<NativeMenu>) -> Self {
        let keymap = Keymap::new(keymap_kind(settings.keymap));
        let editor_config = editor_config(&settings);
        let pool = rc_syntax::ParserPool::new(waker.as_fn());
        DotCraftApp {
            keymap,
            editor_config,
            workspace: None,
            docs: Docs::new(pool),
            dock: Dock::default(),
            explorer: ExplorerState::default(),
            dialog: None,
            notices: Vec::new(),
            insight: Insight::new(waker.clone()),
            runner: Runner::new(waker.clone()),
            debugger: crate::debugger::Debugger::new(waker.clone()),
            window_title: String::new(),
            files: crate::search::Files::new(),
            vcs: crate::vcs::Vcs::new(waker.clone()),
            nuget: None,
            new_project: None,
            search_everywhere: None,
            find_bar: None,
            find_in_files: None,
            terminals: rc_term::Terminals::new(),
            focus_terminal: false,
            find_results: None,
            caret_screen: None,
            test_markers: Vec::new(),
            native_menu,
            fs_inbox: Inbox::new(waker.clone()),
            waker,
            _watcher: None,
            pending: Vec::new(),
            focus_editor: false,
            nav_back: Vec::new(),
            nav_forward: Vec::new(),
            applied_dark: None,
            applied_zoom: None,
            shift_taps: (0.0, 0, false),
            session_dirty: false,
            last_session_save: 0.0,
            had_focus: true,
            settings,
        }
    }

    pub fn dark(&self, ctx: &Context) -> bool {
        match self.settings.theme {
            ThemeChoice::Dark => true,
            ThemeChoice::Light => false,
            ThemeChoice::System => ctx.system_theme().is_none_or(|t| t == egui::Theme::Dark),
        }
    }

    pub fn notify(&mut self, text: impl Into<String>, error: bool) {
        self.notices.push(Notice { text: text.into(), error, at: 0.0 });
    }

    // ------------------------------------------------------------------ workspace

    /// Opens a folder, solution or project, restoring its session.
    pub fn open_workspace(&mut self, path: &Path) {
        let ws = match Workspace::open(path) {
            Ok(ws) => ws,
            Err(e) => {
                self.notify(trf("Could not open {path}: {error}", &[("path", &path.display()), ("error", &e)]), true);
                return;
            }
        };
        self.close_workspace();
        let (solution, solution_error) = match ws.solution() {
            Some(sln) => match Solution::load(sln) {
                Ok(s) => (Some(s), None),
                Err(e) => (None, Some(e.to_string())),
            },
            None => (None, None),
        };
        if let Some(err) = &solution_error {
            self.notify(trf("Could not read the solution: {error}", &[("error", err)]), true);
        }
        self.settings.add_recent(ws.key_path().to_path_buf());
        let _ = self.settings.save();
        self._watcher = FileWatcher::new(&ws.root, self.fs_inbox.sender()).ok();
        self.files.open(&ws.root, self.waker.clone());
        self.vcs.open(&ws.root);
        let session = Session::load(ws.key_path());
        let projects = match &ws.kind {
            rc_core::WorkspaceKind::Project(p) => vec![p.clone()],
            rc_core::WorkspaceKind::Folder => find_projects(&ws.root, 4),
            rc_core::WorkspaceKind::Solution(_) => Vec::new(),
        };
        let solution_path = if solution.is_some() { ws.solution().map(Path::to_path_buf) } else { None };
        self.insight.start(&ws.root, solution_path.as_deref(), projects.clone(), dotnet_path(&self.settings), self.settings.roslyn_path.clone());
        self.runner.compounds = session.compounds.clone();
        self.runner.set_workspace(solution.as_ref(), &projects, &dotnet_path(&self.settings), session.run_configuration.as_deref());
        self.workspace = Some(WorkspaceState { ws, solution });
        self.restore_session(session);
    }

    /// Opens the workspace again (after adding a project: run configurations, explorer and the
    /// language server pick it up).
    pub fn reload_workspace(&mut self) {
        if let Some(path) = self.workspace.as_ref().map(|w| w.ws.key_path().to_path_buf()) {
            self.open_workspace(&path);
        }
    }

    /// New Solution, or Add New Project when `solution` is given.
    pub fn new_project_dialog(&mut self, solution: Option<PathBuf>) {
        self.new_project = Some(crate::ui::new_project::NewProjectDialog::new(self, solution, self.waker.clone()));
    }

    pub fn close_workspace(&mut self) {
        if self.workspace.is_some() {
            self.save_all_docs();
            self.save_session();
        }
        self.workspace = None;
        self._watcher = None;
        self.insight.stop();
        self.runner.stop_all();
        self.runner.clear();
        self.files.close();
        self.vcs.close();
        self.find_in_files = None;
        self.search_everywhere = None;
        self.find_bar = None;
        self.terminals = rc_term::Terminals::new();
        self.find_results = None;
        self.docs.list.clear();
        self.docs.active = None;
        self.explorer = ExplorerState::default();
        self.dock = Dock::default();
        self.nav_back.clear();
        self.nav_forward.clear();
    }

    fn restore_session(&mut self, s: Session) {
        self.dock = Dock::from_session(&s.tool_windows);
        self.explorer.expanded = s.expanded.iter().cloned().collect();
        if self.explorer.expanded.is_empty() {
            // First open: the solution and its top folders expanded, like Rider.
            self.explorer.expanded.insert("sln".into());
            if let Some(sln) = self.workspace.as_ref().and_then(|w| w.solution.as_ref()) {
                for item in &sln.items {
                    if let rc_dotnet::SolutionItem::Folder { name, .. } = item {
                        self.explorer.expanded.insert(format!("sf:/{name}"));
                    }
                }
            }
        }
        for f in &s.files {
            if !f.path.exists() {
                continue;
            }
            if let Ok(i) = self.docs.open(&f.path) {
                let d = &mut self.docs.list[i];
                d.pinned = f.pinned;
                let pos = d.editor.buffer.pos_at(f.line, f.col);
                d.editor.selection = rc_text::Selection::point(pos);
                if d.editor.syntax.tree().is_none() {
                    d.editor.parse_blocking();
                }
                for &line in &f.folded {
                    d.editor.toggle_fold(line);
                }
                d.editor.set_scroll_offset(egui::vec2(0.0, f.scroll_y));
            }
        }
        self.docs.active = s.active.filter(|&a| a < self.docs.list.len()).or(if self.docs.list.is_empty() { None } else { Some(0) });
        for (path, bps) in &s.breakpoints {
            if let Some(i) = self.docs.index_of(path) {
                for (line, cond) in bps {
                    let cond = cond.as_ref().filter(|c| !c.is_empty());
                    let kind = if cond.is_some() { rc_editor::BreakpointKind::Conditional } else { rc_editor::BreakpointKind::Normal };
                    self.docs.list[i].editor.breakpoints.insert(*line, kind);
                    if let Some(c) = cond {
                        self.debugger.conditions.insert((path.clone(), *line), c.clone());
                    }
                }
                for line in s.disabled_breakpoints.get(path).into_iter().flatten() {
                    if let Some(k) = self.docs.list[i].editor.breakpoints.get_mut(line) {
                        *k = rc_editor::BreakpointKind::Disabled;
                    }
                }
            }
        }
        self.focus_editor = true;
    }

    pub fn session(&self) -> Session {
        let files = self
            .docs
            .list
            .iter()
            .map(|d| {
                let (line, col) = d.editor.caret_line_col();
                let folded = d.editor.syntax.folds().iter().filter(|f| d.editor.is_folded(f.start_line)).map(|f| f.start_line).collect();
                OpenFile { path: d.path.clone(), line, col, scroll_y: d.editor.scroll_offset().y, folded, pinned: d.pinned }
            })
            .collect();
        let breakpoints = self
            .docs
            .list
            .iter()
            .filter(|d| !d.editor.breakpoints.is_empty())
            .map(|d| (d.path.clone(), d.editor.breakpoints.keys().map(|l| (*l, self.debugger.conditions.get(&(d.path.clone(), *l)).cloned())).collect()))
            .collect();
        Session {
            files,
            active: self.docs.active,
            tool_windows: self.dock.to_session(),
            expanded: {
                let mut v: Vec<String> = self.explorer.expanded.iter().cloned().collect();
                v.sort();
                v
            },
            run_configuration: self.runner.selected_config().map(|c| c.name.clone()),
            compounds: self.runner.compounds.clone(),
            breakpoints,
            disabled_breakpoints: self
                .docs
                .list
                .iter()
                .map(|d| (d.path.clone(), d.editor.breakpoints.iter().filter(|(_, k)| **k == rc_editor::BreakpointKind::Disabled).map(|(l, _)| *l).collect::<Vec<_>>()))
                .filter(|(_, v)| !v.is_empty())
                .collect(),
        }
    }

    pub fn save_session(&mut self) {
        if let Some(w) = &self.workspace
            && let Err(e) = self.session().save(w.ws.key_path())
        {
            eprintln!("could not save the session: {e:#}");
        }
        self.session_dirty = false;
    }

    pub fn save_all_docs(&mut self) {
        let modified: Vec<PathBuf> = self.docs.list.iter().filter(|d| d.is_modified()).map(|d| d.path.clone()).collect();
        for e in self.docs.save_all() {
            self.notify(e, true);
        }
        for p in modified {
            self.insight.did_save(&p);
        }
    }

    /// Code-insight results that leave the editor: navigation, result lists, dialogs.
    pub fn handle_insight(&mut self, outputs: Vec<InsightOutput>) {
        for o in outputs {
            match o {
                InsightOutput::Navigate(t) => self.open_at(&t.path, t.line, t.col),
                InsightOutput::Usages { title, targets } => {
                    if targets.len() == 1 {
                        let t = &targets[0];
                        self.open_at(&t.path.clone(), t.line, t.col);
                    } else {
                        self.find_results = Some((title, targets));
                        self.dock.show(ToolWindow::Find);
                    }
                }
                InsightOutput::Actions(list) => self.dialog = Some(Dialog::CodeActions { list, selected: 0 }),
                InsightOutput::AskRename { doc, pos, name } => self.dialog = Some(Dialog::RenameSymbol { doc, pos, original: name.clone(), name, focus: true }),
                InsightOutput::Message(m) => self.notify(m, false),
            }
        }
    }

    /// Runs a code-insight action from the editor's keymap on the active document.
    pub fn run_insight_action(&mut self, action: Action, caret_screen: egui::Pos2) {
        let Some(i) = self.docs.active else { return };
        if !self.insight.ready() || !crate::insight::is_csharp(&self.docs.list[i]) {
            let msg = match &self.insight.status {
                crate::insight::InsightStatus::NotInstalled => tr("Download the C# language server from the status bar to use this action"),
                crate::insight::InsightStatus::Failed(_) => tr("The C# language server stopped; restart it from the status bar"),
                _ if !crate::insight::is_csharp(&self.docs.list[i]) => tr("This action is available in C# files"),
                _ => tr("The C# language server is still loading the solution"),
            };
            self.notify(msg, false);
            return;
        }
        let doc = &mut self.docs.list[i];
        self.insight.flush(doc);
        let caret = doc.editor.caret();
        match action {
            Action::Complete => self.insight.complete(doc, None),
            Action::ParameterInfo => self.insight.parameter_info(doc),
            Action::QuickDoc => self.insight.quick_doc(doc, caret_screen),
            Action::ShowIntentions => self.insight.code_actions(doc),
            Action::GotoDeclaration => {
                self.push_nav();
                let doc = &self.docs.list[i];
                self.insight.navigate(doc, caret, NavKind::Declaration);
            }
            Action::GotoImplementation => {
                self.push_nav();
                let doc = &self.docs.list[i];
                self.insight.navigate(doc, caret, NavKind::Implementation);
            }
            Action::GotoTypeDeclaration => {
                self.push_nav();
                let doc = &self.docs.list[i];
                self.insight.navigate(doc, caret, NavKind::TypeDeclaration);
            }
            Action::FindUsages => self.insight.navigate(doc, caret, NavKind::Usages),
            Action::Rename => self.insight.prepare_rename(doc),
            Action::Reformat => self.insight.format(doc),
            Action::FileStructure => self.dialog = Some(Dialog::FileStructure { filter: String::new(), selected: 0 }),
            _ => {}
        }
    }

    // ------------------------------------------------------------------ documents

    /// Opens a file in an editor tab, remembering the current place for "Back".
    pub fn open_file(&mut self, path: &Path) {
        self.push_nav();
        match self.docs.open(path) {
            Ok(_) => {
                self.focus_editor = true;
                self.session_dirty = true;
            }
            Err(e) => self.notify(e, true),
        }
    }

    /// Opens `path` and puts the caret at (line, col).
    pub fn open_at(&mut self, path: &Path, line: usize, col: usize) {
        self.open_file(path);
        if let Some(d) = self.docs.active_doc_mut() {
            d.editor.goto(line, col);
        }
    }

    pub fn push_nav(&mut self) {
        if let Some(d) = self.docs.active_doc() {
            let here = (d.path.clone(), d.editor.caret());
            if self.nav_back.last() != Some(&here) {
                self.nav_back.push(here);
                if self.nav_back.len() > 100 {
                    self.nav_back.remove(0);
                }
            }
            self.nav_forward.clear();
        }
    }

    fn navigate(&mut self, back: bool) {
        let (from, to) = if back { (&mut self.nav_back, &mut self.nav_forward) } else { (&mut self.nav_forward, &mut self.nav_back) };
        let Some((path, pos)) = from.pop() else { return };
        if let Some(d) = self.docs.active_doc() {
            to.push((d.path.clone(), d.editor.caret()));
        }
        if self.docs.open(&path).is_ok()
            && let Some(d) = self.docs.active_doc_mut()
        {
            let pos = pos.min(d.editor.buffer.len_chars());
            d.editor.set_selection(rc_text::Selection::point(pos));
            self.focus_editor = true;
        }
    }

    /// Closes tab `i`, saving it first (IntelliJ autosaves; nothing is ever lost on close).
    pub fn close_tab(&mut self, i: usize) {
        if i < self.docs.list.len() && self.docs.list[i].is_modified()
            && let Err(e) = self.docs.save(i)
        {
            self.notify(e, true);
            return;
        }
        self.docs.close(i);
        self.session_dirty = true;
    }

    // ------------------------------------------------------------------ actions

    /// Runs an IDE action (global shortcuts, menus, toolbar buttons).
    pub fn run_ide_action(&mut self, ctx: &Context, action: Action) {
        match action {
            Action::SaveAll | Action::Save => {
                self.save_all_docs();
                self.save_session();
            }
            Action::CloseTab => {
                if let Some(i) = self.docs.active {
                    self.close_tab(i);
                }
            }
            Action::NextTab | Action::PreviousTab => {
                if let Some(a) = self.docs.active {
                    let n = self.docs.list.len();
                    let next = if action == Action::NextTab { (a + 1) % n } else { (a + n - 1) % n };
                    self.docs.activate(next);
                    self.focus_editor = true;
                }
            }
            Action::NavigateBack => self.navigate(true),
            Action::NavigateForward => self.navigate(false),
            Action::ShowSettings => self.dialog = Some(Dialog::settings(&self.settings)),
            Action::ToggleProjectWindow => self.toggle_tool(ToolWindow::Explorer),
            Action::ToggleTerminal => self.toggle_tool(ToolWindow::Terminal),
            Action::ToggleGitWindow => self.toggle_tool(ToolWindow::Git),
            Action::ToggleProblems => self.toggle_tool(ToolWindow::Problems),
            Action::HideToolWindows => {
                self.dock.hide_all();
                self.focus_editor = true;
                self.session_dirty = true;
            }
            Action::OpenFolder => self.pick_and_open(),
            Action::Build => {
                self.save_all_docs();
                if let Some(target) = self.workspace.as_ref().map(|w| w.ws.key_path().to_path_buf()) {
                    let outs = self.runner.build(&target, rc_dotnet::BuildAction::Build);
                    crate::ui::run_windows::handle_outputs(self, outs);
                }
            }
            Action::Run => {
                self.save_all_docs();
                let outs = self.runner.run_selected();
                crate::ui::run_windows::handle_outputs(self, outs);
            }
            Action::RunTests => {
                self.save_all_docs();
                if let Some(target) = self.workspace.as_ref().map(|w| w.ws.key_path().to_path_buf()) {
                    let outs = self.runner.run_all_tests(&target);
                    crate::ui::run_windows::handle_outputs(self, outs);
                }
            }
            Action::StopRun => {
                if self.debugger.active() {
                    self.debugger.stop();
                } else {
                    self.runner.stop_all();
                }
            }
            // F5 while paused continues (Visual Studio).
            Action::Debug if self.debugger.stopped() => {
                if let Some(s) = &mut self.debugger.session {
                    s.continue_();
                }
            }
            Action::Debug => {
                self.save_all_docs();
                match self.runner.selected_config().cloned() {
                    Some(config) => {
                        let dotnet = dotnet_path(&self.settings);
                        let outs = self.debugger.start(config, &dotnet, self.settings.netcoredbg_path.as_deref(), &self.docs);
                        self.handle_debug(outs);
                    }
                    None => self.notify(tr("There is no run configuration to debug"), false),
                }
            }
            Action::Resume | Action::StepOver | Action::StepInto | Action::StepOut => {
                if let Some(s) = self.debugger.session.as_mut().filter(|s| s.state().is_stopped()) {
                    match action {
                        Action::Resume => s.continue_(),
                        Action::StepOver => s.step_over(),
                        Action::StepInto => s.step_in(),
                        _ => s.step_out(),
                    }
                }
            }
            Action::RunToCursor => {
                if let Some((path, line)) = self.docs.active_doc().map(|d| (d.path.clone(), d.editor.caret_line_col().0)) {
                    self.debugger.run_to_cursor(&path, line);
                }
            }
            Action::Evaluate => {
                if self.debugger.stopped() {
                    let text = self.docs.active_doc().map(|d| d.editor.selected_text()).filter(|t| !t.contains('\n')).unwrap_or_default();
                    if !text.is_empty() {
                        self.debugger.evaluate(text.clone());
                    }
                    self.dialog = Some(Dialog::Evaluate { expression: text, focus: true });
                }
            }
            Action::Commit => self.dock.show(ToolWindow::Commit),
            Action::SearchEverywhere | Action::GotoFile | Action::GotoClass | Action::GotoSymbol => {
                if self.workspace.is_some() {
                    let tab = match action {
                        Action::GotoFile => crate::search::SeTab::Files,
                        Action::GotoClass => crate::search::SeTab::Types,
                        Action::GotoSymbol => crate::search::SeTab::Symbols,
                        _ => crate::search::SeTab::All,
                    };
                    // The selected text (or the word at the caret) pre-fills the search.
                    let text = self.docs.active_doc().map(|d| d.editor.selected_text()).filter(|t| !t.contains('\n')).unwrap_or_default();
                    self.search_everywhere = Some(crate::search::SearchEverywhere::new(tab, text));
                }
            }
            Action::FindInFiles | Action::ReplaceInFiles => {
                if let Some(root) = self.workspace.as_ref().map(|w| w.ws.root.clone()) {
                    let text = self.docs.active_doc().map(|d| d.editor.selected_text()).filter(|t| !t.is_empty() && !t.contains('\n'));
                    let fif = self.find_in_files.get_or_insert_with(|| crate::search::FindInFiles::new(&root, self.waker.clone(), String::new()));
                    if let Some(t) = text {
                        fif.query = t;
                    }
                    fif.show_replace = action == Action::ReplaceInFiles;
                    fif.focus = true;
                    fif.search();
                    self.find_results = None;
                    self.dock.show(ToolWindow::Find);
                }
            }
            Action::ZoomIn => self.set_zoom(ctx, self.settings.ui_zoom + 0.1),
            Action::ZoomOut => self.set_zoom(ctx, self.settings.ui_zoom - 0.1),
            Action::ZoomReset => self.set_zoom(ctx, 1.0),
            Action::RecentFiles => self.dialog = Some(Dialog::RecentFiles { selected: 1.min(self.docs.recent.len().saturating_sub(1)), filter: String::new() }),
            Action::RecentLocations => {
                if !self.nav_back.is_empty() {
                    self.dialog = Some(Dialog::RecentLocations { selected: 0 });
                }
            }
            Action::GotoLine => {
                if let Some(d) = self.docs.active_doc() {
                    let (l, c) = d.editor.caret_line_col();
                    self.dialog = Some(Dialog::GotoLine { text: format!("{}:{}", l + 1, c + 1), focus: true });
                }
            }
            Action::Escape => {
                // Esc in a tool window returns to the editor (IntelliJ).
                self.focus_editor = true;
            }
            _ => {}
        }
    }

    pub fn handle_debug(&mut self, outs: Vec<crate::debugger::DebugOutput>) {
        use crate::debugger::DebugOutput;
        for o in outs {
            match o {
                DebugOutput::Notify(m, err) => self.notify(m, err),
                DebugOutput::ShowDebugWindow => self.dock.show(ToolWindow::Debug),
                DebugOutput::ShowLine(path, line) => {
                    if self.docs.active_doc().is_none_or(|d| d.path != path) || self.docs.active_doc().is_some_and(|d| d.editor.caret_line_col().0 != line) {
                        self.open_at(&path, line, 0);
                    }
                }
            }
        }
    }

    /// Build errors and warnings go to the Problems window next to the server's diagnostics.
    pub fn set_build_problems(&mut self, list: Vec<rc_dotnet::Diagnostic>) {
        let mut per_file: std::collections::BTreeMap<PathBuf, Vec<rc_editor::Diagnostic>> = std::collections::BTreeMap::new();
        for d in list {
            let Some(file) = d.file.clone() else { continue };
            let text = match self.docs.index_of(&file) {
                Some(i) => self.docs.list[i].editor.buffer.rope().clone(),
                None => rc_text::Rope::from_str(&std::fs::read_to_string(&file).unwrap_or_default().replace("\r\n", "\n")),
            };
            let line = d.line.unwrap_or(1).saturating_sub(1) as usize;
            let line = line.min(text.len_lines().saturating_sub(1));
            let start = text.line_to_char(line) + d.column.unwrap_or(1).saturating_sub(1) as usize;
            let start = start.min(text.len_chars());
            per_file.entry(file).or_default().push(rc_editor::Diagnostic {
                start,
                end: start,
                severity: if d.severity == rc_dotnet::Severity::Error { rc_editor::Severity::Error } else { rc_editor::Severity::Warning },
                message: d.message,
                code: (!d.code.is_empty()).then_some(d.code),
                unnecessary: false,
                deprecated: false,
            });
        }
        for (file, diags) in per_file {
            self.insight.problems.insert(file, diags);
        }
    }

    /// Opens the declaration of a test from its fully qualified name (Tests window).
    pub fn goto_test(&mut self, name: &str) {
        let parts: Vec<&str> = name.rsplitn(3, '.').collect();
        let (method, class) = (parts.first().copied().unwrap_or(""), parts.get(1).copied().unwrap_or(""));
        let Some(root) = self.workspace.as_ref().map(|w| w.ws.root.clone()) else { return };
        // The class's file: `Class.cs` under the workspace, else any file declaring it.
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            for n in rc_core::workspace::list_dir(&dir) {
                if n.is_dir {
                    stack.push(n.path);
                } else if n.name.ends_with(".cs")
                    && let Ok(text) = std::fs::read_to_string(&n.path)
                    && text.contains(&format!("class {class}"))
                    && let Some(line) = text.lines().position(|l| l.contains(&format!(" {method}(")))
                {
                    self.open_at(&n.path, line, 0);
                    return;
                }
            }
        }
    }

    /// A new terminal tab in `dir` (the workspace folder by default).
    pub fn open_terminal(&mut self, dir: Option<PathBuf>) {
        let cwd = dir.or_else(|| self.workspace.as_ref().map(|w| w.ws.root.clone()));
        match self.terminals.open(&rc_term::SpawnSpec::shell(cwd), self.waker.clone()) {
            Ok(_) => {
                self.dock.show(ToolWindow::Terminal);
                self.focus_terminal = true;
            }
            Err(e) => self.notify(trf("Could not start the terminal: {error}", &[("error", &e)]), true),
        }
    }

    /// Run icons next to test classes and methods, with their last outcome.
    fn update_test_markers(&mut self) {
        let Some(i) = self.docs.active else { return };
        let doc = &mut self.docs.list[i];
        if !crate::insight::is_csharp(doc) {
            return;
        }
        let Some(tree) = doc.editor.syntax.tree() else { return };
        let markers = crate::runner::test_markers(doc.editor.buffer.rope(), tree);
        doc.editor.gutter_icons.retain(|_, icon| !matches!(icon, rc_editor::GutterIcon::RunTest | rc_editor::GutterIcon::TestPassed | rc_editor::GutterIcon::TestFailed));
        for m in &markers {
            let icon = match self.runner.test_outcomes.get(&m.name) {
                Some(crate::runner::TestState::Passed) => rc_editor::GutterIcon::TestPassed,
                Some(crate::runner::TestState::Failed) => rc_editor::GutterIcon::TestFailed,
                _ => rc_editor::GutterIcon::RunTest,
            };
            doc.editor.gutter_icons.insert(m.line, icon);
        }
        self.test_markers = markers;
    }

    /// Click on a test's gutter icon: runs that test (or class) in its project.
    pub fn run_test_at(&mut self, line: usize) {
        let Some(m) = self.test_markers.iter().find(|m| m.line == line).cloned() else { return };
        let Some(path) = self.docs.active_doc().map(|d| d.path.clone()) else { return };
        let Some(project) = crate::runner::project_of(&path, &self.runner.configs) else {
            self.notify(tr("This file is not part of a test project"), false);
            return;
        };
        self.save_all_docs();
        let filter = if m.is_class { rc_dotnet::TestFilter::Class(m.name.clone()) } else { rc_dotnet::TestFilter::Tests(vec![m.name.clone()]) };
        let outs = self.runner.run_tests(&project, Some(filter));
        crate::ui::run_windows::handle_outputs(self, outs);
    }

    pub fn toggle_tool(&mut self, tw: ToolWindow) {
        let was_open = self.dock.is_open(tw);
        self.dock.toggle(tw);
        if was_open {
            self.focus_editor = true;
        }
        self.session_dirty = true;
    }

    fn set_zoom(&mut self, ctx: &Context, z: f32) {
        self.settings.ui_zoom = (z * 10.0).round().clamp(5.0, 30.0) / 10.0;
        let _ = self.settings.save();
        ctx.request_repaint();
    }

    /// Opens a folder, solution or project as the workspace, and any other file in an editor
    /// (without a workspace it is edited on its own, like Rider's LightEdit).
    pub fn open_path(&mut self, path: &Path) {
        use rc_core::workspace::{is_project, is_solution};
        if path.is_dir() || is_solution(path) || is_project(path) {
            self.open_workspace(path);
        } else if path.is_file() {
            self.open_file(path);
        } else {
            self.notify(trf("Could not open {path}: {error}", &[("path", &path.display()), ("error", &tr("the file does not exist"))]), true);
        }
    }

    pub fn pick_file(&mut self) {
        let mut dialog = rfd::FileDialog::new().set_title(tr("Open File"));
        if let Some(dir) = self.docs.active_doc().and_then(|d| d.path.parent().map(Path::to_path_buf)).or_else(|| self.workspace.as_ref().map(|w| w.ws.root.clone())) {
            dialog = dialog.set_directory(dir);
        }
        if let Some(paths) = dialog.pick_files() {
            for p in paths {
                self.open_path(&p);
            }
        }
    }

    pub fn pick_and_open(&mut self) {
        let dialog = rfd::FileDialog::new().set_title(tr("Open Folder, Solution or Project"));
        if let Some(path) = dialog.pick_folder() {
            self.open_workspace(&path);
        }
    }

    pub fn pick_solution(&mut self) {
        let dialog = rfd::FileDialog::new().set_title(tr("Open Solution or Project")).add_filter(tr("Solutions and projects"), &["sln", "slnx", "slnf", "csproj", "fsproj"]);
        if let Some(path) = dialog.pick_file() {
            self.open_workspace(&path);
        }
    }

    pub fn apply_settings(&mut self, ctx: &Context, new: Settings) {
        let lang_changed = new.language != self.settings.language;
        let keymap_changed = new.keymap != self.settings.keymap;
        self.settings = new;
        self.editor_config = editor_config(&self.settings);
        if keymap_changed {
            self.keymap = Keymap::new(keymap_kind(self.settings.keymap));
        }
        if lang_changed {
            i18n::set_lang(self.settings.language.as_deref().map(Lang::from_locale).unwrap_or_else(Lang::system));
            if let Some(m) = &self.native_menu {
                m.relabel();
            }
        }
        if let Err(e) = self.settings.save() {
            self.notify(trf("Could not save the settings: {error}", &[("error", &e)]), true);
        }
        self.applied_dark = None;
        ctx.request_repaint();
    }

    // ------------------------------------------------------------------ explorer actions

    pub fn handle_explorer(&mut self, ctx: &Context, actions: Vec<ExplorerAction>) {
        for a in actions {
            match a {
                ExplorerAction::Open(p) => self.open_file(&p),
                ExplorerAction::Preview(p) => {
                    self.open_file(&p);
                    self.focus_editor = false;
                }
                ExplorerAction::Collapse => {
                    self.explorer.expanded.retain(|k| k == "sln");
                    self.session_dirty = true;
                }
                ExplorerAction::LocateActive => self.locate_active(),
                ExplorerAction::NewFile(dir) => self.dialog = Some(Dialog::NewFile { dir, name: String::new(), folder: false, error: None }),
                ExplorerAction::NewFolder(dir) => self.dialog = Some(Dialog::NewFile { dir, name: String::new(), folder: true, error: None }),
                ExplorerAction::Rename(path) => {
                    let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                    self.dialog = Some(Dialog::Rename { path, name, error: None });
                }
                ExplorerAction::Delete(path) => self.dialog = Some(Dialog::ConfirmDelete { path }),
                ExplorerAction::CopyPath(path) => ctx.copy_text(path.display().to_string()),
                ExplorerAction::Reveal(path) => reveal_in_file_manager(&path),
                ExplorerAction::OpenInTerminal(dir) => self.open_terminal(Some(dir)),
                ExplorerAction::AddProject(sln) => self.new_project_dialog(Some(sln)),
                ExplorerAction::ManagePackages(project) => self.nuget = Some(crate::ui::nuget::NugetDialog::new(project, self.waker.clone())),
            }
        }
    }

    pub fn locate_active(&mut self) {
        let Some(path) = self.docs.active_doc().map(|d| d.path.clone()) else { return };
        if let Some(w) = &self.workspace {
            self.explorer.locate(&path, w.solution.as_ref(), &w.ws.root);
            self.dock.show(ToolWindow::Explorer);
        }
    }

    // ------------------------------------------------------------------ frame

    fn poll_background(&mut self) {
        let mut changed: Vec<PathBuf> = Vec::new();
        let events = self.fs_inbox.drain();
        self.report_fs_to_server(&events);
        if let Some(index) = &self.files.index
            && !events.is_empty()
        {
            let outside_git: Vec<FsEvent> = events.iter().filter(|e| !e.path().components().any(|c| c.as_os_str() == ".git")).cloned().collect();
            index.apply(&outside_git);
        }
        for e in &events {
            self.vcs.on_fs_change(e.path());
        }
        for ev in events {
            let p = ev.path().to_path_buf();
            self.explorer.invalidate(&p);
            if !changed.contains(&p) {
                changed.push(p);
            }
        }
        for p in changed {
            self.docs.on_disk_change(&p);
            // A changed solution file reloads the explorer's model.
            if let Some(w) = &mut self.workspace
                && w.ws.solution() == Some(p.as_path())
                && let Ok(s) = Solution::load(&p)
            {
                // Projects added or removed by hand: new run configurations too.
                let remembered = self.runner.selected_config().map(|c| c.name.clone());
                self.runner.set_workspace(Some(&s), &[], &dotnet_path(&self.settings), remembered.as_deref());
                w.solution = Some(s);
            }
        }
        self.docs.poll_parses();
    }

    /// Changed C# files tell the language server to reload them (1 created, 2 changed, 3 deleted).
    fn report_fs_to_server(&self, events: &[FsEvent]) {
        let changes: Vec<(PathBuf, u8)> = events
            .iter()
            .filter(|e| e.path().extension().is_some_and(|x| x == "cs" || x == "csproj" || x == "props" || x == "targets"))
            .map(|e| match e {
                FsEvent::Created(p) => (p.clone(), 1),
                FsEvent::Changed(p) => (p.clone(), 2),
                FsEvent::Removed(p) => (p.clone(), 3),
            })
            .collect();
        if !changes.is_empty() {
            self.insight.files_changed(&changes);
        }
    }

    /// IDE shortcuts work whatever has the focus; they are taken out of the input so text
    /// fields don't see them.
    fn global_shortcuts(&mut self, ctx: &Context) {
        let now = ctx.input(|i| i.time);
        // Shift, Shift: Search Everywhere.
        let shift = ctx.input(|i| i.modifiers.shift_only());
        let any_key = ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { .. } | egui::Event::Text(_))));
        let (last, taps, was_down) = self.shift_taps;
        if any_key {
            self.shift_taps = (0.0, 0, shift);
        } else if shift && !was_down {
            let taps = if now - last < 0.4 { taps + 1 } else { 1 };
            self.shift_taps = (now, taps, true);
            if taps >= 2 {
                self.shift_taps = (0.0, 0, true);
                self.pending.push(Action::SearchEverywhere);
            }
        } else if !shift && was_down {
            self.shift_taps.2 = false;
        }
        let keymap = &self.keymap;
        let mut found = Vec::new();
        ctx.input_mut(|i| {
            i.events.retain(|e| {
                if let egui::Event::Key { key, pressed: true, modifiers, .. } = e
                    && let Some(a) = keymap.lookup(*key, *modifiers)
                    && a.is_ide()
                {
                    found.push(a);
                    return false;
                }
                true
            })
        });
        self.pending.extend(found);
    }
}

impl eframe::App for DotCraftApp {
    fn logic(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        // Window title: active file and workspace (Mission Control, the Window menu).
        let title = match (self.docs.active_doc(), &self.workspace) {
            (Some(d), Some(w)) => format!("{} – {} – DotCraft", d.name(), w.ws.name()),
            (Some(d), None) => format!("{} – DotCraft", d.name()),
            (None, Some(w)) => format!("{} – DotCraft", w.ws.name()),
            (None, None) => "DotCraft".to_string(),
        };
        if self.window_title != title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
        for path in crate::open_files::take() {
            self.open_path(&path);
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        if let Some(menu) = &self.native_menu {
            for cmd in menu.poll() {
                match cmd {
                    MenuCommand::Act(a) => self.pending.push(a),
                    MenuCommand::About => self.dialog = Some(Dialog::About),
                    MenuCommand::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                    MenuCommand::OpenProject => self.pick_and_open(),
                    MenuCommand::OpenFile => self.pick_file(),
                    MenuCommand::NewSolution => self.new_project_dialog(None),
                    MenuCommand::CloseProject => self.close_workspace(),
                    MenuCommand::ThemeLight | MenuCommand::ThemeDark => {
                        let mut s = self.settings.clone();
                        s.theme = if cmd == MenuCommand::ThemeDark { ThemeChoice::Dark } else { ThemeChoice::Light };
                        self.apply_settings(ctx, s);
                    }
                    _ => {}
                }
            }
        }
        self.global_shortcuts(ctx);
        self.poll_background();
        let dark = self.dark(ctx);
        if self.applied_dark != Some(dark) {
            rc_kit::theme::apply(ctx, dark);
            self.applied_dark = Some(dark);
        }
        if let Some(m) = &self.native_menu {
            m.sync(dark);
        }
        if self.applied_zoom != Some(self.settings.ui_zoom) {
            ctx.set_zoom_factor(self.settings.ui_zoom);
            self.applied_zoom = Some(self.settings.ui_zoom);
        }
        // Autosave when the window loses focus (IntelliJ's "save on frame deactivation").
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        if self.had_focus && !focused && self.settings.autosave {
            self.save_all_docs();
            self.save_session();
        }
        self.had_focus = focused;
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let pending = std::mem::take(&mut self.pending);
        let mut editor_actions = Vec::new();
        for a in pending {
            if a.is_ide() {
                self.run_ide_action(&ctx, a);
            } else {
                editor_actions.push(a);
            }
        }
        let outputs = self.insight.update(&mut self.docs, self.settings.show_inlay_hints);
        self.handle_insight(outputs);
        let outs = self.runner.update();
        crate::ui::run_windows::handle_outputs(self, outs);
        let outs = self.debugger.update(&mut self.docs);
        self.handle_debug(outs);
        for (_, ev) in self.terminals.poll() {
            if let rc_term::SessionEvent::Clipboard(text) = ev {
                ctx.copy_text(text);
            }
        }
        self.update_test_markers();
        let dark = self.dark(&ctx);
        for n in self.vcs.update(&mut self.docs, dark) {
            self.notify(n, false);
        }
        crate::ui::frame(self, ui, editor_actions);
        self.docs.reparse_edited();
        let now = ctx.input(|i| i.time);
        if self.session_dirty && now - self.last_session_save > 2.0 {
            self.save_session();
            self.last_session_save = now;
        }
    }

    fn on_exit(&mut self) {
        self.runner.stop_all();
        self.debugger.stop();
        self.insight.stop();
        self.save_all_docs();
        self.save_session();
    }
}

pub fn keymap_kind(choice: KeymapChoice) -> KeymapKind {
    match choice {
        KeymapChoice::VisualStudio => KeymapKind::VisualStudio,
        KeymapChoice::Rider if cfg!(target_os = "macos") => KeymapKind::RiderMac,
        KeymapChoice::Rider => KeymapKind::Rider,
    }
}

pub fn editor_config(s: &Settings) -> EditorConfig {
    EditorConfig {
        font: rc_editor::layout::EditorFont { size: s.editor_font_size.clamp(8.0, 32.0), line_spacing: s.editor_line_spacing.clamp(1.0, 2.0) },
        show_indent_guides: s.show_indent_guides,
        show_inlays: s.show_inlay_hints,
        right_margin: (s.right_margin > 0).then_some(s.right_margin),
        ..EditorConfig::default()
    }
}

/// The configured `dotnet`, or the default install location, or `dotnet` from PATH.
pub fn dotnet_path(s: &Settings) -> PathBuf {
    if let Some(p) = s.dotnet_path.clone().filter(|p| p.exists()) {
        return p;
    }
    let candidates: &[&str] = if cfg!(windows) { &["C:\\Program Files\\dotnet\\dotnet.exe"] } else { &["/usr/local/share/dotnet/dotnet", "/opt/homebrew/bin/dotnet", "/usr/local/bin/dotnet", "/usr/share/dotnet/dotnet"] };
    candidates.iter().map(PathBuf::from).find(|p| p.exists()).unwrap_or_else(|| PathBuf::from("dotnet"))
}

/// Project files under `root` (for folders without a solution), skipping build output.
pub fn find_projects(root: &Path, depth: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0)];
    while let Some((dir, d)) = stack.pop() {
        for n in rc_core::workspace::list_dir(&dir) {
            if n.is_dir && d < depth {
                stack.push((n.path, d + 1));
            } else if rc_core::workspace::is_project(&n.path) {
                out.push(n.path);
            }
        }
    }
    out.sort();
    out
}

pub fn reveal_in_file_manager(path: &Path) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer").arg(format!("/select,{}", path.display())).spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = std::process::Command::new("xdg-open").arg(path.parent().unwrap_or(path)).spawn();
}
