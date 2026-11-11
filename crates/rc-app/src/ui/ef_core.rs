//! Entity Framework Core (Rider's Tools › Entity Framework Core): add / remove migrations,
//! update the database, generate SQL scripts and scaffold a DbContext from a database. One
//! dialog with the operations on the left and the projects / DbContext on top; the command
//! runs in the Build window through `rc_dotnet::ef`.

use crate::app::DotCraftApp;
use crate::runner::ToolFollowup;
use crate::ui::kit;
use egui::{Align2, Context, FontId, Id, Rect, Sense, Ui, pos2, vec2};
use rc_core::{Inbox, Runtime, Waker};
use rc_dotnet::ef::{self, DbContextInfo, EfCommand, EfTarget, Migration, Scaffold};
use rc_dotnet::{CancelToken, Dotnet, ProjectInfo};
use rc_kit::i18n::{tr, trf};
use rc_kit::{Ico, ico, icons, theme};
use std::path::{Path, PathBuf};

/// The operations of the dialog (and of the explorer's Entity Framework Core submenu).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EfOp {
    AddMigration,
    RemoveMigration,
    UpdateDatabase,
    Script,
    Scaffold,
}

impl EfOp {
    pub const ALL: [EfOp; 5] = [EfOp::AddMigration, EfOp::RemoveMigration, EfOp::UpdateDatabase, EfOp::Script, EfOp::Scaffold];

    /// Menu and list label (English source text).
    pub fn title(self) -> &'static str {
        match self {
            EfOp::AddMigration => "Add Migration",
            EfOp::RemoveMigration => "Remove Last Migration",
            EfOp::UpdateDatabase => "Update Database",
            EfOp::Script => "Generate SQL Script",
            EfOp::Scaffold => "Scaffold DbContext",
        }
    }

    fn description(self) -> &'static str {
        match self {
            EfOp::AddMigration => "Creates a migration with the model changes since the last one.",
            EfOp::RemoveMigration => "Deletes the last migration and restores the model snapshot.",
            EfOp::UpdateDatabase => "Applies or reverts migrations until the database is at the chosen one.",
            EfOp::Script => "Writes the SQL of a range of migrations to a file.",
            EfOp::Scaffold => "Generates a DbContext and entity classes from an existing database.",
        }
    }

    /// The primary button.
    fn action(self) -> &'static str {
        match self {
            EfOp::AddMigration => "Add Migration",
            EfOp::RemoveMigration => "Remove Migration",
            EfOp::UpdateDatabase => "Update Database",
            EfOp::Script => "Generate Script",
            EfOp::Scaffold => "Scaffold",
        }
    }

    pub fn icon(self) -> Ico {
        match self {
            EfOp::AddMigration => ico::ADD,
            EfOp::RemoveMigration => ico::DELETE,
            EfOp::UpdateDatabase => ico::TW_DATABASE,
            EfOp::Script => ico::FILE_SQL,
            EfOp::Scaffold => ico::DATA_TABLE,
        }
    }

    fn needs_migrations(self) -> bool {
        matches!(self, EfOp::RemoveMigration | EfOp::UpdateDatabase | EfOp::Script)
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Load<T> {
    Idle,
    Loading,
    Ready(T),
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
enum Tool {
    Checking,
    Missing,
    Installing,
    Ready(String),
}

enum Event {
    Tool(Option<String>),
    Contexts(u64, Result<Vec<DbContextInfo>, String>),
    Migrations(u64, Result<Vec<Migration>, String>),
}

struct Proj {
    path: PathBuf,
    name: String,
    info: Option<ProjectInfo>,
}

pub struct EfDialog {
    pub op: EfOp,
    projects: Vec<Proj>,
    project: usize,
    startup: usize,
    tool: Tool,
    contexts: Load<Vec<DbContextInfo>>,
    contexts_key: Option<(usize, usize)>,
    context: Option<String>,
    migrations: Load<Vec<Migration>>,
    migrations_key: Option<(usize, usize, Option<String>)>,
    // Add / remove.
    name: String,
    output_dir: String,
    force: Option<bool>,
    // Update database: `None` = the last migration, `Some("0")` = none.
    update_to: Option<String>,
    connection: String,
    // Script.
    script_from: Option<String>,
    script_to: Option<String>,
    idempotent: bool,
    script_output: String,
    // Scaffold.
    scaffold: Scaffold,
    tables: String,
    schemas: String,
    error: Option<String>,
    focus: bool,
    root: PathBuf,
    dotnet: Option<Dotnet>,
    inbox: Inbox<Event>,
    generation: u64,
    contexts_cancel: CancelToken,
    migrations_cancel: CancelToken,
    /// No background processes (screen captures and UI tests set the lists themselves).
    offline: bool,
}

impl Drop for EfDialog {
    fn drop(&mut self) {
        self.contexts_cancel.cancel();
        self.migrations_cancel.cancel();
    }
}

impl EfDialog {
    /// The dialog for the workspace's projects; `project` preselects the migrations project.
    pub fn new(app: &DotCraftApp, op: EfOp, project: Option<&Path>, waker: Waker) -> Self {
        let projects: Vec<Proj> = app
            .solution_projects()
            .into_iter()
            .map(|path| {
                let info = ProjectInfo::load(&path).ok();
                let name = info.as_ref().map(|i| i.name.clone()).unwrap_or_else(|| path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                Proj { path, name, info }
            })
            .collect();
        let root = app.workspace.as_ref().map(|w| w.ws.root.clone()).unwrap_or_default();
        let remembered = &app.ef_options;
        let find = |p: &Path| projects.iter().position(|x| x.path == p);
        let candidate = |i: usize, strict: bool| projects[i].info.as_ref().is_some_and(|x| !x.is_test && if strict { ef::has_runtime(x) } else { ef::uses_ef(x) });
        let project = project
            .and_then(find)
            .or_else(|| remembered.project.as_deref().and_then(find))
            .or_else(|| (0..projects.len()).find(|&i| candidate(i, true)))
            .or_else(|| (0..projects.len()).find(|&i| candidate(i, false)))
            .unwrap_or(0);
        // The remembered startup project goes with the remembered migrations project.
        let same = remembered.project.as_deref().is_some_and(|r| projects.get(project).is_some_and(|p| p.path == r));
        let startup = same.then(|| remembered.startup.as_deref().and_then(find)).flatten().unwrap_or_else(|| default_startup(&projects, project));
        let provider = projects.get(project).and_then(|p| p.info.as_ref()).and_then(ef::provider_of).unwrap_or(ef::PROVIDERS[0]);
        let script_output = projects.get(project).and_then(|p| p.path.parent()).map(|d| d.join("migrations.sql").display().to_string()).unwrap_or_default();
        let mut d = EfDialog {
            op,
            projects,
            project,
            startup,
            tool: Tool::Checking,
            contexts: Load::Idle,
            contexts_key: None,
            context: remembered.context.clone(),
            migrations: Load::Idle,
            migrations_key: None,
            name: String::new(),
            output_dir: String::new(),
            force: None,
            update_to: None,
            connection: String::new(),
            script_from: None,
            script_to: None,
            idempotent: false,
            script_output,
            scaffold: Scaffold { provider: provider.to_string(), output_dir: "Models".into(), ..Default::default() },
            tables: String::new(),
            schemas: String::new(),
            error: None,
            focus: true,
            root,
            dotnet: Dotnet::locate(Some(&crate::app::dotnet_path(&app.settings))),
            inbox: Inbox::new(waker),
            generation: 0,
            contexts_cancel: CancelToken::new(),
            migrations_cancel: CancelToken::new(),
            offline: false,
        };
        d.check_tool();
        d
    }

    /// Screen captures and tests: no processes; the tool and lists as given.
    #[cfg(test)]
    pub fn set_offline(&mut self, tool: Option<&str>, contexts: Vec<DbContextInfo>, migrations: Vec<Migration>) {
        self.offline = true;
        self.generation += 1;
        self.tool = match tool {
            Some(v) => Tool::Ready(v.to_string()),
            None => Tool::Missing,
        };
        if self.context.is_none() {
            self.context = contexts.first().map(|c| c.id().to_string());
        }
        self.contexts = Load::Ready(contexts);
        self.contexts_key = Some((self.project, self.startup));
        self.migrations = Load::Ready(migrations);
        self.migrations_key = Some((self.project, self.startup, self.context.clone()));
    }

    /// Runs `dotnet ef --version` again (after installing the tool).
    pub fn check_tool(&mut self) {
        if self.offline {
            return;
        }
        let Some(dotnet) = self.dotnet.clone() else {
            self.tool = Tool::Missing;
            return;
        };
        self.tool = Tool::Checking;
        let dir = self.root.clone();
        let out = self.inbox.sender();
        Runtime::spawn(async move {
            out.send(Event::Tool(ef::tool_version(&dotnet, &dir).await));
        });
    }

    fn target(&self) -> Option<EfTarget> {
        let project = self.projects.get(self.project)?.path.clone();
        let startup = self.projects.get(self.startup).map_or_else(|| project.clone(), |p| p.path.clone());
        Some(EfTarget { project, startup, context: self.context.clone() })
    }

    fn migration_list(&self) -> &[Migration] {
        match &self.migrations {
            Load::Ready(m) => m,
            _ => &[],
        }
    }

    /// Starts the listings the current choices need (once per choice).
    fn load(&mut self) {
        if self.offline || !matches!(self.tool, Tool::Ready(_)) || self.op == EfOp::Scaffold {
            return;
        }
        let (Some(dotnet), Some(target)) = (self.dotnet.clone(), self.target()) else { return };
        if self.contexts_key != Some((self.project, self.startup)) {
            self.contexts_key = Some((self.project, self.startup));
            self.migrations_key = None;
            self.migrations = Load::Idle;
            self.contexts = Load::Loading;
            self.contexts_cancel.cancel();
            self.contexts_cancel = CancelToken::new();
            self.generation += 1;
            let (generation, cancel, out) = (self.generation, self.contexts_cancel.clone(), self.inbox.sender());
            Runtime::spawn(async move {
                let r = ef::list_contexts(&dotnet, &target, &cancel).await.map_err(|e| format!("{e:#}"));
                out.send(Event::Contexts(generation, r));
            });
            return;
        }
        let key = Some((self.project, self.startup, self.context.clone()));
        if self.op.needs_migrations() && matches!(self.contexts, Load::Ready(_)) && self.migrations_key != key {
            self.migrations_key = key;
            self.migrations = Load::Loading;
            self.migrations_cancel.cancel();
            self.migrations_cancel = CancelToken::new();
            self.generation += 1;
            let (generation, cancel, out) = (self.generation, self.migrations_cancel.clone(), self.inbox.sender());
            Runtime::spawn(async move {
                let r = ef::list_migrations(&dotnet, &target, &cancel).await.map_err(|e| format!("{e:#}"));
                out.send(Event::Migrations(generation, r));
            });
        }
    }

    fn refresh(&mut self) {
        self.contexts_key = None;
        self.migrations_key = None;
    }

    fn drain(&mut self) {
        for ev in self.inbox.drain() {
            match ev {
                Event::Tool(_) if self.offline => {}
                Event::Tool(v) if self.tool != Tool::Installing || v.is_some() => {
                    self.tool = match v {
                        Some(v) => Tool::Ready(v),
                        None => Tool::Missing,
                    };
                }
                Event::Tool(_) => self.tool = Tool::Missing,
                Event::Contexts(g, r) if g == self.generation => {
                    match &r {
                        Ok(list) if !list.iter().any(|c| Some(c.id()) == self.context.as_deref()) => {
                            self.context = list.first().map(|c| c.id().to_string());
                        }
                        _ => {}
                    }
                    self.contexts = match r {
                        Ok(l) => Load::Ready(l),
                        Err(e) => Load::Failed(e),
                    };
                }
                Event::Migrations(g, r) if g == self.generation => {
                    self.migrations = match r {
                        Ok(l) => Load::Ready(l),
                        Err(e) => Load::Failed(e),
                    };
                }
                _ => {}
            }
        }
    }

    /// The command for the current operation.
    fn command(&self) -> EfCommand {
        let list = self.migration_list();
        match self.op {
            EfOp::AddMigration => EfCommand::AddMigration { name: self.name.clone(), output_dir: Some(self.output_dir.clone()).filter(|d| !d.trim().is_empty()) },
            EfOp::RemoveMigration => EfCommand::RemoveMigration { force: self.force.unwrap_or_else(|| list.last().and_then(|m| m.applied).unwrap_or(false)) },
            EfOp::UpdateDatabase => EfCommand::UpdateDatabase { target: self.update_to.clone(), connection: Some(self.connection.clone()).filter(|c| !c.trim().is_empty()) },
            EfOp::Script => EfCommand::Script { from: self.script_from.clone(), to: self.script_to.clone(), idempotent: self.idempotent, output: PathBuf::from(self.script_output.trim()) },
            EfOp::Scaffold => EfCommand::Scaffold(Scaffold { tables: split_list(&self.tables), schemas: split_list(&self.schemas), ..self.scaffold.clone() }),
        }
    }

    /// Why the operation cannot run yet (translated).
    fn problem(&self) -> Option<String> {
        if self.projects.is_empty() {
            return Some(tr("The workspace has no projects").into());
        }
        match &self.tool {
            Tool::Ready(_) => {}
            Tool::Checking => return Some(tr("Checking dotnet-ef…").into()),
            _ => return Some(tr("Install dotnet-ef to run Entity Framework Core commands").into()),
        }
        if self.op != EfOp::Scaffold {
            match &self.contexts {
                Load::Ready(l) if l.is_empty() => return Some(tr("No DbContext was found in the project").into()),
                Load::Ready(_) => {}
                Load::Failed(_) => return Some(tr("Could not read the DbContexts").into()),
                _ => return Some(tr("Reading DbContexts…").into()),
            }
        }
        if self.op.needs_migrations() {
            match &self.migrations {
                Load::Ready(l) if l.is_empty() && self.op != EfOp::UpdateDatabase => return Some(tr("The project has no migrations").into()),
                Load::Ready(_) => {}
                Load::Failed(_) => return Some(tr("Could not read the migrations").into()),
                _ => return Some(tr("Reading migrations…").into()),
            }
        }
        self.command().problem().map(|p| tr(p).to_string())
    }

    fn remember(&self, app: &mut DotCraftApp) {
        let o = rc_core::session::EfOptions { project: self.projects.get(self.project).map(|p| p.path.clone()), startup: self.projects.get(self.startup).map(|p| p.path.clone()), context: self.context.clone() };
        if app.ef_options != o {
            app.ef_options = o;
            app.session_dirty = true;
        }
    }
}

/// The startup project for a migrations project: an application that references it, else an
/// application using EF Core, else the project itself.
fn default_startup(projects: &[Proj], project: usize) -> usize {
    let Some(target) = projects.get(project) else { return 0 };
    let app = |p: &Proj| p.info.as_ref().is_some_and(|i| i.is_executable() && !i.is_test);
    if target.info.as_ref().is_some_and(|i| i.is_executable()) {
        return project;
    }
    let refs = |p: &Proj| p.info.as_ref().is_some_and(|i| i.project_refs.iter().any(|r| dunce::canonicalize(r).unwrap_or_else(|_| r.clone()) == dunce::canonicalize(&target.path).unwrap_or_else(|_| target.path.clone())));
    projects
        .iter()
        .position(|p| app(p) && refs(p))
        .or_else(|| projects.iter().position(|p| app(p) && p.info.as_ref().is_some_and(ef::uses_ef)))
        .unwrap_or(project)
}

fn split_list(s: &str) -> Vec<String> {
    s.split([',', ';', '\n']).map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect()
}

/// A caption above a drop-down, like [`kit::field`].
fn labelled_dropdown(ui: &mut Ui, label: &str, id: &str, selected: &str, options: &[String], enabled: bool) -> Option<usize> {
    let p = rc_kit::pal(ui);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 3.0;
        ui.label(egui::RichText::new(label).size(12.0).color(p.muted));
        kit::dropdown(ui, id, selected, options, enabled)
    })
    .inner
}

/// A migration row: applied (green check), pending or unknown (no database).
fn migration_row(ui: &mut Ui, m: &Migration, selected: bool) -> egui::Response {
    let (icon, state) = match m.applied {
        Some(true) => (ico::SUCCESS, tr("applied")),
        Some(false) => (ico::TEST_NOT_RUN, tr("pending")),
        None => (ico::TEST_UNKNOWN, tr("database not reachable")),
    };
    let stamp = m.id.split('_').next().unwrap_or_default();
    kit::row(ui, 0, Some(icon), &m.name, &format!("{stamp} · {state}"), selected)
}

/// Shows the dialog; returns false when it closed.
pub fn show(app: &mut DotCraftApp, ctx: &Context, d: &mut EfDialog) -> bool {
    d.drain();
    d.load();
    let mut open = true;
    let busy = matches!(d.tool, Tool::Checking | Tool::Installing) || matches!(d.contexts, Load::Loading) || matches!(d.migrations, Load::Loading);
    if busy {
        ctx.request_repaint_after(std::time::Duration::from_millis(150));
    }
    let p = theme::palette(ctx.global_style().visuals.dark_mode);
    let problem = d.problem();
    let mut run = false;
    let mut install = false;
    let resp = egui::Modal::new(Id::new("ef_core")).show(ctx, |ui| {
        ui.set_width(860.0);
        ui.set_height(600.0);
        ui.label(egui::RichText::new(tr("Entity Framework Core")).font(theme::bold(15.0)));
        ui.add_space(10.0);
        let avail = ui.available_rect_before_wrap();
        let left = Rect::from_min_size(avail.min, vec2(220.0, avail.height() - 44.0));
        let right = Rect::from_min_max(pos2(left.right() + 18.0, avail.top()), pos2(avail.right(), avail.bottom() - 44.0));
        // Operations.
        ui.painter().rect_filled(left, 8.0, p.main_bg);
        let mut lu = ui.new_child(egui::UiBuilder::new().max_rect(left.shrink(4.0)));
        lu.spacing_mut().item_spacing.y = 2.0;
        for op in EfOp::ALL {
            let (r, resp) = lu.allocate_exact_size(vec2(lu.available_width(), 30.0), Sense::click());
            if op == d.op {
                lu.painter().rect_filled(r, 6.0, p.selection);
            } else if resp.hovered() {
                lu.painter().rect_filled(r, 6.0, p.row_hover);
            }
            icons::paint(lu.painter(), Rect::from_center_size(pos2(r.left() + 16.0, r.center().y), vec2(16.0, 16.0)), op.icon(), &p, true);
            lu.painter().text(pos2(r.left() + 32.0, r.center().y), Align2::LEFT_CENTER, tr(op.title()), FontId::proportional(13.0), p.text);
            if resp.clicked() && d.op != op {
                d.op = op;
                d.error = None;
                d.focus = true;
            }
            resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, op == d.op, tr(op.title())));
        }
        // The operation.
        let mut ru = ui.new_child(egui::UiBuilder::new().max_rect(right));
        ru.label(egui::RichText::new(tr(d.op.title())).font(theme::bold(14.0)));
        ru.label(egui::RichText::new(tr(d.op.description())).size(12.5).color(p.muted));
        ru.add_space(10.0);
        if matches!(d.tool, Tool::Missing | Tool::Installing) {
            ru.horizontal(|ui| {
                icons::paint(ui.painter(), Rect::from_center_size(pos2(ui.cursor().left() + 8.0, ui.cursor().top() + 12.0), vec2(16.0, 16.0)), ico::WARNING, &p, true);
                ui.add_space(22.0);
                kit::message(ui, tr("The dotnet-ef tool is not installed."), false);
                let label = if d.tool == Tool::Installing { tr("Installing…") } else { tr("Install dotnet-ef") };
                if kit::button(ui, Some(ico::DOWNLOAD), label, d.tool == Tool::Missing).clicked() {
                    install = true;
                }
            });
            ru.add_space(6.0);
        }
        projects_row(&mut ru, d);
        ru.add_space(12.0);
        egui::ScrollArea::vertical().id_salt("ef_op").max_height(right.bottom() - ru.cursor().top() - 64.0).auto_shrink([false, false]).show(&mut ru, |ui| match d.op {
            EfOp::AddMigration => add_migration(ui, d),
            EfOp::RemoveMigration => remove_migration(ui, d),
            EfOp::UpdateDatabase => update_database(ui, d),
            EfOp::Script => script(ui, d),
            EfOp::Scaffold => scaffold(ui, d),
        });
        // The command line, then the problem.
        let bottom = Rect::from_min_max(pos2(right.left(), right.bottom() - 56.0), right.max);
        let mut bu = ui.new_child(egui::UiBuilder::new().max_rect(bottom));
        if let Some(t) = d.target() {
            // Paths relative to the solution folder, quoted when they contain spaces.
            let shown: Vec<String> = d
                .command()
                .args(&t)
                .into_iter()
                .map(|a| {
                    let rel = Path::new(&a).strip_prefix(&d.root).map(|r| r.display().to_string()).unwrap_or(a);
                    if rel.contains(' ') || rel.is_empty() { format!("\"{rel}\"") } else { rel }
                })
                .collect();
            let mut job = egui::text::LayoutJob::simple_singleline(format!("dotnet {}", shown.join(" ")), FontId::monospace(11.5), p.muted);
            job.wrap.max_width = bottom.width();
            job.wrap.max_rows = 2;
            job.wrap.break_anywhere = true;
            job.wrap.overflow_character = Some('…');
            bu.label(job);
        }
        if let Some(e) = d.error.as_deref().or(problem.as_deref()) {
            bu.label(egui::RichText::new(e).size(12.5).color(if d.error.is_some() { p.error } else { p.muted }));
        }
        // Buttons, with the tool version on the left.
        let bar = Rect::from_min_max(pos2(avail.left(), avail.bottom() - 32.0), avail.max);
        if let Tool::Ready(v) = &d.tool {
            ui.painter().text(pos2(bar.left() + 4.0, bar.center().y), Align2::LEFT_CENTER, format!("dotnet-ef {v}"), FontId::proportional(12.0), p.muted);
        }
        let mut cu = ui.new_child(egui::UiBuilder::new().max_rect(bar).layout(egui::Layout::right_to_left(egui::Align::Center)));
        if cu.add_enabled_ui(problem.is_none(), |ui| theme::primary_button(ui, tr(d.op.action()))).inner.clicked() {
            run = true;
        }
        if theme::secondary_button(&mut cu, tr("Cancel")).clicked() {
            open = false;
        }
        ui.advance_cursor_after_rect(avail);
    });
    if resp.should_close() {
        open = false;
    }
    if install {
        let local = ef::tool_manifest(&d.root).is_some();
        let outs = app.runner.run_tool(tr("Install dotnet-ef").into(), ef::install_args(local), Some(d.root.clone()), ToolFollowup::RecheckEfTool);
        if app.runner.build.as_ref().is_some_and(|b| b.tool == Some(ToolFollowup::RecheckEfTool) && b.handle.is_some()) {
            d.tool = Tool::Installing;
        }
        crate::ui::run_windows::handle_outputs(app, outs);
    }
    if run && problem.is_none() {
        let Some(target) = d.target() else { return open };
        let cmd = d.command();
        let followup = match (&cmd, d.op) {
            (EfCommand::AddMigration { .. }, _) => ToolFollowup::OpenAddedMigration,
            (EfCommand::Script { output, .. }, _) => ToolFollowup::OpenFile(output.clone()),
            _ => ToolFollowup::Nothing,
        };
        d.remember(app);
        let dir = target.project.parent().map(Path::to_path_buf);
        let outs = app.runner.run_tool(tr(d.op.title()).into(), cmd.args(&target), dir, followup);
        let started = app.runner.build.as_ref().is_some_and(|b| b.handle.is_some());
        crate::ui::run_windows::handle_outputs(app, outs);
        if started {
            return false;
        }
    }
    if !open {
        d.remember(app);
    }
    open
}

fn projects_row(ui: &mut Ui, d: &mut EfDialog) {
    let names: Vec<String> = d.projects.iter().map(|p| p.name.clone()).collect();
    let current = |i: usize| names.get(i).cloned().unwrap_or_else(|| tr("No projects").into());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 18.0;
        if let Some(i) = labelled_dropdown(ui, tr("Migrations project"), "ef_project", &current(d.project), &names, !names.is_empty())
            && i != d.project
        {
            d.project = i;
            d.startup = default_startup(&d.projects, i);
            if let Some(prov) = d.projects[i].info.as_ref().and_then(ef::provider_of) {
                d.scaffold.provider = prov.to_string();
            }
            if let Some(dir) = d.projects[i].path.parent() {
                d.script_output = dir.join("migrations.sql").display().to_string();
            }
        }
        if let Some(i) = labelled_dropdown(ui, tr("Startup project"), "ef_startup", &current(d.startup), &names, !names.is_empty()) {
            d.startup = i;
        }
        if d.op != EfOp::Scaffold {
            let contexts: Vec<String> = match &d.contexts {
                Load::Ready(l) => l.iter().map(|c| c.id().to_string()).collect(),
                _ => Vec::new(),
            };
            let shown = match &d.contexts {
                Load::Loading => tr("Loading…").to_string(),
                Load::Ready(l) if l.is_empty() => tr("None found").to_string(),
                Load::Failed(_) => tr("Not available").to_string(),
                _ => d.context.clone().unwrap_or_else(|| tr("Default").into()),
            };
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.label(egui::RichText::new(tr("DbContext")).size(12.0).color(rc_kit::pal(ui).muted));
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    if let Some(i) = kit::dropdown(ui, "ef_context", &shown, &contexts, !contexts.is_empty()) {
                        d.context = contexts.get(i).cloned();
                    }
                    let loading = matches!(d.contexts, Load::Loading) || matches!(d.migrations, Load::Loading);
                    if loading {
                        ui.add(egui::Spinner::new().size(14.0));
                    } else if theme::icon_button(ui, ico::REFRESH, matches!(d.tool, Tool::Ready(_)), tr("Reload DbContexts and migrations")).clicked() {
                        d.refresh();
                    }
                });
            });
        }
    });
    let startup_info = d.projects.get(d.startup).and_then(|p| p.info.as_ref());
    if let Some(info) = startup_info
        && !ef::has_design(info)
    {
        kit::note(ui, &trf("{project} does not reference {package}; design-time commands may fail.", &[("project", &info.name), ("package", &ef::DESIGN_PACKAGE)]));
    }
    if let Load::Failed(e) = &d.contexts {
        kit::message(ui, e, true);
    }
}

fn add_migration(ui: &mut Ui, d: &mut EfDialog) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 18.0;
        let te = kit::field(ui, tr("Migration name"), &mut d.name, "AddOrders", 300.0, true);
        if std::mem::take(&mut d.focus) {
            te.request_focus();
        }
        kit::field(ui, tr("Output folder"), &mut d.output_dir, "Migrations", 220.0, true);
    });
    ui.add_space(10.0);
    if let Load::Ready(list) = &d.migrations
        && let Some(last) = list.last()
    {
        kit::note(ui, &trf("Last migration: {name}", &[("name", &last.name)]));
    }
}

fn migrations_state(ui: &mut Ui, d: &EfDialog) -> bool {
    if !matches!(d.tool, Tool::Ready(_)) {
        return false;
    }
    match &d.migrations {
        Load::Ready(_) => true,
        Load::Failed(e) => {
            kit::message(ui, e, true);
            false
        }
        _ => {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(14.0));
                kit::note(ui, tr("Reading migrations…"));
            });
            false
        }
    }
}

fn remove_migration(ui: &mut Ui, d: &mut EfDialog) {
    if !migrations_state(ui, d) {
        return;
    }
    let list = d.migration_list().to_vec();
    let Some(last) = list.last() else {
        kit::note(ui, tr("The project has no migrations."));
        return;
    };
    kit::note(ui, tr("This migration will be removed:"));
    ui.add_space(4.0);
    migration_row(ui, last, true);
    ui.add_space(8.0);
    let mut force = d.force.unwrap_or(last.applied == Some(true));
    if ui.checkbox(&mut force, tr("Revert it from the database first if it is applied (--force)")).changed() {
        d.force = Some(force);
    }
}

fn update_database(ui: &mut Ui, d: &mut EfDialog) {
    if !migrations_state(ui, d) {
        return;
    }
    let list = d.migration_list().to_vec();
    kit::note(ui, tr("Target migration:"));
    ui.add_space(4.0);
    ui.spacing_mut().item_spacing.y = 0.0;
    let last = list.len().saturating_sub(1);
    if kit::row(ui, 0, Some(ico::UNDO), tr("Initial state"), tr("reverts every migration"), d.update_to.as_deref() == Some("0")).clicked() {
        d.update_to = Some("0".into());
    }
    for (i, m) in list.iter().enumerate() {
        let selected = match &d.update_to {
            None => i == last,
            Some(id) => *id == m.id,
        };
        if migration_row(ui, m, selected).clicked() {
            d.update_to = if i == last { None } else { Some(m.id.clone()) };
        }
    }
    ui.spacing_mut().item_spacing.y = 6.0;
    ui.add_space(10.0);
    kit::field(ui, tr("Connection string (optional)"), &mut d.connection, tr("The one configured in the DbContext"), 520.0, true);
}

fn script(ui: &mut Ui, d: &mut EfDialog) {
    if !migrations_state(ui, d) {
        return;
    }
    let list = d.migration_list().to_vec();
    let name_of = |id: &Option<String>, none: &str| match id {
        None => none.to_string(),
        Some(i) if i == "0" => tr("Initial (empty database)").to_string(),
        Some(i) => list.iter().find(|m| &m.id == i).map_or_else(|| i.clone(), |m| m.name.clone()),
    };
    let mut from_options = vec![tr("Initial (empty database)").to_string()];
    from_options.extend(list.iter().map(|m| m.name.clone()));
    let mut to_options = vec![tr("Last migration").to_string()];
    to_options.extend(list.iter().map(|m| m.name.clone()));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 18.0;
        let from_label = name_of(&d.script_from, tr("Initial (empty database)"));
        if let Some(i) = labelled_dropdown(ui, tr("From"), "ef_script_from", &from_label, &from_options, true) {
            d.script_from = if i == 0 { None } else { Some(list[i - 1].id.clone()) };
        }
        let to_label = name_of(&d.script_to, tr("Last migration"));
        if let Some(i) = labelled_dropdown(ui, tr("To"), "ef_script_to", &to_label, &to_options, true) {
            d.script_to = if i == 0 { None } else { Some(list[i - 1].id.clone()) };
        }
    });
    ui.add_space(8.0);
    ui.checkbox(&mut d.idempotent, tr("Idempotent (checks which migrations the database already has)"));
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        kit::field(ui, tr("Output file"), &mut d.script_output, "migrations.sql", 480.0, true);
        ui.vertical(|ui| {
            ui.add_space(16.0);
            if theme::icon_button(ui, ico::OPEN, true, tr("Choose a file")).clicked() {
                let current = PathBuf::from(d.script_output.trim());
                let mut dialog = rfd::FileDialog::new().add_filter("SQL", &["sql"]);
                if let Some(dir) = current.parent().filter(|p| p.is_dir()) {
                    dialog = dialog.set_directory(dir);
                }
                if let Some(name) = current.file_name() {
                    dialog = dialog.set_file_name(name.to_string_lossy());
                }
                if let Some(f) = dialog.save_file() {
                    d.script_output = f.display().to_string();
                }
            }
        });
    });
}

fn scaffold(ui: &mut Ui, d: &mut EfDialog) {
    let te = kit::field(ui, tr("Connection string"), &mut d.scaffold.connection, "Server=localhost;Database=Shop;Trusted_Connection=True;TrustServerCertificate=True", 560.0, true);
    if std::mem::take(&mut d.focus) {
        te.request_focus();
    }
    ui.add_space(6.0);
    let providers: Vec<String> = ef::PROVIDERS.iter().map(|s| s.to_string()).collect();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 18.0;
        if let Some(i) = labelled_dropdown(ui, tr("Provider"), "ef_provider", &d.scaffold.provider.clone(), &providers, true) {
            d.scaffold.provider = providers[i].clone();
        }
        kit::field(ui, tr("DbContext name"), &mut d.scaffold.context_name, tr("From the database name"), 200.0, true);
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 18.0;
        kit::field(ui, tr("Entities folder"), &mut d.scaffold.output_dir, "Models", 200.0, true);
        kit::field(ui, tr("DbContext folder"), &mut d.scaffold.context_dir, tr("The entities folder"), 200.0, true);
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 18.0;
        kit::field(ui, tr("Tables"), &mut d.tables, tr("All (comma-separated)"), 270.0, true);
        kit::field(ui, tr("Schemas"), &mut d.schemas, tr("All (comma-separated)"), 270.0, true);
    });
    ui.add_space(8.0);
    ui.checkbox(&mut d.scaffold.data_annotations, tr("Use data annotations (attributes) where possible"));
    ui.checkbox(&mut d.scaffold.use_database_names, tr("Use table and column names from the database"));
    ui.checkbox(&mut d.scaffold.no_onconfiguring, tr("Do not write the connection string in OnConfiguring"));
    ui.checkbox(&mut d.scaffold.force, tr("Overwrite existing files"));
}

/// A tool command of the Build window finished.
pub fn tool_finished(app: &mut DotCraftApp, followup: ToolFollowup, success: bool, lines: &[String]) {
    match followup {
        ToolFollowup::RecheckEfTool => {
            if let Some(d) = app.ef.as_mut() {
                d.tool = Tool::Checking;
                d.check_tool();
            }
        }
        _ if !success => {}
        ToolFollowup::OpenAddedMigration => {
            if let Ok(added) = ef::parse_added(&ef::data_json(lines.iter().map(String::as_str)))
                && added.migration_file.is_file()
            {
                app.open_file(&added.migration_file);
            }
        }
        ToolFollowup::OpenFile(path) => {
            if path.is_file() {
                app.open_file(&path);
            }
        }
        ToolFollowup::Nothing => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proj(name: &str, xml: &str) -> Proj {
        let path = PathBuf::from(format!("/s/{name}/{name}.csproj"));
        Proj { info: ProjectInfo::parse(xml, &path).ok(), path, name: name.into() }
    }

    #[test]
    fn the_startup_project_is_the_app_that_references_the_data_project() {
        let projects = vec![
            proj("Tools", r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><OutputType>Exe</OutputType></PropertyGroup></Project>"#),
            proj("Data", r#"<Project Sdk="Microsoft.NET.Sdk"><ItemGroup><PackageReference Include="Microsoft.EntityFrameworkCore.SqlServer" Version="10.0.0" /></ItemGroup></Project>"#),
            proj("Api", r#"<Project Sdk="Microsoft.NET.Sdk.Web"><ItemGroup><ProjectReference Include="../Data/Data.csproj" /></ItemGroup></Project>"#),
        ];
        assert_eq!(default_startup(&projects, 1), 2);
        // An executable project is its own startup project.
        assert_eq!(default_startup(&projects, 0), 0);
        assert_eq!(ef::provider_of(projects[1].info.as_ref().unwrap()), Some("Microsoft.EntityFrameworkCore.SqlServer"));
    }

    #[test]
    fn lists_split_on_commas_and_semicolons() {
        assert_eq!(split_list("dbo.Orders, Customers;  \nProducts,"), ["dbo.Orders", "Customers", "Products"]);
        assert!(split_list(" ").is_empty());
    }

    #[test]
    fn the_dialog_picks_projects_validates_and_previews_the_command() {
        use egui_kittest::kittest::Queryable;
        let _home = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = tempfile::tempdir().unwrap();
        let (_tmp, root) = crate::screens::ef_solution();
        let (contexts, migrations) = crate::screens::ef_samples();
        let mut app = crate::screens::test_app(home.path(), true);
        app.settings.roslyn_path = Some("/nonexistent".into());
        app.open_workspace(&root.join("Shop.slnx"));
        app.ef_dialog(EfOp::AddMigration, None);
        app.ef.as_mut().unwrap().set_offline(Some("10.0.8"), contexts, migrations);
        let mut h = crate::screens::harness(app, egui::vec2(1280.0, 800.0));
        h.run_steps(3);
        // The data project (EF Core) and the API that references it.
        let d = h.state().ef.as_ref().unwrap();
        assert_eq!(d.projects[d.project].name, "Shop.Data");
        assert_eq!(d.projects[d.startup].name, "Shop.Api");
        assert!(h.query_by_label(tr("Enter a migration name")).is_some());
        h.get_by_label(tr("Migration name")).type_text("1st");
        h.run_steps(2);
        assert!(h.query_by_label(tr("The migration name must be a valid C# identifier")).is_some());
        h.state_mut().ef.as_mut().unwrap().name = "AddInvoices".into();
        h.run_steps(2);
        assert!(h.query_by_label_contains("ef migrations add AddInvoices --json --project").is_some());
        assert!(h.query_by_label_contains("--context Shop.Data.ShopContext").is_some());
        // Update Database: the last migration is the default target; picking another one.
        h.get_by_label(tr("Update Database")).click();
        h.run_steps(2);
        h.get_by_label("AddCustomers").click();
        h.run_steps(2);
        assert!(h.query_by_label_contains("ef database update 20261002094512_AddCustomers --project").is_some());
        h.get_by_label(tr("Initial state")).click();
        h.run_steps(2);
        assert!(h.query_by_label_contains("ef database update 0 --project").is_some());
        // Remove: the last migration is pending, so no --force by default.
        h.get_by_label(tr("Remove Last Migration")).click();
        h.run_steps(2);
        assert!(h.query_by_label_contains("ef migrations remove --project").is_some());
        // Closing remembers the projects and the DbContext for the solution.
        h.get_by_label(tr("Cancel")).click();
        h.run_steps(2);
        assert!(h.state().ef.is_none());
        let o = &h.state().ef_options;
        assert_eq!(o.project.as_deref(), Some(root.join("Shop.Data/Shop.Data.csproj").as_path()));
        assert_eq!(o.startup.as_deref(), Some(root.join("Shop.Api/Shop.Api.csproj").as_path()));
        assert_eq!(o.context.as_deref(), Some("Shop.Data.ShopContext"));
        assert_eq!(h.state().session().ef, *o);
    }
}
