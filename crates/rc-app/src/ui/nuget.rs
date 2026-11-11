//! NuGet tool window for the whole solution (Rider's NuGet window): browse a source, the
//! installed packages, the ones with updates and the ones to consolidate; a package's details
//! and, per project, its installed version with Install / Update / Remove.
//!
//! Projects with Central Package Management (CPM) are edited in their `Directory.Packages.props`
//! instead of through `dotnet add package`: one version for the whole solution, overrides per
//! project, and the issues NuGet reports for it (NU1008, NU1010, unused versions) with fixes.

use crate::app::DotCraftApp;
use crate::docs::Docs;
use crate::toolwindow::ToolWindow;
use crate::ui::kit;
use egui::{Align2, FontId, Rect, Sense, Ui, pos2, vec2};
use rc_core::{Inbox, Runtime, Waker};
use rc_dotnet::cpm::{self, CpmCache, CpmIssue, ProjectCpm, SolutionCpm};
use rc_dotnet::nuget::{PackageSource, compare_versions};
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
    /// Central Package Management issues (only when the solution uses CPM).
    Central,
}

enum Event {
    Results(u64, Result<Vec<PackageSummary>, String>),
    Versions(String, Vec<String>),
    Latest(String, Option<String>),
    Details(String, Option<PackageSummary>),
    Done(Result<String, String>),
    /// MSBuild evaluated CPM for these projects.
    Cpm(u64, Vec<ProjectCpm>),
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
    /// CPM of every project: cached or static results first, MSBuild's when they arrive.
    cpm: SolutionCpm,
    cpm_cache: CpmCache,
    cpm_generation: u64,
    cpm_evaluating: bool,
    /// CPM changes waiting for the window (they need the open documents).
    edits: Vec<CpmEdit>,
    /// A package no project uses any more: offer to drop its version (central file, id).
    drop_offer: Option<(PathBuf, String)>,
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
            cpm: SolutionCpm::default(),
            cpm_cache: CpmCache::default(),
            cpm_generation: 0,
            cpm_evaluating: false,
            edits: Vec::new(),
            drop_offer: None,
        }
    }

    /// The solution's projects (re-read when the window opens or after a change).
    pub fn set_projects(&mut self, projects: Vec<PathBuf>, dotnet: Option<rc_dotnet::Dotnet>) {
        if let Some(dir) = projects.first().and_then(|p| p.parent()) {
            let root = common_root(&projects).unwrap_or_else(|| dir.to_path_buf());
            self.sources = rc_dotnet::nuget::package_sources(&root).into_iter().filter(|s| s.enabled).collect();
            self.source = self.source.min(self.sources.len().saturating_sub(1));
            self.root = Some(root);
        }
        self.projects = projects;
        self.dotnet = dotnet;
        self.reload_installed();
        self.loaded = true;
    }

    /// Opens on one project (Manage NuGet Packages… in the explorer).
    pub fn focus_project(&mut self, project: &Path) {
        self.checked = std::iter::once(project.to_path_buf()).collect();
        self.tab = Tab::Installed;
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
        let c = NugetClient::with_credentials(&src.url, src.credentials.clone()).ok()?;
        self.clients.insert(src.url.clone(), c.clone());
        Some(c)
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
            let r = client.search(&q, pre, 0, 50).await.map(|r| r.packages).map_err(|e| format!("{e:#}"));
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
        let base = self.root.as_deref().and_then(|r| r.ancestors().find(|a| p.starts_with(a) && a.parent().is_some()));
        base.and_then(|b| p.strip_prefix(b).ok()).unwrap_or(p).display().to_string()
    }

    fn rows(&self) -> Vec<(String, String, String)> {
        // (id, version shown, description)
        match self.tab {
            Tab::Browse => self.results.iter().map(|r| (r.id.clone(), r.version.clone(), r.description.lines().next().unwrap_or("").to_string())).collect(),
            Tab::Central => Vec::new(),
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
    };
    Ok((msg, unsaved))
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

/// The NuGet tool window.
pub fn window(app: &mut DotCraftApp, ui: &mut Ui) {
    if !app.nuget.loaded {
        let projects = app.solution_projects();
        let dotnet = rc_dotnet::Dotnet::locate(Some(&crate::app::dotnet_path(&app.settings)));
        app.nuget.set_projects(projects, dotnet);
    }
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
    let m = &mut app.nuget;
    if m.busy.is_some() || m.searching || m.cpm_evaluating {
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
    let names: Vec<String> = m.sources.iter().map(|s| s.name.clone()).collect();
    let current = names.get(m.source).cloned().unwrap_or_else(|| "nuget.org".into());
    if let Some(i) = kit::dropdown(&mut hu, "nuget_source", &current, &names, true)
        && i != m.source
    {
        m.source = i;
        m.latest.clear();
        m.reload_installed();
    }
    if hu.checkbox(&mut m.prerelease, tr("Prerelease")).changed() {
        m.latest.clear();
        m.reload_installed();
    }
    if theme::icon_button(&mut hu, ico::REFRESH, true, tr("Refresh")).clicked() {
        m.latest.clear();
        m.last_query = None;
        m.cpm_cache.clear();
        m.reload_installed();
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
        ];
        if m.cpm.in_use() {
            tabs.push((Tab::Central, trf("Central issues ({n})", &[("n", &issues)])));
        }
        for (tab, label) in tabs {
            if theme::tab(ui, m.tab == tab, None, &label, false).0.clicked() {
                m.tab = tab;
            }
        }
    });
    ui.add_space(4.0);
    let avail = ui.available_rect_before_wrap();
    let left = Rect::from_min_size(avail.min, vec2((avail.width() * 0.42).clamp(300.0, 520.0), avail.height()));
    let right = Rect::from_min_max(pos2(left.right() + 1.0, avail.top()), avail.max);
    ui.painter().vline(left.right() + 0.5, avail.y_range(), egui::Stroke::new(1.0, p.island_border));
    // Left: search and list.
    let mut lu = ui.new_child(egui::UiBuilder::new().max_rect(left.shrink2(vec2(8.0, 0.0))));
    let hint = if m.tab == Tab::Browse { tr("Search packages") } else { tr("Filter installed packages") };
    if lu.add(egui::TextEdit::singleline(&mut m.query).hint_text(hint).desired_width(f32::INFINITY)).changed() {
        m.edited = std::time::Instant::now();
    }
    lu.add_space(4.0);
    let mut pick = None;
    if m.tab == Tab::Central {
        issue_list(&mut lu, m, &mut pick);
    } else {
        package_list(&mut lu, m, &mut pick);
    }
    if let Some(id) = pick {
        m.select(&id);
    }
    // Right: details and projects.
    let mut ru = ui.new_child(egui::UiBuilder::new().max_rect(right.shrink2(vec2(12.0, 0.0))));
    let open = details(&mut ru, m);
    ui.advance_cursor_after_rect(avail);
    if m.busy.is_none() && !m.ops.is_empty() {
        m.run_next();
    }
    if let Some(f) = open {
        app.open_file(&f);
    }
}

fn package_list(ui: &mut Ui, m: &NugetManager, pick: &mut Option<String>) {
    let p = rc_kit::pal(ui);
    let rows = m.rows();
    egui::ScrollArea::vertical().id_salt("nuget_rows").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        if rows.is_empty() {
            ui.add_space(16.0);
            let msg = match m.tab {
                Tab::Browse if m.searching => tr("Searching…"),
                Tab::Browse => tr("No packages found"),
                Tab::Installed | Tab::Central => tr("The solution has no package references"),
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
                theme::paint_badge(ui, pos2(t.right() + 8.0, r.top() + 14.0), tr("Central"));
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
                        Some(c) => kit::note(ui, &trf("when {condition}", &[("condition", c)])),
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
                    Err(e) => m.message = Some((true, trf("Could not search NuGet: {error}", &[("error", &e)]))),
                }
            }
            Event::Results(..) => {}
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
            Event::Done(r) => {
                m.busy = None;
                changed = true;
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
        let mut app = test_app(home, true);
        app.settings.roslyn_path = Some("/nonexistent".into());
        app.open_workspace(&root.join("CentralShop.slnx"));
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
}
