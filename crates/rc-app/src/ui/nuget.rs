//! NuGet package manager for one project: search nuget.org, pick a version, install, update
//! or remove (through `dotnet add/remove package`).

use crate::app::DotCraftApp;
use egui::{Align2, Context, FontId, Id, Rect, Sense, pos2, vec2};
use rc_core::{Inbox, Runtime, Waker};
use rc_dotnet::{NugetClient, PackageSummary, ProjectInfo};
use rc_kit::i18n::{tr, trf};
use rc_kit::{ico, icons, theme};
use std::path::PathBuf;

enum Event {
    Results(u64, Result<Vec<PackageSummary>, String>),
    Versions(String, Vec<String>),
    Done(Result<String, String>),
}

pub struct NugetDialog {
    pub project: PathBuf,
    query: String,
    prerelease: bool,
    results: Vec<PackageSummary>,
    installed: Vec<(String, String)>,
    selected: Option<String>,
    versions: Vec<String>,
    version: String,
    searching: bool,
    busy: bool,
    message: Option<(bool, String)>,
    generation: u64,
    last_query: Option<(String, bool)>,
    edited: std::time::Instant,
    inbox: Inbox<Event>,
    client: Option<NugetClient>,
    focus: bool,
}

impl NugetDialog {
    pub fn new(project: PathBuf, waker: Waker) -> Self {
        let installed = ProjectInfo::load(&project).map(|p| p.packages.into_iter().map(|p| (p.name, p.version)).collect()).unwrap_or_default();
        NugetDialog {
            project,
            query: String::new(),
            prerelease: false,
            results: Vec::new(),
            installed,
            selected: None,
            versions: Vec::new(),
            version: String::new(),
            searching: false,
            busy: false,
            message: None,
            generation: 0,
            last_query: None,
            edited: std::time::Instant::now(),
            inbox: Inbox::new(waker),
            client: NugetClient::new().ok(),
            focus: true,
        }
    }

    fn search(&mut self) {
        let Some(client) = self.client.clone() else { return };
        self.generation += 1;
        let g = self.generation;
        self.searching = true;
        self.last_query = Some((self.query.clone(), self.prerelease));
        let (q, pre) = (self.query.clone(), self.prerelease);
        let out = self.inbox.sender();
        Runtime::spawn(async move {
            let r = client.search(&q, pre, 0, 40).await.map(|r| r.packages).map_err(|e| e.to_string());
            out.send(Event::Results(g, r));
        });
    }

    fn load_versions(&mut self, id: &str) {
        let Some(client) = self.client.clone() else { return };
        self.versions.clear();
        let out = self.inbox.sender();
        let id = id.to_string();
        Runtime::spawn(async move {
            let v = client.versions(&id).await.unwrap_or_default();
            out.send(Event::Versions(id, v));
        });
    }
}

/// Shows the dialog; returns false when it closed.
pub fn show(app: &mut DotCraftApp, ctx: &Context, d: &mut NugetDialog) -> bool {
    for ev in d.inbox.drain() {
        match ev {
            Event::Results(g, r) if g == d.generation => {
                d.searching = false;
                match r {
                    Ok(list) => d.results = list,
                    Err(e) => d.message = Some((true, trf("Could not search NuGet: {error}", &[("error", &e)]))),
                }
            }
            Event::Results(..) => {}
            Event::Versions(id, mut v) => {
                if d.selected.as_deref() == Some(id.as_str()) {
                    v.reverse();
                    if !d.prerelease {
                        v.retain(|x| !x.contains('-'));
                    }
                    d.version = v.first().cloned().unwrap_or_default();
                    d.versions = v;
                }
            }
            Event::Done(r) => {
                d.busy = false;
                d.installed = ProjectInfo::load(&d.project).map(|p| p.packages.into_iter().map(|p| (p.name, p.version)).collect()).unwrap_or_default();
                app.explorer.invalidate(&d.project);
                d.message = Some(match r {
                    Ok(m) => (false, m),
                    Err(e) => (true, e),
                });
            }
        }
    }
    // Search as you type (debounced), or show the installed packages.
    if d.last_query.as_ref() != Some(&(d.query.clone(), d.prerelease)) && d.edited.elapsed().as_millis() > 300 {
        if d.query.trim().is_empty() {
            d.results.clear();
            d.last_query = Some((d.query.clone(), d.prerelease));
        } else {
            d.search();
        }
    }
    if d.searching || d.busy || d.last_query.as_ref() != Some(&(d.query.clone(), d.prerelease)) {
        ctx.request_repaint_after(std::time::Duration::from_millis(150));
    }
    let p = rc_kit::theme::palette(ctx.global_style().visuals.dark_mode);
    let name = d.project.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut open = true;
    let mut install: Option<(String, Option<String>)> = None;
    let mut remove: Option<String> = None;
    let resp = egui::Modal::new(Id::new("nuget")).show(ctx, |ui| {
        ui.set_width(820.0);
        ui.set_height(520.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(trf("NuGet Packages — {project}", &[("project", &name)])).font(theme::bold(15.0)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::icon_button(ui, ico::CLOSE_SMALL, true, tr("Close")).clicked() {
                    open = false;
                }
            });
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let te = ui.add(egui::TextEdit::singleline(&mut d.query).hint_text(tr("Search packages on nuget.org")).desired_width(440.0));
            if std::mem::take(&mut d.focus) {
                te.request_focus();
            }
            if te.changed() {
                d.edited = std::time::Instant::now();
            }
            ui.checkbox(&mut d.prerelease, tr("Prerelease"));
            if d.searching {
                ui.add(egui::Spinner::new().size(14.0));
            }
        });
        ui.add_space(6.0);
        let avail = ui.available_rect_before_wrap();
        let left = Rect::from_min_size(avail.min, vec2(460.0, avail.height()));
        let right = Rect::from_min_max(pos2(left.right() + 12.0, avail.top()), avail.max);
        let mut lu = ui.new_child(egui::UiBuilder::new().max_rect(left));
        egui::ScrollArea::vertical().id_salt("nuget_list").auto_shrink([false, false]).show(&mut lu, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let rows: Vec<(String, String, String, bool)> = if d.query.trim().is_empty() {
                d.installed.iter().map(|(n, v)| (n.clone(), v.clone(), tr("Installed").to_string(), true)).collect()
            } else {
                d.results.iter().map(|r| (r.id.clone(), r.version.clone(), r.description.lines().next().unwrap_or("").to_string(), d.installed.iter().any(|(n, _)| n.eq_ignore_ascii_case(&r.id)))).collect()
            };
            if rows.is_empty() && !d.searching {
                ui.add_space(20.0);
                let msg = if d.query.trim().is_empty() { tr("No packages installed. Type to search nuget.org.") } else { tr("No packages found") };
                ui.label(egui::RichText::new(msg).color(p.muted));
            }
            for (id, version, desc, installed) in rows {
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::click());
                if d.selected.as_deref() == Some(id.as_str()) {
                    ui.painter().rect_filled(r, 6.0, p.selection);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, 6.0, p.row_hover);
                }
                icons::paint(ui.painter(), Rect::from_center_size(pos2(r.left() + 16.0, r.top() + 15.0), vec2(16.0, 16.0)), ico::PACKAGE, &p, true);
                ui.painter().text(pos2(r.left() + 32.0, r.top() + 15.0), Align2::LEFT_CENTER, &id, theme::bold(13.0), p.text);
                ui.painter().text(pos2(r.right() - 8.0, r.top() + 15.0), Align2::RIGHT_CENTER, &version, FontId::proportional(12.0), if installed { p.ok } else { p.muted });
                let mut job = egui::text::LayoutJob::simple_singleline(desc, FontId::proportional(12.0), p.muted);
                job.wrap.max_width = r.width() - 40.0;
                job.wrap.max_rows = 1;
                job.wrap.overflow_character = Some('…');
                let g = ui.painter().layout_job(job);
                ui.painter().galley(pos2(r.left() + 32.0, r.top() + 27.0), g, p.muted);
                if resp.clicked() {
                    d.selected = Some(id.clone());
                    d.load_versions(&id);
                }
            }
        });
        let mut ru = ui.new_child(egui::UiBuilder::new().max_rect(right));
        if let Some(id) = d.selected.clone() {
            ru.label(egui::RichText::new(&id).font(theme::bold(14.0)));
            if let Some(r) = d.results.iter().find(|r| r.id == id) {
                ru.label(egui::RichText::new(&r.description).size(12.5));
                ru.label(egui::RichText::new(trf("{n} downloads", &[("n", &r.total_downloads)])).size(12.0).color(p.muted));
            }
            ru.add_space(10.0);
            let current = d.installed.iter().find(|(n, _)| n.eq_ignore_ascii_case(&id)).map(|(_, v)| v.clone());
            if let Some(v) = &current {
                ru.label(trf("Installed version: {version}", &[("version", v)]));
            }
            ru.horizontal(|ui| {
                ui.label(tr("Version"));
                egui::ComboBox::from_id_salt("nuget_version").selected_text(if d.version.is_empty() { tr("Latest").to_string() } else { d.version.clone() }).show_ui(ui, |ui| {
                    for v in d.versions.iter().take(60) {
                        ui.selectable_value(&mut d.version, v.clone(), v);
                    }
                });
            });
            ru.add_space(6.0);
            ru.horizontal(|ui| {
                let label = if current.is_some() { tr("Update") } else { tr("Install") };
                if ui.add_enabled_ui(!d.busy, |ui| theme::primary_button(ui, label)).inner.clicked() {
                    install = Some((id.clone(), (!d.version.is_empty()).then(|| d.version.clone())));
                }
                if current.is_some() && ui.add_enabled_ui(!d.busy, |ui| theme::secondary_button(ui, tr("Remove"))).inner.clicked() {
                    remove = Some(id.clone());
                }
                if d.busy {
                    ui.add(egui::Spinner::new().size(14.0));
                }
            });
        } else {
            ru.add_space(20.0);
            ru.label(egui::RichText::new(tr("Select a package to see its versions")).color(p.muted));
        }
        if let Some((err, m)) = &d.message {
            ru.add_space(8.0);
            ru.label(egui::RichText::new(m).size(12.5).color(if *err { p.error } else { p.ok }));
        }
        ui.advance_cursor_after_rect(avail);
    });
    if resp.should_close() {
        open = false;
    }
    let dotnet = rc_dotnet::Dotnet::locate(Some(&crate::app::dotnet_path(&app.settings)));
    if let (Some((id, version)), Some(dotnet)) = (install, dotnet.clone()) {
        d.busy = true;
        d.message = None;
        let (out, project, pre) = (d.inbox.sender(), d.project.clone(), d.prerelease);
        Runtime::spawn(async move {
            let r = rc_dotnet::nuget::add_package(&dotnet, &project, &id, version.as_deref(), pre).await;
            out.send(Event::Done(match r {
                Ok(o) if o.success => Ok(trf("{id} installed", &[("id", &id)])),
                Ok(o) => Err(o.output.lines().rev().find(|l| l.contains("error")).unwrap_or("dotnet add package failed").to_string()),
                Err(e) => Err(e.to_string()),
            }));
        });
    }
    if let (Some(id), Some(dotnet)) = (remove, dotnet) {
        d.busy = true;
        d.message = None;
        let (out, project) = (d.inbox.sender(), d.project.clone());
        Runtime::spawn(async move {
            let r = rc_dotnet::nuget::remove_package(&dotnet, &project, &id).await;
            out.send(Event::Done(match r {
                Ok(o) if o.success => Ok(trf("{id} removed", &[("id", &id)])),
                Ok(o) => Err(o.output.trim().to_string()),
                Err(e) => Err(e.to_string()),
            }));
        });
    }
    open
}
