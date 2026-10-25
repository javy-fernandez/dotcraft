//! NuGet tool window for the whole solution (Rider's NuGet window): browse a source, the
//! installed packages, the ones with updates and the ones to consolidate; a package's details
//! and, per project, its installed version with Install / Update / Remove.

use crate::app::DotCraftApp;
use crate::toolwindow::ToolWindow;
use egui::{Align2, FontId, Rect, Sense, Ui, pos2, vec2};
use rc_core::{Inbox, Runtime, Waker};
use rc_dotnet::nuget::{PackageSource, compare_versions};
use rc_dotnet::{NugetClient, PackageSummary};
use rc_kit::i18n::{tr, trf};
use rc_kit::{ico, icons, theme};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Browse,
    Installed,
    Updates,
    Consolidate,
}

enum Event {
    Results(u64, Result<Vec<PackageSummary>, String>),
    Versions(String, Vec<String>),
    Latest(String, Option<String>),
    Details(String, Option<PackageSummary>),
    Done(Result<String, String>),
}

/// A pending change: (project, package, version or `None` to remove).
type Op = (PathBuf, String, Option<String>);

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
        }
    }

    /// The solution's projects (re-read when the window opens or after a change).
    pub fn set_projects(&mut self, projects: Vec<PathBuf>, dotnet: Option<rc_dotnet::Dotnet>) {
        if let Some(dir) = projects.first().and_then(|p| p.parent()) {
            let root = common_root(&projects).unwrap_or_else(|| dir.to_path_buf());
            self.sources = rc_dotnet::nuget::package_sources(&root).into_iter().filter(|s| s.enabled).collect();
            self.source = self.source.min(self.sources.len().saturating_sub(1));
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
        let mut installed: BTreeMap<String, Vec<(PathBuf, String)>> = BTreeMap::new();
        for p in &self.projects {
            for r in rc_dotnet::nuget::installed_packages(p) {
                installed.entry(r.name.clone()).or_default().push((p.clone(), r.version));
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

    /// Runs the queued changes one after the other (`dotnet add/remove package`).
    fn run_next(&mut self) {
        let (Some(dotnet), false) = (self.dotnet.clone(), self.ops.is_empty()) else { return };
        let (project, id, version) = self.ops.remove(0);
        let what = match &version {
            Some(v) => trf("Installing {id} {version} in {project}…", &[("id", &id), ("version", v), ("project", &project_name(&project))]),
            None => trf("Removing {id} from {project}…", &[("id", &id), ("project", &project_name(&project))]),
        };
        self.busy = Some(what);
        let (out, pre) = (self.inbox.sender(), self.prerelease);
        Runtime::spawn(async move {
            let r = match &version {
                Some(v) => rc_dotnet::nuget::add_package(&dotnet, &project, &id, Some(v), pre).await,
                None => rc_dotnet::nuget::remove_package(&dotnet, &project, &id).await,
            };
            out.send(Event::Done(match r {
                Ok(o) if o.success => Ok(id),
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

    fn installed_versions(&self, id: &str) -> Vec<(PathBuf, String)> {
        self.installed.iter().find(|(k, _)| k.eq_ignore_ascii_case(id)).map(|(_, v)| v.clone()).unwrap_or_default()
    }

    fn has_update(&self, id: &str) -> Option<String> {
        let latest = self.latest.get(&id.to_lowercase())?;
        let newest_installed = self.installed_versions(id).into_iter().map(|(_, v)| v).max_by(|a, b| compare_versions(a, b))?;
        (compare_versions(latest, &newest_installed).is_gt()).then(|| latest.clone())
    }

    fn rows(&self) -> Vec<(String, String, String)> {
        // (id, version shown, description)
        match self.tab {
            Tab::Browse => self.results.iter().map(|r| (r.id.clone(), r.version.clone(), r.description.lines().next().unwrap_or("").to_string())).collect(),
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
    let changed = poll(&mut app.nuget);
    if changed {
        for p in app.nuget.projects.clone() {
            app.explorer.invalidate(&p);
        }
    }
    let m = &mut app.nuget;
    if m.busy.is_some() || m.searching {
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
    egui::ComboBox::from_id_salt("nuget_source").selected_text(current).width(200.0).show_ui(&mut hu, |ui| {
        for (i, n) in names.iter().enumerate() {
            if ui.selectable_value(&mut m.source, i, n).changed() {
                m.latest.clear();
                m.reload_installed();
            }
        }
    });
    if hu.checkbox(&mut m.prerelease, tr("Prerelease")).changed() {
        m.latest.clear();
        m.reload_installed();
    }
    if theme::icon_button(&mut hu, ico::REFRESH, true, tr("Refresh")).clicked() {
        m.latest.clear();
        m.last_query = None;
        m.reload_installed();
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
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        for (tab, label) in [
            (Tab::Browse, tr("Browse").to_string()),
            (Tab::Installed, trf("Installed ({n})", &[("n", &m.installed.len())])),
            (Tab::Updates, trf("Updates ({n})", &[("n", &updates)])),
            (Tab::Consolidate, trf("Consolidate ({n})", &[("n", &consolidate)])),
        ] {
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
    let rows = m.rows();
    let mut pick = None;
    egui::ScrollArea::vertical().id_salt("nuget_rows").auto_shrink([false, false]).show(&mut lu, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        if rows.is_empty() {
            ui.add_space(16.0);
            let msg = match m.tab {
                Tab::Browse if m.searching => tr("Searching…"),
                Tab::Browse => tr("No packages found"),
                Tab::Installed => tr("The solution has no package references"),
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
            ui.painter().text(pos2(r.left() + 32.0, r.top() + 14.0), Align2::LEFT_CENTER, id, theme::bold(13.0), p.text);
            let installed = !m.installed_versions(id).is_empty();
            ui.painter().text(pos2(r.right() - 8.0, r.top() + 14.0), Align2::RIGHT_CENTER, version, FontId::proportional(12.0), if installed { p.ok } else { p.muted });
            let mut job = egui::text::LayoutJob::simple_singleline(desc.clone(), FontId::proportional(12.0), p.muted);
            job.wrap.max_width = r.width() - 40.0;
            job.wrap.max_rows = 1;
            job.wrap.overflow_character = Some('…');
            let g = ui.painter().layout_job(job);
            ui.painter().galley(pos2(r.left() + 32.0, r.top() + 25.0), g, p.muted);
            if resp.clicked() {
                pick = Some(id.clone());
            }
        }
    });
    if let Some(id) = pick {
        m.select(&id);
    }
    // Right: details and projects.
    let mut ru = ui.new_child(egui::UiBuilder::new().max_rect(right.shrink2(vec2(12.0, 0.0))));
    details(&mut ru, m);
    ui.advance_cursor_after_rect(avail);
    if m.busy.is_none() && !m.ops.is_empty() {
        m.run_next();
    }
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
            Event::Done(r) => {
                m.busy = None;
                changed = true;
                m.message = Some(match r {
                    Ok(id) => (false, trf("{id}: done", &[("id", &id)])),
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

fn details(ui: &mut Ui, m: &mut NugetManager) {
    let p = rc_kit::pal(ui);
    let Some(id) = m.selected.clone() else {
        ui.add_space(20.0);
        ui.label(egui::RichText::new(tr("Select a package to see its details and the projects that use it")).color(p.muted));
        if let Some((err, msg)) = &m.message {
            ui.label(egui::RichText::new(msg).size(12.5).color(if *err { p.error } else { p.ok }));
        }
        return;
    };
    egui::ScrollArea::vertical().id_salt("nuget_details").auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal(|ui| {
            icons::image(ui, ico::PACKAGE, 16.0);
            ui.label(egui::RichText::new(&id).font(theme::bold(15.0)));
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
            ui.label(tr("Version"));
            egui::ComboBox::from_id_salt("nuget_version").selected_text(if m.version.is_empty() { tr("Loading…").to_string() } else { m.version.clone() }).show_ui(ui, |ui| {
                for v in m.versions.iter().take(80) {
                    ui.selectable_value(&mut m.version, v.clone(), v);
                }
            });
        });
        ui.add_space(8.0);
        // Projects: installed version and actions.
        let installed = m.installed_versions(&id);
        let mut ops: Vec<Op> = Vec::new();
        egui::Grid::new("nuget_projects").num_columns(4).spacing([14.0, 6.0]).striped(true).show(ui, |ui| {
            ui.label("");
            ui.label(egui::RichText::new(tr("Project")).strong());
            ui.label(egui::RichText::new(tr("Installed")).strong());
            ui.label("");
            ui.end_row();
            for project in m.projects.clone() {
                let current = installed.iter().find(|(p, _)| *p == project).map(|(_, v)| v.clone());
                let mut on = m.checked.contains(&project);
                if ui.checkbox(&mut on, "").changed() {
                    if on {
                        m.checked.insert(project.clone());
                    } else {
                        m.checked.remove(&project);
                    }
                }
                ui.label(project_name(&project));
                ui.label(egui::RichText::new(current.clone().unwrap_or_else(|| "—".into())).color(if current.is_some() { p.ok } else { p.muted }));
                ui.horizontal(|ui| {
                    let busy = m.busy.is_some() || m.version.is_empty();
                    match &current {
                        Some(v) if *v != m.version => {
                            if ui.add_enabled(!busy, egui::Button::new(tr("Update"))).clicked() {
                                ops.push((project.clone(), id.clone(), Some(m.version.clone())));
                            }
                        }
                        Some(_) => {}
                        None => {
                            if ui.add_enabled(!busy, egui::Button::new(tr("Install"))).clicked() {
                                ops.push((project.clone(), id.clone(), Some(m.version.clone())));
                            }
                        }
                    }
                    if current.is_some() && ui.add_enabled(m.busy.is_none(), egui::Button::new(tr("Remove"))).clicked() {
                        ops.push((project.clone(), id.clone(), None));
                    }
                });
                ui.end_row();
            }
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let ready = m.busy.is_none() && !m.version.is_empty() && !m.checked.is_empty();
            if ui.add_enabled_ui(ready, |ui| theme::primary_button(ui, &trf("Install or update in {n} selected", &[("n", &m.checked.len())]))).inner.clicked() {
                for project in &m.checked {
                    ops.push((project.clone(), id.clone(), Some(m.version.clone())));
                }
            }
            let any_installed = m.checked.iter().any(|c| installed.iter().any(|(p, _)| p == c));
            if ui.add_enabled_ui(m.busy.is_none() && any_installed, |ui| theme::secondary_button(ui, tr("Remove from selected"))).inner.clicked() {
                for project in m.checked.iter().filter(|c| installed.iter().any(|(p, _)| p == *c)) {
                    ops.push((project.clone(), id.clone(), None));
                }
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
        m.ops.extend(ops);
    });
}
