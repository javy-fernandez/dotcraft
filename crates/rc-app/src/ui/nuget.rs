//! NuGet tool window for the whole solution (Rider's NuGet window): browse a source, the
//! installed packages, the ones with updates and the ones to consolidate; a package's details
//! and, per project, its installed version with Install / Update / Remove.
//!
//! Projects with Central Package Management (CPM) are edited in their `Directory.Packages.props`
//! instead of through `dotnet add package`: one version for the whole solution, overrides per
//! project, and the issues NuGet reports for it (NU1008, NU1010, unused versions) with fixes.
//! Solutions without CPM can be migrated to it (with a preview of every file), and an audit
//! lists vulnerable and deprecated packages with fixes.

use crate::app::DotCraftApp;
use crate::docs::Docs;
use crate::toolwindow::ToolWindow;
use crate::ui::kit;
use egui::{Align2, FontId, Rect, Sense, Ui, pos2, vec2};
use rc_core::{Inbox, Runtime, Waker};
use rc_dotnet::nuget_auth::{self, SourceStatus};
use rc_dotnet::cpm::{self, CpmCache, CpmIssue, MigrationPlan, MigrationWarning, ProjectCpm, SolutionCpm};
use rc_dotnet::nuget::{AuditPackage, AuditReport, PackageSource, compare_versions};
use rc_dotnet::{NugetClient, PackageSummary};
use rc_kit::i18n::{tr, trf};
use rc_kit::{ico, icons, theme};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tab {
    Browse,
    Installed,
    Updates,
    Consolidate,
    Graph,
    /// Central Package Management issues (only when the solution uses CPM).
    Central,
    /// Vulnerable and deprecated packages (`dotnet list package`, run on demand).
    Audit,
}

enum Event {
    /// Search results, or the error and whether the feed refused the credentials.
    Results(u64, Result<Vec<PackageSummary>, (String, bool)>),
    Versions(String, Vec<String>),
    Latest(String, Option<String>),
    Details(String, Option<PackageSummary>),
    Done(Result<String, String>),
    /// A source was checked: URL, status, a client when OK, whether it came from Sign in.
    Connected(String, SourceStatus, Option<NugetClient>, bool),
    /// A credential provider's message during Sign in (URL, text).
    SignInMessage(String, String),
    /// MSBuild evaluated CPM for these projects.
    Cpm(u64, Vec<ProjectCpm>),
    /// `dotnet list package --vulnerable / --deprecated` finished.
    Audit(u64, Result<AuditReport, String>),
    /// The first version without known vulnerabilities (lowercase id).
    Safe(String, Option<String>),
    Graph(u64, Vec<(PathBuf, Result<rc_dotnet::nuget_assets::AssetsGraph, String>)>),
}

/// Signing in to a private feed through its credential provider (device flow).
struct SignIn {
    name: String,
    url: String,
    messages: Vec<String>,
    error: Option<String>,
    task: Option<tokio::task::JoinHandle<()>>,
}

/// A pending `dotnet` command.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Op {
    /// `dotnet add/remove package`: (project, package, version or `None` to remove).
    Cli(PathBuf, String, Option<String>),
    /// `dotnet restore` after editing CPM files.
    Restore(PathBuf),
}

/// A CPM change, made by editing the files (through their editor when open).
#[derive(Clone, Debug, PartialEq, Eq)]
enum CpmEdit {
    /// The central version in `file` (every project using it follows).
    SetCentral { file: PathBuf, id: String, version: String },
    /// A reference without version, plus the central version when `file` has none.
    Install { project: PathBuf, file: PathBuf, id: String, version: String },
    Remove { project: PathBuf, id: String },
    /// Drops the project's `VersionOverride`.
    Align { project: PathBuf, id: String },
    /// The override becomes the central version.
    MoveOverride { project: PathBuf, file: PathBuf, id: String, version: String },
    SetOverride { project: PathBuf, id: String, version: String },
    /// NU1008: the reference's `Version` moves to `file`.
    MoveVersion { project: PathBuf, file: PathBuf, id: String },
    /// NU1010 fix.
    AddCentral { file: PathBuf, id: String, version: String },
    /// An unused `PackageVersion`.
    DropCentral { file: PathBuf, id: String },
    /// A conditional `PackageVersion`.
    SetConditional { file: PathBuf, id: String, condition: String, version: String },
    /// Pins a transitive package: its `PackageVersion` plus `CentralPackageTransitivePinningEnabled`.
    Pin { file: PathBuf, id: String, version: String },
}

/// A migration to CPM being previewed.
struct Migration {
    /// Folder of the new central file.
    dir: PathBuf,
    /// The files read, with their text when the preview was made.
    files: Vec<(PathBuf, String)>,
    /// Packages whose lower versions stay as `VersionOverride` (lowercase ids).
    keep_lower: BTreeSet<String>,
    plan: Result<MigrationPlan, String>,
    /// The file shown: 0 the central file, then the changed files.
    shown: usize,
}

impl Migration {
    fn replan(&mut self) {
        self.plan = cpm::plan_migration(&self.dir, &self.files, &self.keep_lower).map_err(|e| format!("{e:#}"));
    }
}

/// The audit tab.
#[derive(Default)]
struct Audit {
    /// The running `dotnet list` and when it started.
    running: Option<(tokio::task::JoinHandle<()>, std::time::Instant)>,
    generation: u64,
    report: Option<AuditReport>,
    error: Option<String>,
    /// Selected package (id).
    selected: Option<String>,
    /// First version without known vulnerabilities, by lowercase id (`None`: none found).
    safe: HashMap<String, Option<String>>,
    /// A transitive package to pin, waiting for confirmation.
    confirm_pin: Option<String>,
}

pub struct NugetManager {
    tab: Tab,
    query: String,
    prerelease: bool,
    sources: Vec<PackageSource>,
    source: usize,
    clients: HashMap<String, NugetClient>,
    results: Vec<PackageSummary>,
    searching: bool,
    generation: u64,
    last_query: Option<(String, bool, usize)>,
    edited: std::time::Instant,
    projects: Vec<PathBuf>,
    /// Folder shared by the projects (central files are shown relative to it).
    root: Option<PathBuf>,
    /// Package id → (project, version) for every reference in the solution.
    installed: BTreeMap<String, Vec<(PathBuf, String)>>,
    /// Newest version of installed packages (for Updates), by lowercase id.
    latest: HashMap<String, String>,
    selected: Option<String>,
    details: HashMap<String, PackageSummary>,
    versions: Vec<String>,
    version: String,
    checked: BTreeSet<PathBuf>,
    ops: Vec<Op>,
    busy: Option<String>,
    message: Option<(bool, String)>,
    inbox: Inbox<Event>,
    dotnet: Option<rc_dotnet::Dotnet>,
    loaded: bool,
    /// Where the nuget.config files that give credentials are looked up from.
    config_dir: PathBuf,
    /// Credentials obtained this session (memory only).
    auth: nuget_auth::Session,
    plugins: Option<Vec<nuget_auth::Plugin>>,
    /// Status of each source, by URL.
    status: HashMap<String, SourceStatus>,
    sign_in: Option<SignIn>,
    /// Notifications for the app: (text, error).
    notices: Vec<(String, bool)>,
    /// CPM of every project: cached or static results first, MSBuild's when they arrive.
    cpm: SolutionCpm,
    cpm_cache: CpmCache,
    cpm_generation: u64,
    cpm_evaluating: bool,
    /// CPM changes waiting for the window (they need the open documents).
    edits: Vec<CpmEdit>,
    /// A package no project uses any more: offer to drop its version (central file, id).
    drop_offer: Option<(PathBuf, String)>,
    /// The solution file (the migration and the audit work on it).
    solution: Option<PathBuf>,
    migration: Option<Migration>,
    /// Migrate when the window opens (from the explorer).
    migrate_requested: bool,
    audit: Audit,
    graph: Vec<(PathBuf, Result<rc_dotnet::nuget_assets::AssetsGraph, String>)>,
    graph_loading: bool,
    graph_loaded: bool,
    graph_generation: u64,
    graph_project: usize,
}

fn project_name(p: &Path) -> String {
    p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

impl NugetManager {
    pub fn new(waker: Waker) -> Self {
        NugetManager {
            tab: Tab::Installed,
            query: String::new(),
            prerelease: false,
            sources: Vec::new(),
            source: 0,
            clients: HashMap::new(),
            results: Vec::new(),
            searching: false,
            generation: 0,
            last_query: None,
            edited: std::time::Instant::now(),
            projects: Vec::new(),
            root: None,
            installed: BTreeMap::new(),
            latest: HashMap::new(),
            selected: None,
            details: HashMap::new(),
            versions: Vec::new(),
            version: String::new(),
            checked: BTreeSet::new(),
            ops: Vec::new(),
            busy: None,
            message: None,
            inbox: Inbox::new(waker),
            dotnet: None,
            loaded: false,
            config_dir: PathBuf::new(),
            auth: nuget_auth::Session::default(),
            plugins: None,
            status: HashMap::new(),
            sign_in: None,
            notices: Vec::new(),
            cpm: SolutionCpm::default(),
            cpm_cache: CpmCache::default(),
            cpm_generation: 0,
            cpm_evaluating: false,
            edits: Vec::new(),
            drop_offer: None,
            solution: None,
            migration: None,
            migrate_requested: false,
            audit: Audit::default(),
            graph: Vec::new(),
            graph_loading: false,
            graph_loaded: false,
            graph_generation: 0,
            graph_project: 0,
        }
    }

    /// Opens the migration preview (Migrate to Central Package Management… in the explorer).
    pub fn request_migration(&mut self) {
        self.migrate_requested = true;
    }

    /// Plans the migration of the solution to CPM from the files as the editors see them.
    fn start_migration(&mut self, docs: &Docs) {
        let Some(dir) = self.solution.as_ref().and_then(|s| s.parent()).map(Path::to_path_buf).or_else(|| self.root.clone()) else { return };
        let files: Vec<(PathBuf, String)> = cpm::migration_files(&dir, &self.projects).into_iter().filter_map(|p| current_text(docs, &p).ok().map(|t| (p, t))).collect();
        let mut m = Migration { dir, files, keep_lower: BTreeSet::new(), plan: Err(String::new()), shown: 0 };
        m.replan();
        self.migration = Some(m);
        self.message = None;
    }

    /// Writes the previewed migration: the new central file, then every file through its
    /// editor when open; restores the solution after. Returns the files changed.
    fn apply_migration(&mut self, docs: &mut Docs) -> Vec<PathBuf> {
        let Some(Ok(plan)) = self.migration.as_ref().map(|m| m.plan.clone()) else { return Vec::new() };
        // A file changed since the preview: preview again.
        if let Some(c) = plan.changes.iter().find(|c| current_text(docs, &c.path).ok().as_deref() != Some(c.old.as_str())) {
            let keep = self.migration.as_ref().map(|m| m.keep_lower.clone()).unwrap_or_default();
            self.start_migration(docs);
            if let Some(m) = &mut self.migration {
                m.keep_lower = keep;
                m.replan();
            }
            self.message = Some((true, trf("{file} changed since the preview: review the migration again", &[("file", &self.display_path(&c.path))])));
            return Vec::new();
        }
        if let Err(e) = std::fs::OpenOptions::new().write(true).create_new(true).open(&plan.central_file).and_then(|mut f| std::io::Write::write_all(&mut f, plan.central_text.as_bytes())) {
            self.message = Some((true, format!("{}: {e}", self.display_path(&plan.central_file))));
            return Vec::new();
        }
        self.migration = None;
        let mut touched = vec![plan.central_file.clone()];
        let mut unsaved = Vec::new();
        let mut error = None;
        for c in &plan.changes {
            let r = docs.edit_file(&c.path, |t| if t == c.old { Ok(c.edits.iter().map(|e| (e.range.clone(), e.text.clone())).collect()) } else { Err(tr("The file changed since the preview").to_string()) });
            match r {
                Ok(saved) => {
                    touched.push(c.path.clone());
                    if !saved {
                        unsaved.push(c.path.clone());
                    }
                }
                Err(e) => {
                    error = Some(format!("{}: {e}", self.display_path(&c.path)));
                    break;
                }
            }
        }
        self.message = Some(match (error, unsaved.first()) {
            (Some(e), _) => (true, e),
            (None, Some(f)) => (true, trf("{file} has unsaved changes: save it to apply the change", &[("file", &self.display_path(f))])),
            (None, None) => (false, trf("Migrated {n} files to Central Package Management", &[("n", &plan.changes.len())])),
        });
        if self.message.as_ref().is_some_and(|(err, _)| !err) {
            self.ops.extend(self.restore_targets().into_iter().map(Op::Restore));
        }
        self.cpm_cache.clear();
        self.reload_installed();
        self.tab = Tab::Installed;
        touched
    }

    /// The solution, or every project when there is none.
    fn restore_targets(&self) -> Vec<PathBuf> {
        match &self.solution {
            Some(s) => vec![s.clone()],
            None => self.projects.clone(),
        }
    }

    /// Runs the audit in the background (`dotnet list package`, slow).
    fn start_audit(&mut self) {
        let Some(dotnet) = self.dotnet.clone() else { return };
        let targets = self.restore_targets();
        self.cancel_audit();
        self.audit.error = None;
        let (out, g) = (self.inbox.sender(), self.audit.generation);
        let handle = Runtime::spawn(async move {
            let mut report = AuditReport::default();
            for t in targets {
                match rc_dotnet::nuget::audit(&dotnet, &t).await {
                    Ok(r) => report.merge(r),
                    Err(e) => {
                        out.send(Event::Audit(g, Err(format!("{e:#}"))));
                        return;
                    }
                }
            }
            out.send(Event::Audit(g, Ok(report)));
        });
        self.audit.running = Some((handle, std::time::Instant::now()));
    }

    /// Stops the audit (dropping the task kills `dotnet list`).
    fn cancel_audit(&mut self) {
        if let Some((h, _)) = self.audit.running.take() {
            h.abort();
        }
        self.audit.generation += 1;
    }

    /// The audit's result: look up versions without known vulnerabilities.
    fn set_audit(&mut self, report: AuditReport) {
        let ids: Vec<(String, String)> = report.packages.iter().filter(|p| !p.advisories.is_empty()).map(|p| (p.id.clone(), p.version.clone())).collect();
        if self.audit.selected.as_ref().is_none_or(|s| !report.packages.iter().any(|p| p.id.eq_ignore_ascii_case(s))) {
            self.audit.selected = report.packages.first().map(|p| p.id.clone());
        }
        self.audit.report = Some(report);
        let Some(client) = self.client() else { return };
        for (id, current) in ids {
            if self.audit.safe.contains_key(&id.to_lowercase()) {
                continue;
            }
            let (client, out) = (client.clone(), self.inbox.sender());
            Runtime::spawn(async move {
                let safe = match client.registration(&id).await {
                    Ok(vs) if !vs.is_empty() => rc_dotnet::nuget::first_safe_version(&vs, &current),
                    // No registration (some private feeds): the newest stable version.
                    _ => client.versions(&id).await.ok().and_then(|vs| vs.into_iter().filter(|v| !v.contains('-') && compare_versions(v, &current).is_gt()).max_by(|a, b| compare_versions(a, b))),
                };
                out.send(Event::Safe(id.to_lowercase(), safe));
            });
        }
    }

    /// Feeds an audit without running `dotnet list` (tests, captures).
    #[cfg(test)]
    pub fn set_audit_for_test(&mut self, report: AuditReport, safe: &[(&str, &str)]) {
        for (id, v) in safe {
            self.audit.safe.insert(id.to_lowercase(), Some(v.to_string()));
        }
        self.set_audit(report);
        self.tab = Tab::Audit;
    }

    /// Opens the migration preview on the changed file whose path ends with `file`.
    #[cfg(test)]
    pub fn start_migration_for_test(&mut self, docs: &Docs, file: &str) {
        self.start_migration(docs);
        if let Some(m) = &mut self.migration
            && let Ok(plan) = &m.plan
        {
            m.shown = plan.changes.iter().position(|c| c.path.ends_with(file)).map_or(0, |i| i + 1);
        }
    }

    /// Selects an audited package, optionally asking to pin it.
    #[cfg(test)]
    pub fn select_audit_for_test(&mut self, id: &str, confirm_pin: bool) {
        self.audit.selected = Some(id.to_string());
        self.audit.confirm_pin = confirm_pin.then(|| id.to_string());
    }

    /// The changes that replace a transitive package with `version`: a pin in the central file
    /// (CPM) or a direct reference (other projects).
    fn plan_pin(&self, p: &AuditPackage, version: &str) -> (Vec<CpmEdit>, Vec<Op>) {
        let (mut edits, mut ops) = (Vec::new(), Vec::new());
        for project in &p.transitive {
            match self.cpm.project(project).filter(|c| c.enabled) {
                Some(_) => {
                    if let Some(file) = self.cpm.file_for(project, &p.id) {
                        let e = CpmEdit::Pin { file, id: p.id.clone(), version: version.to_string() };
                        if !edits.contains(&e) {
                            edits.push(e);
                        }
                    }
                }
                None => ops.push(Op::Cli(project.clone(), p.id.clone(), Some(version.to_string()))),
            }
        }
        (edits, ops)
    }

    /// The solution's projects (re-read when the window opens or after a change).
    pub fn set_projects(&mut self, projects: Vec<PathBuf>, dotnet: Option<rc_dotnet::Dotnet>) {
        self.graph.clear();
        self.graph_loading = false;
        self.graph_loaded = false;
        self.graph_generation += 1;
        self.graph_project = 0;
        if let Some(dir) = projects.first().and_then(|p| p.parent()) {
            let root = common_root(&projects).unwrap_or_else(|| dir.to_path_buf());
            self.sources = rc_dotnet::nuget::package_sources(&root).into_iter().filter(|s| s.enabled).collect();
            self.source = self.source.min(self.sources.len().saturating_sub(1));
            self.config_dir = root.clone();
            self.root = Some(root);
        }
        self.projects = projects;
        self.dotnet = dotnet;
        for i in 0..self.sources.len() {
            self.connect(i);
        }
        self.reload_installed();
        self.loaded = true;
    }

    /// Opens on one project (Manage NuGet Packages… in the explorer).
    pub fn focus_project(&mut self, project: &Path) {
        self.checked = std::iter::once(project.to_path_buf()).collect();
        self.tab = Tab::Installed;
    }

    /// The source chosen in the window (the editor's package completion uses it too).
    pub fn selected_source(&self) -> Option<&PackageSource> {
        self.sources.get(self.source)
    }

    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// Shows `id` (from the editor): installed packages, or the search when none has it.
    pub fn open_package(&mut self, id: &str) {
        self.tab = if self.installed_versions(id).is_empty() { Tab::Browse } else { Tab::Installed };
        self.query = id.to_string();
        self.select(id);
    }

    fn reload_installed(&mut self) {
        // CPM: cached or quick static results now, MSBuild in the background for the projects
        // under a Directory.Packages.props (the others never pay for it).
        let mut list = Vec::new();
        let mut evaluate = Vec::new();
        for p in &self.projects {
            match self.cpm_cache.get(p) {
                Some(c) => list.push(c.clone()),
                None => {
                    let s = cpm::detect_static(p);
                    if s.central_file.is_some() || s.enabled {
                        evaluate.push(p.clone());
                    }
                    list.push(s);
                }
            }
        }
        self.cpm = SolutionCpm::new(list);
        self.cpm_generation += 1;
        self.cpm_evaluating = false;
        if let (Some(dotnet), false) = (self.dotnet.clone(), evaluate.is_empty()) {
            self.cpm_evaluating = true;
            let (out, g) = (self.inbox.sender(), self.cpm_generation);
            Runtime::spawn(async move {
                let list = cpm::detect_all(&dotnet, evaluate, 4).await.into_iter().filter_map(|(_, r)| r.ok()).collect();
                out.send(Event::Cpm(g, list));
            });
        }
        if self.tab == Tab::Central && !self.cpm.in_use() {
            self.tab = Tab::Installed;
        }
        self.rebuild_installed();
    }

    fn load_graph(&mut self) {
        if self.graph_loading {
            return;
        }
        self.graph_loading = true;
        self.graph_generation += 1;
        let generation = self.graph_generation;
        let projects = self.projects.clone();
        let out = self.inbox.sender();
        Runtime::spawn(async move {
            if let Ok(graphs) = tokio::task::spawn_blocking(move || {
                projects.into_iter().map(|project| {
                    let result = rc_dotnet::nuget_assets::load(&project).map_err(|e| format!("{e:#}"));
                    (project, result)
                }).collect()
            }).await {
                out.send(Event::Graph(generation, graphs));
            }
        });
    }

    fn rebuild_installed(&mut self) {
        let mut installed: BTreeMap<String, Vec<(PathBuf, String)>> = BTreeMap::new();
        for p in &self.projects {
            match self.cpm.project(p).filter(|c| c.enabled) {
                Some(c) => {
                    for id in c.references.iter().map(|r| &r.id).chain(c.globals.iter().map(|g| &g.id)) {
                        let list = installed.entry(id.clone()).or_default();
                        if !list.iter().any(|(q, _)| q == p) {
                            list.push((p.clone(), c.effective_version(id).unwrap_or_default()));
                        }
                    }
                }
                None => {
                    for r in rc_dotnet::nuget::installed_packages(p) {
                        installed.entry(r.name.clone()).or_default().push((p.clone(), r.version));
                    }
                }
            }
        }
        self.installed = installed;
        // Newest versions, for the Updates tab.
        let ids: Vec<String> = self.installed.keys().filter(|id| !self.latest.contains_key(&id.to_lowercase())).cloned().collect();
        for id in ids {
            self.fetch_latest(id);
        }
    }

    fn client(&mut self) -> Option<NugetClient> {
        let src = self.sources.get(self.source)?.clone();
        if let Some(c) = self.clients.get(&src.url) {
            return Some(c.clone());
        }
        self.connect(self.source);
        None
    }

    fn plugins(&mut self) -> Vec<nuget_auth::Plugin> {
        let dotnet = self.dotnet.as_ref().map(|d| d.path.clone());
        self.plugins.get_or_insert_with(|| nuget_auth::discover_plugins(dotnet.as_deref())).clone()
    }

    /// Checks a source in the background (credentials from nuget.config, the environment or a
    /// credential provider, silently); once per source until Refresh.
    fn connect(&mut self, i: usize) {
        let Some(src) = self.sources.get(i).cloned() else { return };
        if self.status.contains_key(&src.url) {
            return;
        }
        self.status.insert(src.url.clone(), SourceStatus::Checking);
        let (dir, session, plugins, out) = (self.config_dir.clone(), self.auth.clone(), self.plugins(), self.inbox.sender());
        Runtime::spawn(async move {
            let feed = nuget_auth::Feed::new(&src.name, &src.url, &dir);
            let c = nuget_auth::connect(&feed, &session, &plugins).await;
            out.send(Event::Connected(src.url, c.status, c.client, false));
        });
    }

    fn current_status(&self) -> Option<&SourceStatus> {
        self.status.get(&self.sources.get(self.source)?.url)
    }

    /// Runs the credential providers interactively for a source (the Sign in dialog).
    fn start_sign_in(&mut self, name: String, url: String) {
        if let Some(t) = self.sign_in.take().and_then(|d| d.task) {
            t.abort();
        }
        let (dir, session, plugins, out) = (self.config_dir.clone(), self.auth.clone(), self.plugins(), self.inbox.sender());
        let (n, u) = (name.clone(), url.clone());
        let task = Runtime::spawn(async move {
            let feed = nuget_auth::Feed::new(&n, &u, &dir);
            let (o2, u2) = (out.clone(), u.clone());
            let log = move |m: String| {
                o2.send(Event::SignInMessage(u2.clone(), m));
            };
            let c = nuget_auth::sign_in(&feed, &session, &plugins, &log).await;
            out.send(Event::Connected(u, c.status, c.client, true));
        });
        self.sign_in = Some(SignIn { name, url, messages: Vec::new(), error: None, task: Some(task) });
    }

    fn sign_in_current(&mut self) {
        if let Some(src) = self.sources.get(self.source).cloned() {
            self.start_sign_in(src.name, src.url);
        }
    }

    fn cancel_sign_in(&mut self) {
        if let Some(t) = self.sign_in.take().and_then(|d| d.task) {
            // Dropping the work kills the provider.
            t.abort();
        }
    }

    fn fetch_latest(&mut self, id: String) {
        let Some(client) = self.client() else { return };
        let (out, pre) = (self.inbox.sender(), self.prerelease);
        Runtime::spawn(async move {
            let v = client.versions(&id).await.ok().and_then(|vs| vs.into_iter().filter(|v| pre || !v.contains('-')).max_by(|a, b| compare_versions(a, b)));
            out.send(Event::Latest(id, v));
        });
    }

    fn search(&mut self) {
        let Some(client) = self.client() else { return };
        self.generation += 1;
        let g = self.generation;
        self.searching = true;
        self.last_query = Some((self.query.clone(), self.prerelease, self.source));
        let (q, pre, out) = (self.query.clone(), self.prerelease, self.inbox.sender());
        Runtime::spawn(async move {
            let r = client.search(&q, pre, 0, 50).await.map(|r| r.packages).map_err(|e| (format!("{e:#}"), nuget_auth::is_auth_error(&e)));
            out.send(Event::Results(g, r));
        });
    }

    fn select(&mut self, id: &str) {
        self.selected = Some(id.to_string());
        self.versions.clear();
        self.version.clear();
        let Some(client) = self.client() else { return };
        let out = self.inbox.sender();
        let id2 = id.to_string();
        let c2 = client.clone();
        Runtime::spawn(async move {
            let v = c2.versions(&id2).await.unwrap_or_default();
            out.send(Event::Versions(id2, v));
        });
        if !self.details.contains_key(&id.to_lowercase()) && !self.results.iter().any(|r| r.id.eq_ignore_ascii_case(id)) {
            let out = self.inbox.sender();
            let id = id.to_string();
            Runtime::spawn(async move {
                let d = client.search(&format!("packageid:{id}"), true, 0, 1).await.ok().and_then(|r| r.packages.into_iter().next());
                out.send(Event::Details(id, d));
            });
        }
    }

    /// Runs the queued commands one after the other.
    fn run_next(&mut self) {
        let (Some(dotnet), false) = (self.dotnet.clone(), self.ops.is_empty()) else { return };
        let op = self.ops.remove(0);
        self.busy = Some(match &op {
            Op::Cli(project, id, Some(v)) => trf("Installing {id} {version} in {project}…", &[("id", id), ("version", v), ("project", &project_name(project))]),
            Op::Cli(project, id, None) => trf("Removing {id} from {project}…", &[("id", id), ("project", &project_name(project))]),
            Op::Restore(project) => trf("Restoring {project}…", &[("project", &project_name(project))]),
        });
        let (out, pre) = (self.inbox.sender(), self.prerelease);
        Runtime::spawn(async move {
            let (r, done) = match &op {
                Op::Cli(project, id, Some(v)) => (rc_dotnet::nuget::add_package(&dotnet, project, id, Some(v), pre).await, trf("{id}: done", &[("id", id)])),
                Op::Cli(project, id, None) => (rc_dotnet::nuget::remove_package(&dotnet, project, id).await, trf("{id}: done", &[("id", id)])),
                Op::Restore(project) => (rc_dotnet::nuget::restore(&dotnet, project).await, trf("{project} restored", &[("project", &project_name(project))])),
            };
            out.send(Event::Done(match r {
                Ok(o) if o.success => Ok(done),
                Ok(o) => Err(o.output.lines().rev().find(|l| l.to_lowercase().contains("error")).unwrap_or(o.output.trim()).trim().to_string()),
                Err(e) => Err(format!("{e:#}")),
            }));
        });
    }

    #[cfg(test)]
    pub fn select_for_test(&mut self, id: &str) {
        self.select(id);
    }

    /// Sources with a known status (for captures).
    #[cfg(test)]
    pub fn set_sources_for_test(&mut self, sources: Vec<(PackageSource, SourceStatus)>, current: usize) {
        self.status = sources.iter().map(|(s, st)| (s.url.clone(), st.clone())).collect();
        self.sources = sources.into_iter().map(|(s, _)| s).collect();
        self.source = current;
        self.tab = Tab::Browse;
        self.last_query = Some((self.query.clone(), self.prerelease, self.source));
    }

    /// The Sign in dialog with provider messages (for captures).
    #[cfg(test)]
    pub fn sign_in_for_test(&mut self, name: &str, url: &str, messages: &[&str], error: Option<&str>) {
        self.sign_in = Some(SignIn { name: name.into(), url: url.into(), messages: messages.iter().map(|m| m.to_string()).collect(), error: error.map(str::to_string), task: error.is_none().then(|| Runtime::spawn(std::future::pending::<()>())) });
    }

    /// Versions and newest versions arrived (for captures).
    #[cfg(test)]
    pub fn ready_for_test(&self) -> bool {
        !self.versions.is_empty() && self.installed.keys().all(|id| self.latest.contains_key(&id.to_lowercase()))
    }

    /// MSBuild evaluated CPM (for tests and captures).
    #[cfg(test)]
    pub fn cpm_ready_for_test(&self) -> bool {
        self.loaded && !self.cpm_evaluating && self.busy.is_none() && self.ops.is_empty()
    }

    #[cfg(test)]
    pub fn show_central_for_test(&mut self) {
        self.tab = Tab::Central;
    }

    fn installed_versions(&self, id: &str) -> Vec<(PathBuf, String)> {
        self.installed.iter().find(|(k, _)| k.eq_ignore_ascii_case(id)).map(|(_, v)| v.clone()).unwrap_or_default()
    }

    fn has_update(&self, id: &str) -> Option<String> {
        let latest = self.latest.get(&id.to_lowercase())?;
        let newest_installed = self.installed_versions(id).into_iter().map(|(_, v)| v).max_by(|a, b| compare_versions(a, b))?;
        (compare_versions(latest, &newest_installed).is_gt()).then(|| latest.clone())
    }

    /// Some project takes the version of `id` from a central file.
    fn is_central(&self, id: &str) -> bool {
        self.cpm.users(id).iter().any(|p| p.is_central(id))
    }

    /// The central versions of `id`: (file, version, condition, global), nearest files first.
    fn central_versions(&self, id: &str) -> Vec<(PathBuf, String, Option<String>, bool)> {
        let users = self.cpm.users(id);
        let mut files: Vec<PathBuf> = Vec::new();
        for u in &users {
            for f in self.cpm.chain(&u.project) {
                if !files.contains(&f.path) {
                    files.push(f.path.clone());
                }
            }
        }
        if users.is_empty() {
            files = self.cpm.files.keys().cloned().collect();
        }
        files
            .iter()
            .filter_map(|f| self.cpm.files.get(f))
            .flat_map(|f| f.items.iter().filter(|i| i.id.eq_ignore_ascii_case(id)).map(|i| (f.path.clone(), i.resolved.clone(), i.condition.clone(), i.global)))
            .collect()
    }

    /// A central file relative to the solution folder.
    fn display_path(&self, p: &Path) -> String {
        relative(self.root.as_deref(), p)
    }

    fn rows(&self) -> Vec<(String, String, String)> {
        // (id, version shown, description)
        match self.tab {
            Tab::Browse => self.results.iter().map(|r| (r.id.clone(), r.version.clone(), r.description.lines().next().unwrap_or("").to_string())).collect(),
            Tab::Central | Tab::Audit | Tab::Graph => Vec::new(),
            Tab::Installed | Tab::Updates | Tab::Consolidate => self
                .installed
                .iter()
                .filter(|(id, _)| self.query.trim().is_empty() || id.to_lowercase().contains(&self.query.trim().to_lowercase()))
                .filter_map(|(id, list)| {
                    let mut vs: Vec<String> = list.iter().map(|(_, v)| v.clone()).collect();
                    vs.sort_by(|a, b| compare_versions(a, b));
                    vs.dedup();
                    let shown = vs.join(", ");
                    match self.tab {
                        Tab::Updates => self.has_update(id).map(|l| (id.clone(), format!("{shown} → {l}"), String::new())),
                        Tab::Consolidate => (vs.len() > 1).then(|| (id.clone(), shown, String::new())),
                        _ => Some((id.clone(), shown, if list.len() == 1 { tr("1 project").to_string() } else { trf("{n} projects", &[("n", &list.len())]) })),
                    }
                })
                .collect(),
        }
    }

    /// CPM issues as list rows: (issue, package, title, detail, fix label, fix).
    fn issue_rows(&self) -> Vec<(String, String, String, String, Option<CpmEdit>)> {
        self.cpm
            .issues()
            .into_iter()
            .map(|i| match i {
                CpmIssue::UnusedVersion { file, id } => {
                    let detail = tr("No project references it").to_string();
                    (id.clone(), tr("Unused").to_string(), detail, tr("Remove version").to_string(), Some(CpmEdit::DropCentral { file, id }))
                }
                CpmIssue::VersionOnReference { project, id, version } => {
                    let detail = trf("{project} sets Version {version} on its reference", &[("project", &project_name(&project)), ("version", &version)]);
                    let fix = self.cpm.file_for(&project, &id).map(|file| CpmEdit::MoveVersion { project, file, id: id.clone() });
                    (id, "NU1008".to_string(), detail, tr("Move version to central").to_string(), fix)
                }
                CpmIssue::MissingVersion { project, id } => {
                    let detail = trf("{project} references it without a central version", &[("project", &project_name(&project))]);
                    let version = self.latest.get(&id.to_lowercase()).cloned();
                    let fix = self.cpm.file_for(&project, &id).zip(version).map(|(file, version)| CpmEdit::AddCentral { file, id: id.clone(), version });
                    (id, "NU1010".to_string(), detail, tr("Add central version").to_string(), fix)
                }
            })
            .collect()
    }

    /// The changes that install or update `id` at `version` in `projects`: CPM edits (one per
    /// central file) and `dotnet add` commands for the other projects.
    fn plan_install(&self, projects: &[PathBuf], id: &str, version: &str) -> (Vec<CpmEdit>, Vec<Op>) {
        let (mut edits, mut ops) = (Vec::new(), Vec::new());
        for project in projects {
            let Some(c) = self.cpm.project(project).filter(|c| c.enabled) else {
                ops.push(Op::Cli(project.clone(), id.to_string(), Some(version.to_string())));
                continue;
            };
            let Some(file) = self.cpm.file_for(project, id) else { continue };
            let e = match c.reference(id) {
                None if c.global(id).is_some() => continue,
                None => CpmEdit::Install { project: project.clone(), file, id: id.to_string(), version: version.to_string() },
                Some(r) if r.version_override.is_some() => CpmEdit::SetOverride { project: project.clone(), id: id.to_string(), version: version.to_string() },
                Some(r) if r.version.is_some() => {
                    ops.push(Op::Cli(project.clone(), id.to_string(), Some(version.to_string())));
                    continue;
                }
                Some(_) => CpmEdit::SetCentral { file, id: id.to_string(), version: version.to_string() },
            };
            if !edits.contains(&e) {
                edits.push(e);
            }
        }
        (edits, ops)
    }

    /// Applies the queued CPM edits; returns the files changed (for the explorer).
    fn apply_edits(&mut self, docs: &mut Docs) -> Vec<PathBuf> {
        let mut touched = Vec::new();
        let mut restore: Vec<PathBuf> = Vec::new();
        for e in std::mem::take(&mut self.edits) {
            // Projects that read the central file being changed.
            let readers = |file: &Path| -> Vec<PathBuf> { self.cpm.projects.iter().filter(|p| p.enabled && self.cpm.chain(&p.project).iter().any(|f| f.path == file)).map(|p| p.project.clone()).collect() };
            let (affected, files): (Vec<PathBuf>, Vec<PathBuf>) = match &e {
                CpmEdit::SetCentral { file, .. } | CpmEdit::AddCentral { file, .. } | CpmEdit::DropCentral { file, .. } => (readers(file), vec![file.clone()]),
                CpmEdit::Install { project, file, .. } | CpmEdit::MoveVersion { project, file, .. } => (vec![project.clone()], vec![file.clone(), project.clone()]),
                CpmEdit::MoveOverride { project, file, .. } => (readers(file), vec![file.clone(), project.clone()]),
                CpmEdit::Remove { project, .. } | CpmEdit::Align { project, .. } | CpmEdit::SetOverride { project, .. } => (vec![project.clone()], vec![project.clone()]),
                CpmEdit::SetConditional { file, .. } | CpmEdit::Pin { file, .. } => (readers(file), vec![file.clone()]),
            };
            let drop_file = match &e {
                CpmEdit::Remove { project, id } => self.cpm.file_for(project, id).map(|f| (f, id.clone(), project.clone())),
                _ => None,
            };
            match apply_cpm_edit(docs, &e) {
                Ok((msg, unsaved)) => {
                    touched.extend(files.iter().cloned());
                    self.message = Some(match unsaved.first() {
                        Some(f) => (true, trf("{file} has unsaved changes: save it to apply the change", &[("file", &self.display_path(f))])),
                        None => {
                            restore.extend(affected);
                            (false, msg)
                        }
                    });
                    // Nothing uses the package any more: offer to drop its version.
                    if let Some((file, id, project)) = drop_file {
                        if let Some(p) = self.cpm.projects.iter_mut().find(|p| p.project == project) {
                            p.references.retain(|r| !r.id.eq_ignore_ascii_case(&id));
                        }
                        if !self.cpm.is_used(&file, &id) && self.cpm.files.get(&file).is_some_and(|f| f.versions(&id).next().is_some()) {
                            self.drop_offer = Some((file, id));
                        }
                    }
                }
                Err(err) => self.message = Some((true, err)),
            }
        }
        if !touched.is_empty() {
            for p in &self.projects {
                self.cpm_cache.invalidate(p);
            }
            self.reload_installed();
        }
        for p in restore {
            let op = Op::Restore(p);
            if !self.ops.contains(&op) {
                self.ops.push(op);
            }
        }
        touched
    }
}

/// Edits one file through [`Docs::edit_file`]; the paths of files left with unsaved changes.
fn edit_one(docs: &mut Docs, path: &Path, f: impl FnOnce(&str) -> anyhow::Result<Vec<cpm::TextEdit>>, unsaved: &mut Vec<PathBuf>) -> Result<(), String> {
    let saved = docs.edit_file(path, |text| f(text).map(|v| v.into_iter().map(|e| (e.range, e.text)).collect()).map_err(|e| format!("{}: {e:#}", path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned()))))?;
    if !saved {
        unsaved.push(path.to_path_buf());
    }
    Ok(())
}

/// Applies a CPM change; (message, files left with unsaved changes).
fn apply_cpm_edit(docs: &mut Docs, e: &CpmEdit) -> Result<(String, Vec<PathBuf>), String> {
    let mut unsaved = Vec::new();
    let file_name = |p: &Path| p.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let msg = match e {
        CpmEdit::SetCentral { file, id, version } => {
            edit_one(docs, file, |t| cpm::set_central_version(t, id, None, version), &mut unsaved)?;
            trf("Central version of {id} set to {version}", &[("id", id), ("version", version)])
        }
        CpmEdit::Install { project, file, id, version } => {
            // Keep an existing central version: the project follows it.
            edit_one(docs, file, |t| if cpm::CentralFile::parse(file, t)?.unconditional(id).is_some() { Ok(Vec::new()) } else { cpm::add_central_version(t, id, version) }, &mut unsaved)?;
            edit_one(docs, project, |t| cpm::add_reference(t, id), &mut unsaved)?;
            trf("Installed {id} in {project}", &[("id", id), ("project", &project_name(project))])
        }
        CpmEdit::Remove { project, id } => {
            edit_one(docs, project, |t| cpm::remove_reference(t, id), &mut unsaved)?;
            trf("Removed {id} from {project}", &[("id", id), ("project", &project_name(project))])
        }
        CpmEdit::Align { project, id } => {
            edit_one(docs, project, |t| cpm::remove_version_override(t, id), &mut unsaved)?;
            trf("{project} now uses the central version of {id}", &[("id", id), ("project", &project_name(project))])
        }
        CpmEdit::MoveOverride { project, file, id, version } => {
            edit_one(docs, file, |t| cpm::set_central_version(t, id, None, version), &mut unsaved)?;
            edit_one(docs, project, |t| cpm::remove_version_override(t, id), &mut unsaved)?;
            trf("Central version of {id} set to {version}", &[("id", id), ("version", version)])
        }
        CpmEdit::SetOverride { project, id, version } => {
            edit_one(docs, project, |t| cpm::set_reference_metadata(t, id, "VersionOverride", version), &mut unsaved)?;
            trf("Installed {id} {version} in {project}", &[("id", id), ("version", version), ("project", &project_name(project))])
        }
        CpmEdit::MoveVersion { project, file, id } => {
            let mut moved = String::new();
            edit_one(
                docs,
                project,
                |t| {
                    let (edits, v) = cpm::remove_reference_version(t, id)?;
                    moved = v;
                    Ok(edits)
                },
                &mut unsaved,
            )?;
            edit_one(docs, file, |t| if cpm::CentralFile::parse(file, t)?.unconditional(id).is_some() { Ok(Vec::new()) } else { cpm::add_central_version(t, id, &moved) }, &mut unsaved)?;
            trf("Moved the version of {id} to {file}", &[("id", id), ("file", &file_name(file))])
        }
        CpmEdit::AddCentral { file, id, version } => {
            edit_one(docs, file, |t| cpm::add_central_version(t, id, version), &mut unsaved)?;
            trf("Added {id} {version} to {file}", &[("id", id), ("version", version), ("file", &file_name(file))])
        }
        CpmEdit::DropCentral { file, id } => {
            edit_one(docs, file, |t| cpm::remove_central_version(t, id), &mut unsaved)?;
            trf("Removed {id} from {file}", &[("id", id), ("file", &file_name(file))])
        }
        CpmEdit::SetConditional { file, id, condition, version } => {
            edit_one(docs, file, |t| cpm::set_central_version(t, id, Some(condition), version), &mut unsaved)?;
            trf("Central version of {id} set to {version}", &[("id", id), ("version", version)])
        }
        CpmEdit::Pin { file, id, version } => {
            edit_one(
                docs,
                file,
                |t| {
                    let mut edits = match cpm::CentralFile::parse(file, t)?.unconditional(id) {
                        Some(_) => cpm::set_central_version(t, id, None, version)?,
                        None => cpm::add_central_version(t, id, version)?,
                    };
                    edits.extend(cpm::set_central_property(t, "CentralPackageTransitivePinningEnabled", "true")?);
                    Ok(edits)
                },
                &mut unsaved,
            )?;
            trf("Pinned {id} to {version}", &[("id", id), ("version", version)])
        }
    };
    Ok((msg, unsaved))
}

/// The text of `path` as its editor has it, or from disk.
fn current_text(docs: &Docs, path: &Path) -> Result<String, String> {
    let canon = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    match docs.list.iter().find(|d| d.path == canon || d.path == path) {
        Some(d) => Ok(d.editor.buffer.rope().to_string()),
        None => crate::docs::read_text(path).map(|(t, _, _)| t),
    }
}

/// A unified diff of two texts: (`' '`, `'+'` or `'-'`, line), three lines of context around
/// each change and `None` between distant changes.
fn unified_diff(old: &str, new: &str) -> Vec<Option<(char, String)>> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let changes = rc_vcs::textdiff::diff(&a, &b);
    let mut out = Vec::new();
    let mut shown = 0usize; // old lines already shown
    for (k, c) in changes.iter().enumerate() {
        let from = c.old.start.saturating_sub(3).max(shown);
        if from > shown {
            out.push(None);
        }
        out.extend(a[from..c.old.start].iter().map(|l| Some((' ', l.to_string()))));
        out.extend(a[c.old.clone()].iter().map(|l| Some(('-', l.to_string()))));
        out.extend(b[c.new.clone()].iter().map(|l| Some(('+', l.to_string()))));
        let next = changes.get(k + 1).map_or(a.len(), |n| n.old.start);
        let to = (c.old.end + 3).min(next).min(a.len());
        out.extend(a[c.old.end..to].iter().map(|l| Some((' ', l.to_string()))));
        shown = to;
    }
    if shown < a.len() && !changes.is_empty() {
        out.push(None);
    }
    out
}

/// `p` relative to the solution folder `root`.
fn relative(root: Option<&Path>, p: &Path) -> String {
    let base = root.and_then(|r| r.ancestors().find(|a| p.starts_with(a) && a.parent().is_some()));
    base.and_then(|b| p.strip_prefix(b).ok()).unwrap_or(p).display().to_string()
}

fn common_root(paths: &[PathBuf]) -> Option<PathBuf> {
    let mut root = paths.first()?.parent()?.to_path_buf();
    for p in paths {
        while !p.starts_with(&root) {
            root = root.parent()?.to_path_buf();
        }
    }
    Some(root)
}

/// Takes finished work: applies CPM edits, reloads the explorer when references changed and
/// shows notifications.
fn pump(app: &mut DotCraftApp) {
    let mut changed = poll(&mut app.nuget);
    if !app.nuget.edits.is_empty() {
        for f in app.nuget.apply_edits(&mut app.docs) {
            app.explorer.invalidate(&f);
        }
        changed = true;
    }
    if changed {
        for p in app.nuget.projects.clone() {
            app.explorer.invalidate(&p);
        }
    }
    for (text, error) in std::mem::take(&mut app.nuget.notices) {
        app.notify(text, error);
    }
}

/// The NuGet tool window.
pub fn window(app: &mut DotCraftApp, ui: &mut Ui) {
    if !app.nuget.loaded {
        let projects = app.solution_projects();
        let dotnet = rc_dotnet::Dotnet::locate(Some(&crate::app::dotnet_path(&app.settings)));
        app.nuget.solution = app.workspace.as_ref().and_then(|w| w.ws.solution()).map(Path::to_path_buf);
        app.nuget.set_projects(projects, dotnet);
    }
    if std::mem::take(&mut app.nuget.migrate_requested) && !app.nuget.cpm.in_use() {
        app.nuget.start_migration(&app.docs);
    }
    pump(app);
    let m = &mut app.nuget;
    if m.busy.is_some() || m.searching || m.cpm_evaluating || m.audit.running.is_some() || m.graph_loading {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(150));
    }
    // Search as you type in Browse (debounced); empty shows the most downloaded.
    if m.tab == Tab::Browse && m.last_query.as_ref() != Some(&(m.query.clone(), m.prerelease, m.source)) && m.edited.elapsed().as_millis() > 300 {
        m.search();
    }
    let p = rc_kit::pal(ui);
    // Header: title, source, prerelease, refresh, hide.
    let header = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), crate::ui::toolwindows::HEADER_H));
    ui.allocate_rect(header, Sense::hover());
    ui.painter().text(pos2(header.left() + 12.0, header.center().y), Align2::LEFT_CENTER, "NuGet", theme::bold(13.0), p.text);
    let mut hu = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(pos2(header.left() + 70.0, header.top()), pos2(header.right() - 40.0, header.bottom()))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    hu.label(egui::RichText::new(tr("Source")).size(12.5).color(p.muted));
    if let Some(i) = source_picker(&mut hu, m)
        && i != m.source
    {
        m.source = i;
        m.latest.clear();
        m.reload_installed();
    }
    match m.current_status().cloned() {
        Some(SourceStatus::NeedsSignIn(_)) => {
            if crate::ui::kit::button(&mut hu, Some(ico::USER), tr("Sign in"), m.sign_in.is_none()).clicked() {
                m.sign_in_current();
            }
        }
        Some(SourceStatus::Failed(why)) => {
            icons::image(&mut hu, ico::ERROR, 16.0).on_hover_text(why);
        }
        _ => {}
    }
    if hu.checkbox(&mut m.prerelease, tr("Prerelease")).changed() {
        m.latest.clear();
        m.reload_installed();
    }
    if theme::icon_button(&mut hu, ico::REFRESH, true, tr("Refresh")).clicked() {
        m.latest.clear();
        m.last_query = None;
        // Check again the sources that failed or need a sign-in.
        m.status.retain(|_, s| *s == SourceStatus::Ok);
        m.plugins = None;
        for i in 0..m.sources.len() {
            m.connect(i);
        }
        m.cpm_cache.clear();
        m.reload_installed();
        if m.tab == Tab::Graph { m.load_graph(); }
    }
    if m.cpm_evaluating {
        hu.add(egui::Spinner::new().size(12.0));
        kit::note(&mut hu, tr("Reading Central Package Management…"));
    }
    let hide = Rect::from_min_size(pos2(header.right() - 34.0, header.center().y - 13.0), vec2(26.0, 26.0));
    let mut hb = ui.new_child(egui::UiBuilder::new().max_rect(hide));
    if theme::icon_button(&mut hb, ico::HIDE, true, tr("Hide")).clicked() {
        app.toggle_tool(ToolWindow::Nuget);
        return;
    }
    let m = &mut app.nuget;
    // Tabs with counts.
    let updates = m.installed.keys().filter(|id| m.has_update(id).is_some()).count();
    let consolidate = m.installed.values().filter(|l| l.iter().map(|(_, v)| v).collect::<BTreeSet<_>>().len() > 1).count();
    let issues = m.cpm.issues().len();
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        let mut tabs = vec![
            (Tab::Browse, tr("Browse").to_string()),
            (Tab::Installed, trf("Installed ({n})", &[("n", &m.installed.len())])),
            (Tab::Updates, trf("Updates ({n})", &[("n", &updates)])),
            (Tab::Consolidate, trf("Consolidate ({n})", &[("n", &consolidate)])),
            (Tab::Graph, tr("Dependency graph").to_string()),
        ];
        if m.cpm.in_use() {
            tabs.push((Tab::Central, trf("Central issues ({n})", &[("n", &issues)])));
        }
        tabs.push((
            Tab::Audit,
            match &m.audit.report {
                Some(r) => trf("Audit ({n})", &[("n", &r.packages.len())]),
                None => tr("Audit").to_string(),
            },
        ));
        for (tab, label) in tabs {
            if theme::tab(ui, m.tab == tab && m.migration.is_none(), None, &label, false).0.clicked() {
                m.tab = tab;
                m.migration = None;
                if tab == Tab::Graph && !m.graph_loaded { m.load_graph(); }
            }
        }
        // Solutions without CPM: move every version to a Directory.Packages.props.
        if !m.cpm.in_use() && !m.cpm_evaluating && !m.projects.is_empty() && m.migration.is_none() {
            ui.add_space(8.0);
            if kit::button(ui, Some(ico::FILE_XML), tr("Migrate to Central Package Management…"), m.busy.is_none()).clicked() {
                m.migrate_requested = true;
                ui.ctx().request_repaint();
            }
        }
    });
    ui.add_space(4.0);
    let avail = ui.available_rect_before_wrap();
    if m.tab == Tab::Graph {
        graph_view(ui, m, avail);
        ui.advance_cursor_after_rect(avail);
        return;
    }
    if m.migration.is_some() {
        let action = migration_view(ui, m, avail);
        ui.advance_cursor_after_rect(avail);
        match action {
            Some(true) => {
                for f in app.nuget.apply_migration(&mut app.docs) {
                    app.explorer.invalidate(&f);
                }
                for p in app.nuget.projects.clone() {
                    app.explorer.invalidate(&p);
                }
            }
            Some(false) => app.nuget.migration = None,
            None => {}
        }
        let m = &mut app.nuget;
        if m.busy.is_none() && !m.ops.is_empty() {
            m.run_next();
        }
        return;
    }
    let left = Rect::from_min_size(avail.min, vec2((avail.width() * 0.42).clamp(300.0, 520.0), avail.height()));
    let right = Rect::from_min_max(pos2(left.right() + 1.0, avail.top()), avail.max);
    ui.painter().vline(left.right() + 0.5, avail.y_range(), egui::Stroke::new(1.0, p.island_border));
    // Left: search and list.
    let mut lu = ui.new_child(egui::UiBuilder::new().max_rect(left.shrink2(vec2(8.0, 0.0))));
    if m.tab == Tab::Audit {
        audit_list(&mut lu, m);
    } else {
        let hint = if m.tab == Tab::Browse { tr("Search packages") } else { tr("Filter installed packages") };
        if lu.add(egui::TextEdit::singleline(&mut m.query).hint_text(hint).desired_width(f32::INFINITY)).changed() {
            m.edited = std::time::Instant::now();
        }
        lu.add_space(4.0);
        let mut pick = None;
        if m.tab == Tab::Central {
            issue_list(&mut lu, m, &mut pick);
        } else {
            let mut sign_in = false;
            package_list(&mut lu, m, &mut pick, &mut sign_in);
            if sign_in {
                m.sign_in_current();
            }
        }
        if let Some(id) = pick {
            m.select(&id);
        }
    }
    // Right: details and projects.
    let mut ru = ui.new_child(egui::UiBuilder::new().max_rect(right.shrink2(vec2(12.0, 0.0))));
    let open = if m.tab == Tab::Audit {
        audit_details(&mut ru, m);
        None
    } else {
        details(&mut ru, m)
    };
    ui.advance_cursor_after_rect(avail);
    if m.busy.is_none() && !m.ops.is_empty() {
        m.run_next();
    }
    if let Some(f) = open {
        app.open_file(&f);
    }
}

fn graph_view(ui: &mut Ui, m: &mut NugetManager, rect: Rect) {
    let mut body = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(12.0, 4.0))));
    body.horizontal(|ui| {
        ui.label(egui::RichText::new(tr("Resolved dependencies")).font(theme::bold(13.0)));
        if theme::secondary_button(ui, tr("Reload graph")).clicked() {
            m.graph_loading = false;
            m.load_graph();
        }
    });
    if m.graph_loading {
        body.spinner();
        return;
    }
    if !m.graph_loaded {
        body.label(tr("Restore the project to view its dependencies"));
        return;
    }
    if m.graph.is_empty() {
        body.label(tr("No projects in the workspace"));
        return;
    }
    m.graph_project = m.graph_project.min(m.graph.len() - 1);
    egui::ComboBox::from_id_salt("nuget_graph_project")
        .selected_text(project_name(&m.graph[m.graph_project].0))
        .show_ui(&mut body, |ui| {
            for (i, (project, _)) in m.graph.iter().enumerate() {
                ui.selectable_value(&mut m.graph_project, i, project_name(project));
            }
        });
    match &m.graph[m.graph_project].1 {
        Err(error) => { body.label(error); }
        Ok(graph) => {
            body.label(format!("{} · {}", graph.framework, trf("{packages} packages, {edges} dependencies", &[("packages", &graph.packages.len()), ("edges", &graph.edges.len())])));
            let mut children = vec![Vec::new(); graph.packages.len()];
            for edge in &graph.edges { children[edge.from].push(&graph.packages[edge.to]); }
            egui::ScrollArea::vertical().id_salt("nuget_graph_packages").show(&mut body, |ui| {
                for (index, package) in graph.packages.iter().enumerate() {
                    let kind = if package.direct { tr("Direct") } else { tr("Transitive") };
                    let label = format!("{} {} ({kind})", package.id, package.version);
                    if children[index].is_empty() {
                        ui.label(label);
                    } else {
                        ui.collapsing(label, |ui| {
                            for child in &children[index] {
                                ui.label(format!("{} {}", child.id, child.version));
                            }
                        });
                    }
                }
            });
        }
    }
}

fn package_list(ui: &mut Ui, m: &NugetManager, pick: &mut Option<String>, sign_in_clicked: &mut bool) {
    let p = rc_kit::pal(ui);
    let rows = m.rows();
    egui::ScrollArea::vertical().id_salt("nuget_rows").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let source = m.sources.get(m.source).map(|s| s.name.clone()).unwrap_or_default();
        if rows.is_empty() && m.tab == Tab::Browse && !m.searching && m.current_status().is_some_and(|s| *s != SourceStatus::Ok) {
            ui.add_space(16.0);
            match m.current_status() {
                Some(SourceStatus::NeedsSignIn(_)) => {
                    crate::ui::kit::note(ui, &trf("Sign in to {source} to browse its packages", &[("source", &source)]));
                    ui.add_space(6.0);
                    *sign_in_clicked = crate::ui::kit::button(ui, Some(ico::USER), tr("Sign in"), m.sign_in.is_none()).clicked();
                }
                Some(SourceStatus::Failed(why)) => {
                    crate::ui::kit::message(ui, &trf("Could not connect to {source}", &[("source", &source)]), true);
                    crate::ui::kit::note(ui, why);
                }
                _ => crate::ui::kit::note(ui, &trf("Connecting to {source}…", &[("source", &source)])),
            }
        } else if rows.is_empty() {
            ui.add_space(16.0);
            let msg = match m.tab {
                Tab::Browse if m.searching => tr("Searching…"),
                Tab::Browse => tr("No packages found"),
                Tab::Installed | Tab::Central | Tab::Audit | Tab::Graph => tr("The solution has no package references"),
                Tab::Updates => tr("Every package is up to date"),
                Tab::Consolidate => tr("Every project uses the same versions"),
            };
            ui.label(egui::RichText::new(msg).color(p.muted));
        }
        for (id, version, desc) in &rows {
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click());
            if m.selected.as_deref() == Some(id.as_str()) {
                ui.painter().rect_filled(r, 6.0, p.selection);
            } else if resp.hovered() {
                ui.painter().rect_filled(r, 6.0, p.row_hover);
            }
            icons::paint(ui.painter(), Rect::from_center_size(pos2(r.left() + 16.0, r.top() + 14.0), vec2(16.0, 16.0)), ico::PACKAGE, &p, true);
            let t = ui.painter().text(pos2(r.left() + 32.0, r.top() + 14.0), Align2::LEFT_CENTER, id, theme::bold(13.0), p.text);
            if m.tab != Tab::Browse && m.is_central(id) {
                let mut x = t.right() + 8.0;
                // Global references and conditional versions are told apart.
                let global = m.cpm.users(id).iter().any(|u| u.global(id).is_some());
                let badge = if global { tr("Global") } else { tr("Central") };
                x = theme::paint_badge(ui, pos2(x, r.top() + 14.0), badge).right() + 6.0;
                if m.central_versions(id).iter().any(|(_, _, c, _)| c.is_some()) {
                    theme::paint_badge(ui, pos2(x, r.top() + 14.0), tr("Conditional"));
                }
            }
            let installed = !m.installed_versions(id).is_empty();
            ui.painter().text(pos2(r.right() - 8.0, r.top() + 14.0), Align2::RIGHT_CENTER, version, FontId::proportional(12.0), if installed { p.ok } else { p.muted });
            let mut job = egui::text::LayoutJob::simple_singleline(desc.clone(), FontId::proportional(12.0), p.muted);
            job.wrap.max_width = r.width() - 40.0;
            job.wrap.max_rows = 1;
            job.wrap.overflow_character = Some('…');
            let g = ui.painter().layout_job(job);
            ui.painter().galley(pos2(r.left() + 32.0, r.top() + 25.0), g, p.muted);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, id));
            if resp.clicked() {
                *pick = Some(id.clone());
            }
        }
    });
}

/// The Central issues tab: one row per issue with its fix.
fn issue_list(ui: &mut Ui, m: &mut NugetManager, pick: &mut Option<String>) {
    let p = rc_kit::pal(ui);
    let rows = m.issue_rows();
    let mut fixes = Vec::new();
    egui::ScrollArea::vertical().id_salt("nuget_issues").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        if rows.is_empty() {
            ui.add_space(16.0);
            ui.label(egui::RichText::new(tr("No Central Package Management issues")).color(p.muted));
        }
        for (i, (id, kind, detail, fix_label, fix)) in rows.iter().enumerate() {
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click());
            if m.selected.as_deref() == Some(id.as_str()) {
                ui.painter().rect_filled(r, 6.0, p.selection);
            } else if resp.hovered() {
                ui.painter().rect_filled(r, 6.0, p.row_hover);
            }
            icons::paint(ui.painter(), Rect::from_center_size(pos2(r.left() + 16.0, r.top() + 14.0), vec2(16.0, 16.0)), ico::WARNING, &p, true);
            let t = ui.painter().text(pos2(r.left() + 32.0, r.top() + 14.0), Align2::LEFT_CENTER, id, theme::bold(13.0), p.text);
            theme::paint_badge(ui, pos2(t.right() + 8.0, r.top() + 14.0), kind);
            let fix_w = ui.painter().layout_no_wrap(fix_label.clone(), FontId::proportional(12.5), p.text).size().x + 16.0;
            let mut job = egui::text::LayoutJob::simple_singleline(detail.clone(), FontId::proportional(12.0), p.muted);
            job.wrap.max_width = (r.width() - 48.0 - fix_w).max(40.0);
            job.wrap.max_rows = 1;
            job.wrap.overflow_character = Some('…');
            let g = ui.painter().layout_job(job);
            ui.painter().galley(pos2(r.left() + 32.0, r.top() + 25.0), g, p.muted);
            let br = Rect::from_min_size(pos2(r.right() - fix_w - 4.0, r.center().y - 13.0), vec2(fix_w, 26.0));
            let mut bu = ui.new_child(egui::UiBuilder::new().max_rect(br).id_salt(("nuget_fix", i)));
            if kit::button(&mut bu, None, fix_label, fix.is_some() && m.busy.is_none()).clicked()
                && let Some(f) = fix
            {
                fixes.push(f.clone());
            } else if resp.clicked() {
                *pick = Some(id.clone());
            }
        }
    });
    m.edits.extend(fixes);
}

/// A project row of the details: check box, name, version and badges, actions on the right.
/// Returns the clicked action label index.
fn project_row(ui: &mut Ui, checked: &mut bool, name: &str, version: Option<&str>, badges: &[&str], actions: &[(&str, bool)]) -> Option<usize> {
    let p = rc_kit::pal(ui);
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
    if resp.hovered() {
        ui.painter().rect_filled(r, 4.0, p.row_hover);
    }
    let mut cu = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(pos2(r.left() + 4.0, r.top() + 4.0), vec2(22.0, 20.0))).id_salt(("nuget_check", name)));
    cu.checkbox(checked, "").widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, *checked, name));
    icons::paint(ui.painter(), Rect::from_center_size(pos2(r.left() + 38.0, r.center().y), vec2(16.0, 16.0)), ico::PROJECT, &p, true);
    let name_w = (r.width() * 0.34).max(110.0);
    let mut job = egui::text::LayoutJob::simple_singleline(name.to_string(), FontId::proportional(13.0), p.text);
    job.wrap.max_width = name_w;
    job.wrap.max_rows = 1;
    job.wrap.overflow_character = Some('…');
    let g = ui.painter().layout_job(job);
    ui.painter().galley(pos2(r.left() + 52.0, r.center().y - g.size().y / 2.0), g, p.text);
    let mut x = r.left() + 60.0 + name_w;
    let t = ui.painter().text(pos2(x, r.center().y), Align2::LEFT_CENTER, version.unwrap_or("—"), FontId::proportional(12.5), if version.is_some() { p.ok } else { p.muted });
    x = t.right() + 8.0;
    for b in badges {
        x = theme::paint_badge(ui, pos2(x, r.center().y), b).right() + 6.0;
    }
    let mut clicked = None;
    let mut au = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(pos2(x, r.top() + 1.0), pos2(r.right() - 2.0, r.bottom() - 1.0))).layout(egui::Layout::right_to_left(egui::Align::Center)).id_salt(("nuget_actions", name)));
    for (i, (label, enabled)) in actions.iter().enumerate().rev() {
        if kit::button(&mut au, None, label, *enabled).clicked() && *enabled {
            clicked = Some(i);
        }
    }
    clicked
}

/// The package's details; returns a file to open (a central file).
fn details(ui: &mut Ui, m: &mut NugetManager) -> Option<PathBuf> {
    let p = rc_kit::pal(ui);
    let mut open = None;
    let Some(id) = m.selected.clone() else {
        ui.add_space(20.0);
        ui.label(egui::RichText::new(tr("Select a package to see its details and the projects that use it")).color(p.muted));
        drop_offer(ui, m);
        if let Some((err, msg)) = &m.message {
            ui.label(egui::RichText::new(msg).size(12.5).color(if *err { p.error } else { p.ok }));
        }
        return None;
    };
    egui::ScrollArea::vertical().id_salt("nuget_details").auto_shrink([false, false]).show(ui, |ui| {
        let central = m.is_central(&id);
        ui.horizontal(|ui| {
            icons::image(ui, ico::PACKAGE, 16.0);
            let t = ui.label(egui::RichText::new(&id).font(theme::bold(15.0)));
            if central {
                let (r, _) = ui.allocate_exact_size(vec2(70.0, 18.0), Sense::hover());
                theme::paint_badge(ui, pos2(r.left() + 4.0, t.rect.center().y), tr("Central"));
            }
        });
        if let Some(d) = m.details.get(&id.to_lowercase()) {
            if !d.authors.is_empty() {
                ui.label(egui::RichText::new(trf("by {authors}", &[("authors", &d.authors.join(", "))])).size(12.0).color(p.muted));
            }
            ui.add_space(4.0);
            ui.label(egui::RichText::new(&d.description).size(12.5));
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(trf("{n} downloads", &[("n", &d.total_downloads)])).size(12.0).color(p.muted));
                if let Some(url) = &d.project_url {
                    ui.hyperlink_to(egui::RichText::new(tr("Project site")).size(12.0), url);
                }
                ui.hyperlink_to(egui::RichText::new("nuget.org").size(12.0), format!("https://www.nuget.org/packages/{id}"));
            });
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tr("Version")).size(12.5).color(p.muted));
            let shown = if m.version.is_empty() { tr("Loading…").to_string() } else { m.version.clone() };
            let options: Vec<String> = m.versions.iter().take(80).cloned().collect();
            if let Some(i) = kit::dropdown(ui, "nuget_version", &shown, &options, !options.is_empty()) {
                m.version = options[i].clone();
            }
        });
        let installed = m.installed_versions(&id);
        let mut ops: Vec<Op> = Vec::new();
        let mut edits: Vec<CpmEdit> = Vec::new();
        let busy = m.busy.is_some();
        // Central versions: where they live, update once for every project.
        let centrals = m.central_versions(&id);
        if !centrals.is_empty() {
            ui.add_space(10.0);
            ui.label(egui::RichText::new(tr("Central version")).font(theme::bold(12.5)));
            for (file, version, condition, global) in &centrals {
                ui.horizontal(|ui| {
                    if kit::button(ui, Some(ico::FILE_XML), &m.display_path(file), true).on_hover_text(tr("Open the central file")).clicked() {
                        open = Some(file.clone());
                    }
                    ui.label(egui::RichText::new(version).size(12.5).color(p.ok));
                    if *global {
                        let (r, _) = ui.allocate_exact_size(vec2(64.0, 18.0), Sense::hover());
                        theme::paint_badge(ui, pos2(r.left(), r.center().y), tr("Global"));
                    }
                    match condition {
                        // A conditional version is only updated on its own.
                        Some(c) => {
                            kit::note(ui, &trf("when {condition}", &[("condition", c)]));
                            if !m.version.is_empty() && *version != m.version && kit::button(ui, Some(ico::UP), &trf("Update to {version}", &[("version", &m.version)]), !busy).on_hover_text(tr("Updates only this conditional version")).clicked() {
                                edits.push(CpmEdit::SetConditional { file: file.clone(), id: id.clone(), condition: c.clone(), version: m.version.clone() });
                            }
                        }
                        None if !m.version.is_empty() && *version != m.version && kit::button(ui, Some(ico::UP), &trf("Update to {version}", &[("version", &m.version)]), !busy).clicked() => {
                            edits.push(CpmEdit::SetCentral { file: file.clone(), id: id.clone(), version: m.version.clone() });
                        }
                        None => {}
                    }
                });
            }
            let users: Vec<String> = m.cpm.users(&id).iter().map(|u| project_name(&u.project)).collect();
            if !users.is_empty() {
                kit::note(ui, &trf("Used by {projects}", &[("projects", &users.join(", "))]));
            }
            let overrides: Vec<PathBuf> = m.cpm.users(&id).iter().filter(|u| u.reference(&id).is_some_and(|r| r.version_override.is_some())).map(|u| u.project.clone()).collect();
            if overrides.len() > 1 && kit::button(ui, None, &trf("Align all to central version ({n})", &[("n", &overrides.len())]), !busy).clicked() {
                edits.extend(overrides.into_iter().map(|project| CpmEdit::Align { project, id: id.clone() }));
            }
        }
        ui.add_space(10.0);
        ui.label(egui::RichText::new(tr("Projects")).font(theme::bold(12.5)));
        ui.spacing_mut().item_spacing.y = 2.0;
        for project in m.projects.clone() {
            let current = installed.iter().find(|(p, _)| *p == project).map(|(_, v)| v.clone());
            let c = m.cpm.project(&project).filter(|c| c.enabled).cloned();
            let r = c.as_ref().and_then(|c| c.reference(&id)).cloned();
            let global = c.as_ref().is_some_and(|c| c.global(&id).is_some());
            let inherited = r.as_ref().is_some_and(|r| r.defined_in != project);
            let mut badges: Vec<&str> = Vec::new();
            if global {
                badges.push(tr("Global"));
            } else if r.as_ref().is_some_and(|r| r.version_override.is_some()) {
                badges.push(tr("Override"));
            } else if c.as_ref().is_some_and(|c| c.is_central(&id)) {
                badges.push(tr("Central"));
            }
            // Actions: (label, enabled).
            let ready = !busy && !m.version.is_empty();
            let mut actions: Vec<(&str, bool)> = Vec::new();
            match (&current, &c) {
                (None, _) => actions.push((tr("Install"), ready)),
                (Some(_), Some(_)) if global || inherited => {}
                (Some(_), Some(_)) => actions.push((tr("Remove"), !busy)),
                (Some(v), None) => {
                    if *v != m.version {
                        actions.push((tr("Update"), ready));
                    }
                    actions.push((tr("Remove"), !busy));
                }
            }
            let mut on = m.checked.contains(&project);
            let name = project_name(&project);
            let clicked = project_row(ui, &mut on, &name, current.as_deref(), &badges, &actions);
            if on {
                m.checked.insert(project.clone());
            } else {
                m.checked.remove(&project);
            }
            match clicked.map(|i| actions[i].0) {
                Some(a) if a == tr("Remove") => match &c {
                    Some(_) => edits.push(CpmEdit::Remove { project: project.clone(), id: id.clone() }),
                    None => ops.push(Op::Cli(project.clone(), id.clone(), None)),
                },
                Some(_) => {
                    let (e, o) = m.plan_install(std::slice::from_ref(&project), &id, &m.version);
                    edits.extend(e);
                    ops.extend(o);
                }
                None => {}
            }
            // CPM details under the row: overrides, versions on the reference, inherited ones.
            let (Some(c), Some(r)) = (&c, &r) else { continue };
            let file = m.cpm.file_for(&project, &id);
            let central_version = c.central_version(&id).map(|v| v.version.clone());
            if !inherited && r.version_override.is_none() && r.version.is_none() {
                continue;
            }
            // A grey or amber note under the name, then its fixes.
            let (note, warn) = if inherited {
                (trf("Referenced in {file}", &[("file", &m.display_path(&r.defined_in))]), false)
            } else if let Some(o) = &r.version_override {
                (trf("VersionOverride {version} (central {central})", &[("version", o), ("central", &central_version.as_deref().unwrap_or("—"))]), false)
            } else {
                (trf("NU1008: Version {version} on the reference", &[("version", &r.version.clone().unwrap_or_default())]), true)
            };
            ui.horizontal(|ui| {
                ui.add_space(52.0);
                if warn { kit::message(ui, &note, false) } else { kit::note(ui, &note) }
            });
            if inherited {
                continue;
            }
            ui.horizontal(|ui| {
                ui.add_space(44.0);
                if let Some(o) = &r.version_override {
                    if kit::button(ui, None, tr("Align to central version"), !busy && central_version.is_some()).clicked() {
                        edits.push(CpmEdit::Align { project: project.clone(), id: id.clone() });
                    }
                    if let Some(file) = &file
                        && kit::button(ui, None, tr("Move override to central"), !busy).clicked()
                    {
                        edits.push(CpmEdit::MoveOverride { project: project.clone(), file: file.clone(), id: id.clone(), version: o.clone() });
                    }
                } else if let Some(file) = &file
                    && kit::button(ui, None, tr("Move version to central"), !busy).clicked()
                {
                    edits.push(CpmEdit::MoveVersion { project: project.clone(), file: file.clone(), id: id.clone() });
                }
            });
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let ready = !busy && !m.version.is_empty() && !m.checked.is_empty();
            if ui.add_enabled_ui(ready, |ui| theme::primary_button(ui, &trf("Install or update in {n} selected", &[("n", &m.checked.len())]))).inner.clicked() {
                let checked: Vec<PathBuf> = m.checked.iter().cloned().collect();
                let (e, o) = m.plan_install(&checked, &id, &m.version);
                edits.extend(e);
                ops.extend(o);
            }
            let removable: Vec<PathBuf> = m
                .checked
                .iter()
                .filter(|c| installed.iter().any(|(p, _)| p == *c))
                .filter(|c| m.cpm.project(c).filter(|x| x.enabled).is_none_or(|x| x.reference(&id).is_some_and(|r| r.defined_in == **c)))
                .cloned()
                .collect();
            if ui.add_enabled_ui(!busy && !removable.is_empty(), |ui| theme::secondary_button(ui, tr("Remove from selected"))).inner.clicked() {
                for project in removable {
                    if m.cpm.project(&project).is_some_and(|c| c.enabled) {
                        edits.push(CpmEdit::Remove { project, id: id.clone() });
                    } else {
                        ops.push(Op::Cli(project, id.clone(), None));
                    }
                }
            }
        });
        drop_offer(ui, m);
        if let Some(b) = &m.busy {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(14.0));
                ui.label(egui::RichText::new(b).size(12.5).color(p.muted));
            });
        } else if let Some((err, msg)) = &m.message {
            ui.label(egui::RichText::new(msg).size(12.5).color(if *err { p.error } else { p.ok }));
        }
        m.ops.extend(ops);
        m.edits.extend(edits);
    });
    open
}

/// "Nothing uses X any more: remove its PackageVersion?"
fn drop_offer(ui: &mut Ui, m: &mut NugetManager) {
    let Some((file, id)) = m.drop_offer.clone() else { return };
    ui.add_space(8.0);
    kit::message(ui, &trf("No project uses {id} any more. Remove its PackageVersion from {file}?", &[("id", &id), ("file", &m.display_path(&file))]), false);
    ui.horizontal(|ui| {
        if kit::button(ui, Some(ico::DELETE), tr("Remove version"), m.busy.is_none()).clicked() {
            m.edits.push(CpmEdit::DropCentral { file: file.clone(), id: id.clone() });
            m.drop_offer = None;
        }
        if kit::button(ui, None, tr("Keep"), true).clicked() {
            m.drop_offer = None;
        }
    });
}

/// A migration warning as text.
fn warning_text(root: Option<&Path>, w: &MigrationWarning) -> String {
    match w {
        MigrationWarning::Conditional { file, id, condition } => trf("{file}: {id} is conditional ({condition}); kept with its condition", &[("file", &relative(root, file)), ("id", id), ("condition", condition)]),
        MigrationWarning::Floating { file, id, version } => trf("{file}: {id} {version} is a floating version or a range; kept as written", &[("file", &relative(root, file)), ("id", id), ("version", version)]),
        MigrationWarning::Property { file, id, version } => trf("{file}: {id} uses {version}, which that file does not define; kept as VersionOverride", &[("file", &relative(root, file)), ("id", id), ("version", version)]),
        MigrationWarning::NoVersion { file, id } => trf("{file}: {id} has no version; add a PackageVersion for it after migrating", &[("file", &relative(root, file)), ("id", id)]),
        MigrationWarning::Outside { file } => trf("{file} is outside the solution folder; left unchanged", &[("file", &relative(root, file))]),
    }
}

/// The migration preview: files, conflicts and warnings on the left, the shown file's diff on
/// the right, Apply / Cancel below. `Some(true)` applies, `Some(false)` cancels.
fn migration_view(ui: &mut Ui, m: &mut NugetManager, avail: Rect) -> Option<bool> {
    let p = rc_kit::pal(ui);
    let left = Rect::from_min_size(avail.min, vec2((avail.width() * 0.42).clamp(300.0, 520.0), avail.height()));
    let right = Rect::from_min_max(pos2(left.right() + 1.0, avail.top()), avail.max);
    ui.painter().vline(left.right() + 0.5, avail.y_range(), egui::Stroke::new(1.0, p.island_border));
    let root = m.root.clone();
    let busy = m.busy.is_some();
    let message = m.message.clone();
    let mig = m.migration.as_mut()?;
    let mut action = None;
    let mut lu = ui.new_child(egui::UiBuilder::new().max_rect(left.shrink2(vec2(8.0, 0.0))));
    lu.label(egui::RichText::new(tr("Migrate to Central Package Management")).font(theme::bold(13.0)));
    lu.add_space(2.0);
    let list_h = (lu.available_height() - 64.0).max(80.0);
    egui::ScrollArea::vertical().id_salt("nuget_migration").max_height(list_h).auto_shrink([false, false]).show(&mut lu, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        let plan = match &mig.plan {
            Ok(plan) => plan.clone(),
            Err(e) => {
                kit::message(ui, e, true);
                return;
            }
        };
        kit::note(ui, &trf("Creates {file} with {n} package versions and removes the versions from {m} files.", &[("file", &relative(root.as_deref(), &plan.central_file)), ("n", &plan.versions.len()), ("m", &plan.changes.len())]));
        ui.add_space(6.0);
        ui.label(egui::RichText::new(tr("Files")).font(theme::bold(12.5)));
        if kit::row(ui, 0, Some(ico::FILE_XML), &relative(root.as_deref(), &plan.central_file), tr("new file"), mig.shown == 0).clicked() {
            mig.shown = 0;
        }
        for (i, c) in plan.changes.iter().enumerate() {
            let lines = unified_diff(&c.old, &c.new);
            let (add, del) = (lines.iter().flatten().filter(|(k, _)| *k == '+').count(), lines.iter().flatten().filter(|(k, _)| *k == '-').count());
            let icon = if c.path.extension().is_some_and(|e| e.to_string_lossy().ends_with("proj")) { ico::PROJECT } else { ico::FILE_XML };
            let name = c.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if kit::row(ui, 0, Some(icon), &name, &format!("+{add} −{del}"), mig.shown == i + 1).on_hover_text(relative(root.as_deref(), &c.path)).clicked() {
                mig.shown = i + 1;
            }
        }
        if !plan.conflicts.is_empty() {
            ui.add_space(8.0);
            ui.label(egui::RichText::new(trf("Version conflicts ({n})", &[("n", &plan.conflicts.len())])).font(theme::bold(12.5)));
            kit::note(ui, tr("Projects reference different versions. The highest becomes the central version."));
            for c in &plan.conflicts {
                let key = c.id.to_ascii_lowercase();
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    icons::image(ui, ico::PACKAGE, 16.0);
                    ui.label(egui::RichText::new(&c.id).font(theme::bold(12.5)));
                });
                let uses: Vec<String> = c.uses.iter().map(|(f, v)| format!("{} {v}", f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default())).collect();
                kit::note(ui, &uses.join(" · "));
                ui.horizontal(|ui| {
                    let options = vec![trf("Use {version} everywhere", &[("version", &c.chosen)]), tr("Keep lower versions as VersionOverride").to_string()];
                    let keep = mig.keep_lower.contains(&key);
                    if let Some(i) = kit::dropdown(ui, ("nuget_conflict", &key), &options[usize::from(keep)], &options, true)
                        && (i == 1) != keep
                    {
                        if i == 1 {
                            mig.keep_lower.insert(key.clone());
                        } else {
                            mig.keep_lower.remove(&key);
                        }
                        mig.replan();
                    }
                });
            }
        }
        if !plan.warnings.is_empty() {
            ui.add_space(8.0);
            ui.label(egui::RichText::new(trf("Warnings ({n})", &[("n", &plan.warnings.len())])).font(theme::bold(12.5)));
            for w in &plan.warnings {
                kit::message(ui, &warning_text(root.as_deref(), w), false);
            }
        }
    });
    lu.add_space(6.0);
    if let Some((err, msg)) = &message {
        lu.label(egui::RichText::new(msg).size(12.5).color(if *err { p.error } else { p.ok }));
    }
    lu.horizontal(|ui| {
        if ui.add_enabled_ui(mig.plan.is_ok() && !busy, |ui| theme::primary_button(ui, tr("Apply"))).inner.clicked() {
            action = Some(true);
        }
        if theme::secondary_button(ui, tr("Cancel")).clicked() {
            action = Some(false);
        }
    });
    // Right: the shown file, as a unified diff.
    let mut ru = ui.new_child(egui::UiBuilder::new().max_rect(right.shrink2(vec2(12.0, 0.0))));
    if let Ok(plan) = &mig.plan {
        let (path, lines) = match mig.shown.checked_sub(1).and_then(|i| plan.changes.get(i)) {
            Some(c) => (c.path.clone(), unified_diff(&c.old, &c.new)),
            None => (plan.central_file.clone(), plan.central_text.lines().map(|l| Some(('+', l.to_string()))).collect()),
        };
        ru.horizontal(|ui| {
            icons::image(ui, ico::FILE_XML, 16.0);
            ui.label(egui::RichText::new(relative(root.as_deref(), &path)).font(theme::bold(13.0)));
        });
        ru.add_space(4.0);
        diff_lines(&mut ru, &lines);
    }
    action
}

/// Paints a unified diff: added lines green, removed ones red, `⋯` between hunks.
fn diff_lines(ui: &mut Ui, lines: &[Option<(char, String)>]) {
    let p = rc_kit::pal(ui);
    let font = FontId::monospace(12.5);
    egui::ScrollArea::both().id_salt("nuget_diff").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let width = lines.iter().flatten().map(|(_, l)| ui.painter().layout_no_wrap(l.clone(), font.clone(), p.text).size().x).fold(0.0, f32::max) + 40.0;
        for line in lines {
            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width().max(width), 19.0), Sense::hover());
            let Some((kind, text)) = line else {
                ui.painter().text(pos2(r.left() + 6.0, r.center().y), Align2::LEFT_CENTER, "⋯", font.clone(), p.muted);
                continue;
            };
            let (bg, sign) = match kind {
                '+' => (Some(p.ok.gamma_multiply(0.16)), p.ok),
                '-' => (Some(p.error.gamma_multiply(0.16)), p.error),
                _ => (None, p.muted),
            };
            if let Some(bg) = bg {
                ui.painter().rect_filled(r, 0.0, bg);
            }
            ui.painter().text(pos2(r.left() + 6.0, r.center().y), Align2::LEFT_CENTER, kind.to_string(), font.clone(), sign);
            ui.painter().text(pos2(r.left() + 24.0, r.center().y), Align2::LEFT_CENTER, text.replace('\t', "    "), font.clone(), p.text);
        }
    });
}

/// "Critical", "High"… for a severity rank.
fn severity_label(rank: u8) -> &'static str {
    match rank {
        4 => tr("Critical"),
        3 => tr("High"),
        2 => tr("Moderate"),
        1 => tr("Low"),
        _ => "",
    }
}

/// The audit tab's list: run or cancel, then vulnerable and deprecated packages.
fn audit_list(ui: &mut Ui, m: &mut NugetManager) {
    let p = rc_kit::pal(ui);
    ui.horizontal(|ui| {
        ui.set_min_height(26.0);
        match &m.audit.running {
            Some((_, started)) => {
                let secs = started.elapsed().as_secs();
                if kit::button(ui, Some(ico::STOP), tr("Cancel audit"), true).clicked() {
                    m.cancel_audit();
                }
                ui.add(egui::Spinner::new().size(12.0));
                kit::note(ui, &trf("Auditing… {n} s", &[("n", &secs)]));
            }
            None => {
                let label = if m.audit.report.is_some() { tr("Run audit again") } else { tr("Run audit") };
                if kit::button(ui, Some(ico::RUN), label, m.dotnet.is_some() && !m.projects.is_empty()).clicked() {
                    m.start_audit();
                }
            }
        }
    });
    ui.add_space(4.0);
    if let Some(e) = &m.audit.error {
        kit::message(ui, e, true);
    }
    let Some(report) = m.audit.report.clone() else {
        if m.audit.running.is_none() {
            kit::note(ui, tr("The audit runs dotnet list package --vulnerable and --deprecated on the whole solution. It asks every package source, so it can take a while."));
        }
        return;
    };
    for problem in &report.problems {
        kit::message(ui, problem, false);
    }
    egui::ScrollArea::vertical().id_salt("nuget_audit").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        if report.packages.is_empty() {
            ui.add_space(16.0);
            ui.label(egui::RichText::new(tr("No vulnerable or deprecated packages")).color(p.muted));
        }
        for pkg in &report.packages {
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click());
            let selected = m.audit.selected.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(&pkg.id));
            if selected {
                ui.painter().rect_filled(r, 6.0, p.selection);
            } else if resp.hovered() {
                ui.painter().rect_filled(r, 6.0, p.row_hover);
            }
            let vulnerable = !pkg.advisories.is_empty();
            let icon = match pkg.severity() {
                4 | 3 => ico::ERROR,
                0 => ico::INFO,
                _ => ico::WARNING,
            };
            icons::paint(ui.painter(), Rect::from_center_size(pos2(r.left() + 16.0, r.top() + 14.0), vec2(16.0, 16.0)), icon, &p, true);
            let t = ui.painter().text(pos2(r.left() + 32.0, r.top() + 14.0), Align2::LEFT_CENTER, &pkg.id, theme::bold(13.0), p.text);
            let mut x = t.right() + 8.0;
            if vulnerable {
                x = theme::paint_badge(ui, pos2(x, r.top() + 14.0), severity_label(pkg.severity())).right() + 6.0;
            }
            if pkg.direct.is_empty() {
                x = theme::paint_badge(ui, pos2(x, r.top() + 14.0), tr("Transitive")).right() + 6.0;
            }
            if pkg.deprecation.is_some() {
                theme::paint_badge(ui, pos2(x, r.top() + 14.0), tr("Deprecated"));
            }
            let n = pkg.direct.len() + pkg.transitive.len();
            let mut desc = format!("{} · {}", pkg.version, if n == 1 { tr("1 project").to_string() } else { trf("{n} projects", &[("n", &n)]) });
            for a in &pkg.advisories {
                desc.push_str(" · ");
                desc.push_str(a.url.rsplit('/').next().unwrap_or(&a.url));
            }
            if let Some(d) = &pkg.deprecation {
                desc.push_str(" · ");
                desc.push_str(&d.reasons.join(", "));
            }
            let mut job = egui::text::LayoutJob::simple_singleline(desc, FontId::proportional(12.0), p.muted);
            job.wrap.max_width = r.width() - 40.0;
            job.wrap.max_rows = 1;
            job.wrap.overflow_character = Some('…');
            let g = ui.painter().layout_job(job);
            ui.painter().galley(pos2(r.left() + 32.0, r.top() + 25.0), g, p.muted);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &pkg.id));
            if resp.clicked() {
                m.audit.selected = Some(pkg.id.clone());
                m.audit.confirm_pin = None;
            }
        }
    });
}

/// The selected audit package: advisories, projects, deprecation and fixes.
fn audit_details(ui: &mut Ui, m: &mut NugetManager) {
    let p = rc_kit::pal(ui);
    let pkg = m.audit.selected.as_ref().and_then(|s| m.audit.report.as_ref()?.packages.iter().find(|q| q.id.eq_ignore_ascii_case(s)).cloned());
    let Some(pkg) = pkg else {
        ui.add_space(20.0);
        ui.label(egui::RichText::new(tr("Run the audit and select a package to see its advisories and fixes")).color(p.muted));
        if let Some((err, msg)) = &m.message {
            ui.label(egui::RichText::new(msg).size(12.5).color(if *err { p.error } else { p.ok }));
        }
        return;
    };
    let busy = m.busy.is_some();
    let mut edits: Vec<CpmEdit> = Vec::new();
    let mut ops: Vec<Op> = Vec::new();
    egui::ScrollArea::vertical().id_salt("nuget_audit_details").max_height((ui.available_height() - 28.0).max(60.0)).auto_shrink([false, true]).show(ui, |ui| {
        ui.horizontal(|ui| {
            icons::image(ui, ico::PACKAGE, 16.0);
            ui.label(egui::RichText::new(&pkg.id).font(theme::bold(15.0)));
            ui.label(egui::RichText::new(&pkg.version).size(13.0).color(if pkg.advisories.is_empty() { p.warn } else { p.error }));
        });
        if !pkg.advisories.is_empty() {
            ui.add_space(8.0);
            ui.label(egui::RichText::new(tr("Vulnerabilities")).font(theme::bold(12.5)));
            for a in &pkg.advisories {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(72.0, 18.0), Sense::hover());
                    theme::paint_badge(ui, pos2(r.left(), r.center().y), severity_label(rc_dotnet::nuget::severity_rank(&a.severity)));
                    ui.hyperlink_to(egui::RichText::new(a.url.rsplit('/').next().unwrap_or(&a.url)).size(12.5), &a.url);
                });
            }
        }
        if let Some(d) = &pkg.deprecation {
            ui.add_space(8.0);
            ui.label(egui::RichText::new(tr("Deprecated")).font(theme::bold(12.5)));
            if !d.reasons.is_empty() {
                kit::note(ui, &trf("Reason: {reasons}", &[("reasons", &d.reasons.join(", "))]));
            }
            match &d.alternative {
                Some((id, range)) => kit::note(ui, &trf("Use {id} {range} instead", &[("id", id), ("range", range)])),
                None => kit::note(ui, tr("No alternative package suggested")),
            }
        }
        ui.add_space(8.0);
        ui.label(egui::RichText::new(tr("Projects")).font(theme::bold(12.5)));
        for (project, transitive) in pkg.direct.iter().map(|x| (x, false)).chain(pkg.transitive.iter().map(|x| (x, true))) {
            ui.horizontal(|ui| {
                icons::image(ui, ico::PROJECT, 16.0);
                ui.label(egui::RichText::new(project_name(project)).size(13.0));
                let c = m.cpm.project(project).filter(|c| c.enabled && !transitive);
                let source = match c {
                    Some(c) if c.reference(&pkg.id).is_some_and(|r| r.version_override.is_some()) => Some(tr("Override")),
                    Some(c) if c.is_central(&pkg.id) => Some(tr("Central")),
                    _ => None,
                };
                let (r, _) = ui.allocate_exact_size(vec2(90.0, 18.0), Sense::hover());
                let x = theme::paint_badge(ui, pos2(r.left(), r.center().y), if transitive { tr("Transitive") } else { tr("Direct") }).right() + 6.0;
                if let Some(s) = source {
                    theme::paint_badge(ui, pos2(x, r.center().y), s);
                }
            });
        }
        if pkg.advisories.is_empty() {
            return;
        }
        // Fixes: the first version without known vulnerabilities.
        ui.add_space(10.0);
        ui.label(egui::RichText::new(tr("Fix")).font(theme::bold(12.5)));
        let safe = m.audit.safe.get(&pkg.id.to_lowercase()).cloned();
        let Some(safe) = safe else {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(12.0));
                kit::note(ui, tr("Looking for a version without known vulnerabilities…"));
            });
            return;
        };
        let Some(version) = safe else {
            kit::message(ui, tr("No version without known vulnerabilities was found"), false);
            return;
        };
        if !pkg.direct.is_empty() {
            let central = pkg.direct.iter().any(|d| m.cpm.is_central(d, &pkg.id));
            ui.horizontal(|ui| {
                if kit::button(ui, Some(ico::UP), &trf("Update to {version}", &[("version", &version)]), !busy).clicked() {
                    let (e, o) = m.plan_install(&pkg.direct, &pkg.id, &version);
                    edits.extend(e);
                    ops.extend(o);
                }
                kit::note(ui, &if central { tr("Updates the central version").to_string() } else { trf("Updates {n} projects", &[("n", &pkg.direct.len())]) });
            });
        }
        let (pins, refs) = m.plan_pin(&pkg, &version);
        if !pins.is_empty() {
            ui.horizontal(|ui| {
                if kit::button(ui, Some(ico::PIN), &trf("Pin version {version}", &[("version", &version)]), !busy && m.audit.confirm_pin.is_none()).clicked() {
                    m.audit.confirm_pin = Some(pkg.id.clone());
                }
                kit::note(ui, tr("For projects that get it through another package"));
            });
        }
        if !refs.is_empty() {
            ui.horizontal(|ui| {
                if kit::button(ui, Some(ico::ADD), &trf("Reference {version} directly", &[("version", &version)]), !busy).clicked() {
                    ops.extend(refs.clone());
                }
                kit::note(ui, &trf("In {n} projects without Central Package Management", &[("n", &refs.len())]));
            });
        }
        // Pinning changes how the central file works: explain and ask first.
        if m.audit.confirm_pin.as_deref() == Some(pkg.id.as_str()) {
            ui.add_space(6.0);
            for e in &pins {
                let CpmEdit::Pin { file, .. } = e else { continue };
                let on = m.cpm.files.get(file).and_then(|f| f.transitive_pinning).unwrap_or(false);
                let name = m.display_path(file);
                let text = if on {
                    trf("Transitive pinning is already on in {file}: a PackageVersion for {id} {version} raises it wherever projects get it through other packages.", &[("file", &name), ("id", &pkg.id), ("version", &version)])
                } else {
                    trf(
                        "Pinning adds a PackageVersion for {id} {version} to {file} and turns on CentralPackageTransitivePinningEnabled. Then every PackageVersion of the file also sets the version of packages that projects get through other packages, so {id} is raised to {version} there; other transitive packages listed in the file are raised too.",
                        &[("file", &name), ("id", &pkg.id), ("version", &version)],
                    )
                };
                kit::message(ui, &text, false);
            }
            ui.horizontal(|ui| {
                if theme::primary_button(ui, tr("Pin")).clicked() {
                    edits.extend(pins.clone());
                    m.audit.confirm_pin = None;
                }
                if theme::secondary_button(ui, tr("Cancel")).clicked() {
                    m.audit.confirm_pin = None;
                }
            });
        }
    });
    if let Some(b) = &m.busy {
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(14.0));
            ui.label(egui::RichText::new(b).size(12.5).color(p.muted));
        });
    } else if let Some((err, msg)) = &m.message {
        ui.label(egui::RichText::new(msg).size(12.5).color(if *err { p.error } else { p.ok }));
    }
    m.edits.extend(edits);
    m.ops.extend(ops);
}


/// Takes finished work; true when references changed (the explorer reloads them).
fn poll(m: &mut NugetManager) -> bool {
    let mut changed = false;
    for ev in m.inbox.drain() {
        match ev {
            Event::Results(g, r) if g == m.generation => {
                m.searching = false;
                match r {
                    Ok(list) => {
                        for p in &list {
                            m.details.insert(p.id.to_lowercase(), p.clone());
                        }
                        m.results = list;
                        m.message = None;
                    }
                    Err((e, auth)) => {
                        m.message = Some((true, trf("Could not search NuGet: {error}", &[("error", &e)])));
                        // Expired or revoked credentials: check the source again.
                        if let (true, Some(src)) = (auth, m.sources.get(m.source).cloned()) {
                            m.clients.remove(&src.url);
                            m.status.remove(&src.url);
                            m.auth.forget(&src.url);
                            m.connect(m.source);
                        }
                    }
                }
            }
            Event::Results(..) => {}
            Event::Audit(g, r) if g == m.audit.generation => {
                m.audit.running = None;
                match r {
                    Ok(report) => m.set_audit(report),
                    Err(e) => m.audit.error = Some(e),
                }
            }
            Event::Audit(..) => {}
            Event::Safe(id, v) => {
                m.audit.safe.insert(id, v);
            }
            Event::Versions(id, mut v) => {
                if m.selected.as_deref() == Some(id.as_str()) {
                    v.sort_by(|a, b| compare_versions(b, a));
                    if !m.prerelease {
                        v.retain(|x| !x.contains('-'));
                    }
                    m.version = v.first().cloned().unwrap_or_default();
                    m.versions = v;
                }
            }
            Event::Latest(id, v) => {
                if let Some(v) = v {
                    m.latest.insert(id.to_lowercase(), v);
                }
            }
            Event::Details(id, d) => {
                if let Some(d) = d {
                    m.details.insert(id.to_lowercase(), d);
                }
            }
            Event::SignInMessage(url, text) => {
                if let Some(d) = m.sign_in.as_mut().filter(|d| d.url == url) {
                    let text = text.trim().trim_start_matches("[CredentialProvider]").trim().to_string();
                    if !text.is_empty() {
                        d.messages.push(text);
                    }
                }
            }
            Event::Connected(url, status, client, from_sign_in) => {
                let name = m.sources.iter().find(|s| s.url == url).map_or_else(|| url.clone(), |s| s.name.clone());
                let current = m.sources.get(m.source).is_some_and(|s| s.url == url);
                if let Some(c) = client {
                    m.clients.insert(url.clone(), c);
                }
                if from_sign_in {
                    if let Some(d) = m.sign_in.as_mut().filter(|d| d.url == url) {
                        match &status {
                            SourceStatus::Ok => {
                                m.sign_in = None;
                                m.notices.push((trf("Signed in to {source}", &[("source", &name)]), false));
                            }
                            SourceStatus::NeedsSignIn(why) | SourceStatus::Failed(why) => {
                                d.task = None;
                                d.error = Some(why.clone());
                            }
                            SourceStatus::Checking => {}
                        }
                    }
                } else if let (true, SourceStatus::Failed(why)) = (current, &status) {
                    m.notices.push((trf("Could not connect to {source}: {error}", &[("source", &name), ("error", why)]), true));
                }
                let ok = status == SourceStatus::Ok;
                m.status.insert(url, status);
                // The current source is ready: load what waited for it.
                if ok && current {
                    m.last_query = None;
                    m.latest.clear();
                    m.reload_installed();
                    if let Some(id) = m.selected.clone() {
                        m.select(&id);
                    }
                }
            }
            Event::Cpm(g, list) => {
                if g == m.cpm_generation {
                    m.cpm_evaluating = false;
                    for c in list {
                        m.cpm_cache.insert(c);
                    }
                    let list: Vec<ProjectCpm> = m.projects.iter().map(|p| m.cpm_cache.get(p).cloned().unwrap_or_else(|| cpm::detect_static(p))).collect();
                    m.cpm = SolutionCpm::new(list);
                    m.rebuild_installed();
                    changed = true;
                }
            }
            Event::Graph(generation, graphs) => {
                if generation == m.graph_generation {
                    m.graph = graphs;
                    m.graph_loading = false;
                    m.graph_loaded = true;
                }
            }
            Event::Done(r) => {
                m.busy = None;
                changed = true;
                m.graph_loaded = false;
                m.graph_loading = false;
                m.graph_generation += 1;
                if m.tab == Tab::Graph { m.load_graph(); }
                m.message = Some(match r {
                    Ok(msg) => (false, msg),
                    Err(e) => {
                        m.ops.clear();
                        (true, e)
                    }
                });
                if m.ops.is_empty() {
                    m.reload_installed();
                }
            }
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::{HOME_LOCK, fixtures, test_app};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    /// A copy of CentralShop (temp path canonicalized for macOS).
    fn central_shop() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::Builder::new().prefix("dotcraft-cpm-ui").tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap().join("CentralShop");
        copy(&fixtures().join("CentralShop"), &root);
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

    fn harness(root: &Path, home: &Path) -> Harness<'static, DotCraftApp> {
        harness_at(&root.join("CentralShop.slnx"), home)
    }

    fn harness_at(sln: &Path, home: &Path) -> Harness<'static, DotCraftApp> {
        let mut app = test_app(home, true);
        app.settings.roslyn_path = Some("/nonexistent".into());
        app.open_workspace(sln);
        let mut first = true;
        let mut h = Harness::builder().with_size(vec2(1100.0, 700.0)).build_ui_state(
            move |ui, app: &mut DotCraftApp| {
                if std::mem::take(&mut first) {
                    rc_kit::theme::install_fonts(ui.ctx());
                    return;
                }
                window(app, ui);
            },
            app,
        );
        h.run_steps(2);
        wait(&mut h);
        h
    }

    /// Waits for MSBuild and any restore.
    fn wait(h: &mut Harness<'static, DotCraftApp>) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        loop {
            h.run_steps(1);
            if h.state().nuget.cpm_ready_for_test() {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "NuGet window did not settle");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        h.run_steps(2);
    }

    #[test]
    fn central_packages_show_where_they_live_and_move_overrides() {
        let _lock = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = tempfile::tempdir().unwrap();
        let (_tmp, root) = central_shop();
        let mut h = harness(&root, home.path());
        let m = &h.state().nuget;
        assert!(m.cpm.in_use());
        assert!(m.cpm.projects.iter().all(|p| p.evaluated && p.enabled));
        assert!(m.is_central("Newtonsoft.Json") && m.is_central("Humanizer.Core"));
        // The Central issues tab reports the unused Dapper version.
        h.get_by_label("Central issues (1)");
        h.state_mut().nuget.select_for_test("Newtonsoft.Json");
        h.state_mut().nuget.version = "13.0.4".into();
        h.run_steps(3);
        h.get_by_label("Directory.Packages.props");
        h.get_by_label("Update to 13.0.4");
        h.get_by_label("Align to central version");
        // The override becomes the central version: one change for every project.
        h.get_by_label("Move override to central").click();
        h.run_steps(2);
        let central = std::fs::read_to_string(root.join("Directory.Packages.props")).unwrap();
        assert!(central.contains(r#"<PackageVersion Include="Newtonsoft.Json" Version="13.0.3" />"#), "{central}");
        assert!(central.contains("<!-- Versions shared by every project -->"));
        let cli = std::fs::read_to_string(root.join("src/CentralShop.Cli/CentralShop.Cli.csproj")).unwrap();
        assert!(cli.contains(r#"<PackageReference Include="Newtonsoft.Json" />"#), "{cli}");
        wait(&mut h);
        let m = &h.state().nuget;
        assert!(m.message.as_ref().is_some_and(|(err, _)| !err), "{:?}", m.message);
        assert_eq!(m.installed_versions("Newtonsoft.Json").iter().map(|(_, v)| v.as_str()).collect::<Vec<_>>(), ["13.0.3", "13.0.3"]);
    }

    #[test]
    fn central_issues_are_fixed_and_unused_versions_offered_for_removal() {
        let _lock = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = tempfile::tempdir().unwrap();
        let (_tmp, root) = central_shop();
        let mut h = harness(&root, home.path());
        h.state_mut().nuget.show_central_for_test();
        h.run_steps(2);
        h.get_by_label("Remove version").click();
        h.run_steps(2);
        let central = std::fs::read_to_string(root.join("Directory.Packages.props")).unwrap();
        assert!(!central.contains("Dapper"), "{central}");
        wait(&mut h);
        assert!(h.state().nuget.cpm.issues().is_empty());
        // Removing Humanizer from its only project offers to drop its version.
        h.state_mut().nuget.tab = Tab::Installed;
        h.state_mut().nuget.select_for_test("Humanizer.Core");
        h.run_steps(3);
        h.get_by_label("Remove").click();
        h.run_steps(2);
        let core = std::fs::read_to_string(root.join("src/CentralShop.Core/CentralShop.Core.csproj")).unwrap();
        assert!(!core.contains("Humanizer"), "{core}");
        wait(&mut h);
        assert!(h.state().nuget.drop_offer.is_some());
        h.get_by_label("Remove version").click();
        h.run_steps(2);
        assert!(!std::fs::read_to_string(root.join("Directory.Packages.props")).unwrap().contains("Humanizer"));
        wait(&mut h);
    }

    #[test]
    fn installing_in_cpm_projects_edits_the_central_file_once() {
        let _lock = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = tempfile::tempdir().unwrap();
        let (_tmp, root) = central_shop();
        let mut h = harness(&root, home.path());
        let central = root.join("Directory.Packages.props");
        let cli = root.join("src/CentralShop.Cli/CentralShop.Cli.csproj");
        // Dapper has a central version: the project only gets a reference.
        {
            let m = &mut h.state_mut().nuget;
            m.selected = Some("Dapper".into());
            m.version = "9.9.9".into();
        }
        h.run_steps(3);
        h.get_all_by_label("Install").next().unwrap().click();
        h.run_steps(2);
        assert!(std::fs::read_to_string(&cli).unwrap().contains("    <PackageReference Include=\"Dapper\" />\n    <PackageReference Include=\"Newtonsoft.Json\" VersionOverride=\"13.0.3\" />"));
        assert!(std::fs::read_to_string(&central).unwrap().contains(r#"<PackageVersion Include="Dapper" Version="2.1.66" />"#));
        wait(&mut h);
        // A new package in both projects: one PackageVersion, sorted, and two references.
        {
            let app = h.state_mut();
            app.nuget.selected = Some("Microsoft.Extensions.Logging.Abstractions".into());
            app.nuget.version = "8.0.0".into();
            app.nuget.checked = app.nuget.projects.iter().cloned().collect();
        }
        h.run_steps(3);
        h.get_by_label("Install or update in 2 selected").click();
        h.run_steps(2);
        let text = std::fs::read_to_string(&central).unwrap();
        assert!(text.contains("    <PackageVersion Include=\"Humanizer.Core\" Version=\"2.14.1\" />\n    <PackageVersion Include=\"Microsoft.Extensions.Logging.Abstractions\" Version=\"8.0.0\" />\n"), "{text}");
        assert_eq!(text.matches("Microsoft.Extensions.Logging.Abstractions").count(), 1);
        assert!(std::fs::read_to_string(root.join("src/CentralShop.Core/CentralShop.Core.csproj")).unwrap().contains(r#"<PackageReference Include="Microsoft.Extensions.Logging.Abstractions" />"#));
        wait(&mut h);
        let m = &h.state().nuget;
        assert!(m.message.as_ref().is_some_and(|(err, _)| !err), "{:?}", m.message);
        assert!(m.cpm.issues().is_empty(), "{:?}", m.cpm.issues());
    }

    #[test]
    fn solutions_without_cpm_are_not_evaluated() {
        let _lock = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = tempfile::tempdir().unwrap();
        let mut app = test_app(home.path(), true);
        app.settings.roslyn_path = Some("/nonexistent".into());
        app.open_workspace(&fixtures().join("Shop/Shop.sln"));
        let mut first = true;
        let mut h = Harness::builder().with_size(vec2(1100.0, 700.0)).build_ui_state(
            move |ui, app: &mut DotCraftApp| {
                if std::mem::take(&mut first) {
                    rc_kit::theme::install_fonts(ui.ctx());
                    return;
                }
                window(app, ui);
            },
            app,
        );
        h.run_steps(3);
        let m = &h.state().nuget;
        assert!(m.loaded && !m.cpm_evaluating && !m.cpm.in_use());
        assert!(m.installed_versions("xunit").iter().all(|(_, v)| !v.is_empty()));
        assert!(!m.is_central("xunit"));
        assert!(h.query_by_label_contains("Central issues").is_none());
    }

    #[test]
    fn open_editors_with_unsaved_changes_are_edited_not_overwritten() {
        let _lock = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = tempfile::tempdir().unwrap();
        let (_tmp, root) = central_shop();
        let mut h = harness(&root, home.path());
        let file = root.join("Directory.Packages.props");
        let on_disk = std::fs::read_to_string(&file).unwrap();
        {
            let app = h.state_mut();
            app.open_file(&file);
            let d = app.docs.active_doc_mut().unwrap();
            d.editor.apply_text_edits(vec![rc_text::Change::insert(0, "<!-- mine -->\n")]);
            app.nuget.edits.push(CpmEdit::SetCentral { file: file.clone(), id: "Humanizer.Core".into(), version: "2.14.0".into() });
        }
        h.run_steps(2);
        let app = h.state();
        let text = app.docs.active_doc().unwrap().editor.buffer.rope().to_string();
        assert!(text.starts_with("<!-- mine -->\n") && text.contains(r#""Humanizer.Core" Version="2.14.0""#), "{text}");
        assert!(app.docs.active_doc().unwrap().is_modified());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), on_disk);
        assert!(app.nuget.message.as_ref().is_some_and(|(err, m)| *err && m.contains("unsaved")));
        // An editor without changes is edited and saved.
        let proj = root.join("src/CentralShop.Cli/CentralShop.Cli.csproj");
        {
            let app = h.state_mut();
            app.docs.list.retain(|d| d.path != file);
            app.docs.active = None;
            app.open_file(&proj);
            app.nuget.edits.push(CpmEdit::Align { project: proj.clone(), id: "Newtonsoft.Json".into() });
        }
        h.run_steps(2);
        assert!(!h.state().docs.active_doc().unwrap().is_modified());
        assert!(std::fs::read_to_string(&proj).unwrap().contains(r#"<PackageReference Include="Newtonsoft.Json" />"#));
        wait(&mut h);
    }

    /// A copy of Shop (no CPM) where two projects disagree on coverlet.collector.
    fn shop_with_conflict() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::Builder::new().prefix("dotcraft-cpm-migrate-ui").tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap().join("Shop");
        copy(&fixtures().join("Shop"), &root);
        let nunit = root.join("tests/Shop.Core.NunitTests/Shop.Core.NunitTests.csproj");
        let text = std::fs::read_to_string(&nunit).unwrap().replace(r#""coverlet.collector" Version="6.0.0""#, r#""coverlet.collector" Version="6.0.4""#);
        std::fs::write(&nunit, text).unwrap();
        (tmp, root)
    }

    #[test]
    fn migration_is_previewed_then_applied_and_restored() {
        let _lock = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = tempfile::tempdir().unwrap();
        let (_tmp, root) = shop_with_conflict();
        let mut h = harness_at(&root.join("Shop.sln"), home.path());
        let api = root.join("src/Shop.Api/Shop.Api.csproj");
        // An open editor is edited (and saved) rather than overwritten.
        h.state_mut().open_file(&api);
        h.get_by_label("Migrate to Central Package Management…").click();
        h.run_steps(3);
        let central = root.join("Directory.Packages.props");
        {
            let m = &h.state().nuget;
            let plan = m.migration.as_ref().unwrap().plan.as_ref().unwrap();
            assert_eq!(plan.conflicts.len(), 1);
            assert_eq!(plan.changes.len(), 4);
        }
        // Nothing is written before Apply.
        assert!(!central.exists());
        // The new file is listed and shown first.
        assert_eq!(h.get_all_by_label("Directory.Packages.props").count(), 2);
        h.get_by_label("Shop.Core.XunitTests.csproj").click();
        h.run_steps(2);
        {
            let mig = h.state().nuget.migration.as_ref().unwrap();
            let shown = &mig.plan.as_ref().unwrap().changes[mig.shown - 1];
            assert!(shown.path.ends_with("Shop.Core.XunitTests.csproj"));
        }
        // Keep the lower version in the xunit project.
        h.get_by_label("Use 6.0.4 everywhere").click();
        h.run_steps(2);
        h.get_by_label("Keep lower versions as VersionOverride").click();
        h.run_steps(2);
        assert!(h.state().nuget.migration.as_ref().unwrap().keep_lower.contains("coverlet.collector"));
        h.get_by_label("Apply").click();
        h.run_steps(2);
        let text = std::fs::read_to_string(&central).unwrap();
        assert!(text.contains(r#"<PackageVersion Include="coverlet.collector" Version="6.0.4" />"#), "{text}");
        assert!(text.contains("<ManagePackageVersionsCentrally>true</ManagePackageVersionsCentrally>"));
        let xunit = std::fs::read_to_string(root.join("tests/Shop.Core.XunitTests/Shop.Core.XunitTests.csproj")).unwrap();
        assert!(xunit.contains(r#"<PackageReference Include="coverlet.collector" VersionOverride="6.0.0" />"#), "{xunit}");
        assert!(xunit.contains(r#"<PackageReference Include="xunit" />"#));
        assert!(std::fs::read_to_string(&api).unwrap().contains(r#"<PackageReference Include="Swashbuckle.AspNetCore" />"#));
        assert!(!h.state().docs.active_doc().unwrap().is_modified());
        assert!(h.state().nuget.migration.is_none());
        wait(&mut h);
        let m = &h.state().nuget;
        assert!(m.message.as_ref().is_some_and(|(err, msg)| !err && msg.contains("restored")), "{:?}", m.message);
        assert!(m.cpm.in_use() && m.cpm.issues().is_empty(), "{:?}", m.cpm.issues());
        assert!(m.is_central("xunit"));
    }

    #[test]
    fn cancelling_a_migration_writes_nothing() {
        let _lock = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = tempfile::tempdir().unwrap();
        let (_tmp, root) = shop_with_conflict();
        let before = std::fs::read_to_string(root.join("src/Shop.Api/Shop.Api.csproj")).unwrap();
        let mut h = harness_at(&root.join("Shop.sln"), home.path());
        h.get_by_label("Migrate to Central Package Management…").click();
        h.run_steps(3);
        h.get_by_label("Cancel").click();
        h.run_steps(2);
        assert!(h.state().nuget.migration.is_none());
        assert!(!root.join("Directory.Packages.props").exists());
        assert_eq!(std::fs::read_to_string(root.join("src/Shop.Api/Shop.Api.csproj")).unwrap(), before);
    }

    /// The sample `dotnet list package --vulnerable` output, on CentralShop's projects.
    fn sample_audit(root: &Path) -> AuditReport {
        let json = include_str!("../../../rc-dotnet/tests/data/list-package-vulnerable.json")
            .replace("/work/Audit/A/A.csproj", &root.join("src/CentralShop.Core/CentralShop.Core.csproj").display().to_string())
            .replace("/work/Audit/B/B.csproj", &root.join("src/CentralShop.Cli/CentralShop.Cli.csproj").display().to_string());
        rc_dotnet::nuget::parse_list_packages(&json).unwrap()
    }

    #[test]
    fn audit_updates_direct_packages_and_pins_transitive_ones() {
        let _lock = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = tempfile::tempdir().unwrap();
        let (_tmp, root) = central_shop();
        let mut h = harness(&root, home.path());
        let central = root.join("Directory.Packages.props");
        let cli = root.join("src/CentralShop.Cli/CentralShop.Cli.csproj");
        h.state_mut().nuget.set_audit_for_test(sample_audit(&root), &[("Newtonsoft.Json", "13.0.3"), ("System.Net.Http", "4.3.4"), ("System.Text.RegularExpressions", "4.3.1")]);
        h.run_steps(3);
        h.get_by_label("Audit (3)");
        h.get_by_label("GHSA-5crp-9r3c-p9vr");
        // Newtonsoft.Json: the central version and the CLI's override move to 13.0.3.
        h.get_by_label("Update to 13.0.3").click();
        h.run_steps(3);
        assert!(std::fs::read_to_string(&central).unwrap().contains(r#"<PackageVersion Include="Newtonsoft.Json" Version="13.0.3" />"#));
        assert!(std::fs::read_to_string(&cli).unwrap().contains(r#"VersionOverride="13.0.3""#));
        wait(&mut h);
        // A transitive package: pinning asks first.
        h.get_by_label("System.Text.RegularExpressions").click();
        h.run_steps(2);
        h.get_by_label("Pin version 4.3.1").click();
        h.run_steps(2);
        assert!(h.query_by_label_contains("turns on CentralPackageTransitivePinningEnabled").is_some());
        assert!(!std::fs::read_to_string(&central).unwrap().contains("RegularExpressions"));
        h.get_by_label("Pin").click();
        h.run_steps(3);
        let text = std::fs::read_to_string(&central).unwrap();
        assert!(text.contains(r#"<PackageVersion Include="System.Text.RegularExpressions" Version="4.3.1" />"#), "{text}");
        assert!(text.contains("<CentralPackageTransitivePinningEnabled>true</CentralPackageTransitivePinningEnabled>"), "{text}");
        wait(&mut h);
        let m = &h.state().nuget;
        assert!(m.message.as_ref().is_some_and(|(err, _)| !err), "{:?}", m.message);
        assert!(m.cpm.projects.iter().all(|p| p.transitive_pinning));
    }

    #[test]
    fn audit_runs_only_on_demand_and_can_be_cancelled() {
        let _lock = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = tempfile::tempdir().unwrap();
        let (_tmp, root) = central_shop();
        let mut h = harness(&root, home.path());
        h.get_by_label("Audit").click();
        h.run_steps(2);
        assert!(h.state().nuget.audit.running.is_none() && h.state().nuget.audit.report.is_none());
        h.get_by_label("Run audit").click();
        h.run_steps(2);
        assert!(h.state().nuget.audit.running.is_some());
        h.get_by_label("Cancel audit").click();
        h.run_steps(2);
        assert!(h.state().nuget.audit.running.is_none() && h.state().nuget.audit.report.is_none());
        h.get_by_label("Run audit");
    }

    #[test]
    #[ignore = "requires a local .NET 8 SDK and cached Shop packages"]
    fn dependency_graph_tab_shows_resolved_shop_edges() {
        let host = std::env::var_os("DOTCRAFT_TEST_DOTNET").expect("set DOTCRAFT_TEST_DOTNET");
        let _lock = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let shop = dir.path().join("Shop");
        copy(&fixtures().join("Shop"), &shop);
        std::fs::write(shop.join("NuGet.Config"), "<configuration><packageSources><clear/></packageSources></configuration>\n").unwrap();
        let dotnet = rc_dotnet::Dotnet::new(host.clone());
        let restored = dotnet.std_command().arg("restore").arg(shop.join("Shop.sln")).args(["-p:NuGetAudit=false", "--ignore-failed-sources", "-v:quiet"]).output().unwrap();
        assert!(restored.status.success(), "{}\n{}", String::from_utf8_lossy(&restored.stdout), String::from_utf8_lossy(&restored.stderr));
        let mut app = test_app(&dir.path().join("home"), true);
        app.settings.dotnet_path = Some(PathBuf::from(host));
        app.settings.roslyn_path = Some("/nonexistent".into());
        app.open_workspace(&shop.join("Shop.sln"));
        let mut first = true;
        let mut h = Harness::builder().with_size(vec2(1100.0, 700.0)).build_ui_state(move |ui, app: &mut DotCraftApp| {
            if std::mem::take(&mut first) { rc_kit::theme::install_fonts(ui.ctx()); return; }
            window(app, ui);
        }, app);
        h.run_steps(3);
        h.get_by_label("Dependency graph").click();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !h.state().nuget.graph_loaded && std::time::Instant::now() < deadline {
            h.run_steps(1);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(h.state().nuget.graph_loaded);
        let api = shop.join("src/Shop.Api/Shop.Api.csproj");
        let index = h.state().nuget.graph.iter().position(|(project, _)| project == &api).unwrap();
        h.state_mut().nuget.graph_project = index;
        h.run_steps(3);
        let graph = h.state().nuget.graph[index].1.as_ref().unwrap();
        assert!(graph.edges.len() >= 5);
        h.get_by_label("Swashbuckle.AspNetCore 6.6.2 (Direct)").click();
        h.run_steps(2);
        h.get_by_label("Swashbuckle.AspNetCore.Swagger 6.6.2");
        h.get_by_label("Microsoft.OpenApi 1.6.14 (Transitive)");
    }
}

/// Icon and short label for a source's status.
fn status_look(s: Option<&SourceStatus>) -> (rc_kit::Ico, &'static str) {
    match s {
        Some(SourceStatus::Ok) => (ico::SUCCESS, tr("OK")),
        Some(SourceStatus::NeedsSignIn(_)) => (ico::USER, tr("Sign-in required")),
        Some(SourceStatus::Failed(_)) => (ico::ERROR, tr("Failed")),
        Some(SourceStatus::Checking) | None => (ico::REFRESH, tr("Checking…")),
    }
}

/// The source drop-down: each source with its status (the reason on hover); returns a pick.
fn source_picker(ui: &mut Ui, m: &NugetManager) -> Option<usize> {
    let current = m.sources.get(m.source).map_or("nuget.org", |s| s.name.as_str());
    let (icon, _) = status_look(m.current_status());
    let resp = ui.push_id("nuget_source", |ui| theme::flat_button(ui, Some(icon), current, FontId::proportional(12.5), true, true)).inner;
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, current));
    let mut picked = None;
    egui::Popup::menu(&resp).show(|ui| {
        ui.set_min_width(280.0);
        for (i, s) in m.sources.iter().enumerate() {
            let status = m.status.get(&s.url);
            let (icon, label) = status_look(status);
            let r = theme::menu_entry(ui, Some(icon), &s.name, Some(label), i == m.source, true);
            let r = match status {
                Some(SourceStatus::NeedsSignIn(why) | SourceStatus::Failed(why)) if !why.is_empty() => r.on_hover_text(format!("{}\n{why}", s.url)),
                _ => r.on_hover_text(&s.url),
            };
            if r.clicked() {
                picked = Some(i);
                ui.close();
            }
        }
    });
    picked
}

/// The Sign in dialog: the credential provider's device flow instructions (page and code) while
/// it waits, or why signing in failed.
pub fn sign_in_dialog(app: &mut DotCraftApp, ctx: &egui::Context) {
    if app.nuget.sign_in.is_none() {
        return;
    }
    pump(app);
    let m = &mut app.nuget;
    let Some(d) = m.sign_in.as_ref() else { return };
    let running = d.task.is_some();
    if running {
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
    // The device flow message gives the page and the code; other messages are shown as notes.
    let (mut link, mut code) = (None, None);
    for msg in &d.messages {
        let (l, c) = nuget_auth::device_code(msg);
        link = l.or(link);
        code = c.or(code);
    }
    let (mut cancel, mut retry, mut open) = (false, false, false);
    let resp = egui::Modal::new(egui::Id::new("nuget_sign_in")).show(ctx, |ui| {
        ui.set_width(460.0);
        let p = rc_kit::pal(ui);
        ui.horizontal(|ui| {
            icons::image(ui, ico::USER, 16.0);
            ui.label(egui::RichText::new(trf("Sign in to {source}", &[("source", &d.name)])).font(theme::bold(15.0)));
        });
        ui.add_space(2.0);
        crate::ui::kit::note(ui, &d.url);
        ui.add_space(10.0);
        if let Some(code) = &code {
            ui.label(egui::RichText::new(tr("Open the sign-in page in a browser and enter this code:")).size(13.0));
            ui.add_space(6.0);
            egui::Frame::new().fill(p.control_bg).stroke(egui::Stroke::new(1.0, p.control_border)).corner_radius(6).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(code).font(theme::mono_bold(18.0)).color(p.text));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if theme::icon_button(ui, ico::COPY, true, tr("Copy Code")).clicked() {
                            ui.ctx().copy_text(code.clone());
                        }
                    });
                });
            });
            if let Some(link) = &link {
                ui.add_space(4.0);
                crate::ui::kit::note(ui, link);
            }
        }
        for msg in d.messages.iter().filter(|m| nuget_auth::device_code(m).1.is_none()) {
            crate::ui::kit::note(ui, msg);
        }
        ui.add_space(8.0);
        if running {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(14.0));
                crate::ui::kit::note(ui, if code.is_some() { tr("Waiting for you to sign in…") } else { tr("Asking the credential provider…") });
            });
        } else if let Some(e) = &d.error {
            crate::ui::kit::message(ui, &trf("Could not sign in: {error}", &[("error", e)]), true);
        }
        ui.add_space(6.0);
        crate::ui::kit::note(ui, tr("The token stays in memory until DotCraft closes; it is never saved."));
        ui.add_space(14.0);
        // Fixed height, so the dialog shrinks when its content does.
        ui.allocate_ui_with_layout(vec2(ui.available_width(), 28.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if running {
                if link.is_some() && theme::primary_button(ui, tr("Open Browser")).clicked() {
                    open = true;
                }
                cancel = theme::secondary_button(ui, tr("Cancel")).clicked();
            } else {
                retry = theme::primary_button(ui, tr("Try Again")).clicked();
                cancel = theme::secondary_button(ui, tr("Close")).clicked();
            }
        });
    });
    if open && let Some(link) = &link {
        crate::ui::run_windows::open_url(link);
    }
    if retry {
        let (name, url) = (d.name.clone(), d.url.clone());
        m.start_sign_in(name, url);
    } else if cancel || resp.should_close() {
        m.cancel_sign_in();
    }
}
