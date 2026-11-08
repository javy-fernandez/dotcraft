//! Workspace-owned templates, favorites, and bounded related-file navigation.
use crate::app::DotCraftApp;
use crate::ui::kit;
use egui::{FontId, Rect, Sense, pos2, vec2};
use rc_kit::theme;
use rc_kit::i18n::tr;
use std::{collections::BTreeMap, path::{Path, PathBuf}, sync::{Arc, atomic::{AtomicBool, Ordering}}};

#[derive(Default)]
pub struct EditorWorkflows {
    pub visible: bool,
    pub templates: Arc<BTreeMap<String, String>>,
    pub favorites: Vec<PathBuf>,
    key: String,
    body: String,
    error: Option<String>,
    related: Vec<PathBuf>,
    pending: Option<rc_core::Inbox<Vec<PathBuf>>>,
    cancel: Option<Arc<AtomicBool>>,
}

impl Drop for EditorWorkflows {
    fn drop(&mut self) { self.cancel_related(); }
}

impl EditorWorkflows {
    fn cancel_related(&mut self) {
        if let Some(stop) = self.cancel.take() { stop.store(true, Ordering::Relaxed); }
        self.pending = None;
    }
    fn find_related(&mut self, snapshot: Arc<rc_search::IndexSnapshot>, file: PathBuf, waker: rc_core::Waker) {
        self.cancel_related();
        self.related.clear();
        let inbox = rc_core::Inbox::new(waker);
        let out = inbox.sender();
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        std::thread::spawn(move || {
            let mut found = Vec::new();
            let key = related_key(&file);
            for item in snapshot.files() {
                if stop.load(Ordering::Relaxed) { return; }
                let path = snapshot.abs_path(item);
                if path != file && related_key(&path) == key {
                    found.push(path);
                    if found.len() == 200 { break; }
                }
            }
            out.send(found);
        });
        self.pending = Some(inbox);
        self.cancel = Some(cancel);
    }
    fn poll(&mut self) {
        if let Some(inbox) = &self.pending {
            let results = inbox.drain();
            if let Some(paths) = results.into_iter().last() {
                self.related = paths;
                self.pending = None;
                self.cancel = None;
            }
        }
    }
    fn save_template(&mut self) -> bool {
        self.error = None;
        if self.key.is_empty() || self.key.len() > 32 || !self.key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || self.key.starts_with(|c: char| c.is_ascii_digit()) {
            self.error = Some(tr("Use a template abbreviation of 1–32 letters, digits, or underscores.").into()); return false;
        }
        if self.body.trim().is_empty() || self.body.len() > 16 * 1024 || (self.templates.len() >= 100 && !self.templates.contains_key(&self.key)) {
            self.error = Some(tr("Templates require a body up to 16 KiB; at most 100 templates are allowed.").into()); return false;
        }
        Arc::make_mut(&mut self.templates).insert(self.key.clone(), self.body.clone());
        true
    }
}

/// Filename conventions only: no semantic claim or disk reads.
fn related_key(file: &Path) -> String {
    let name = file.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
    let mut stem = name.as_str();
    for suffix in [".cs", ".razor", ".cshtml", ".xaml", ".designer", ".tests", ".test"] {
        if let Some(rest) = stem.strip_suffix(suffix) { stem = rest; }
    }
    stem.strip_suffix("tests").unwrap_or(stem).to_string()
}

/// A file row: file icon and name, grey folder; reports the full path to AccessKit.
fn path_row(ui: &mut egui::Ui, path: &Path, width: f32) -> egui::Response {
    let p = rc_kit::pal(ui);
    let (r, resp) = ui.allocate_exact_size(vec2(width, 24.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r.shrink2(vec2(6.0, 0.0)), 4.0, p.row_hover);
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    rc_kit::icons::paint(ui.painter(), Rect::from_center_size(pos2(r.left() + 20.0, r.center().y), vec2(16.0, 16.0)), rc_kit::icons::for_file(&name), &p, true);
    let g = ui.painter().layout_no_wrap(name, FontId::proportional(13.0), p.text);
    ui.painter().galley(pos2(r.left() + 36.0, (r.center().y - g.size().y / 2.0).round()), g.clone(), p.text);
    let x = r.left() + 44.0 + g.size().x;
    let folder = path.parent().map(|d| d.display().to_string()).unwrap_or_default();
    let mut job = egui::text::LayoutJob::simple_singleline(folder, FontId::proportional(12.0), p.muted);
    job.wrap.max_width = (r.right() - x - 8.0).max(1.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    let f = ui.painter().layout_job(job);
    ui.painter().galley(pos2(x, r.center().y - f.size().y / 2.0), f, p.muted);
    let label = path.display().to_string();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
    resp.on_hover_text(label)
}

/// A section whose open state lives in egui memory.
fn section(ui: &mut egui::Ui, icon: rc_kit::Ico, title: &str, secondary: &str, default_open: bool) -> bool {
    let id = egui::Id::new(("editor_workflows_section", title));
    let mut open = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(default_open);
    kit::section(ui, icon, title, secondary, &mut open);
    ui.data_mut(|d| d.insert_temp(id, open));
    open
}

pub fn show(app: &mut DotCraftApp, ctx: &egui::Context) {
    if !app.editor_workflows.visible { return; }
    let mut state = std::mem::take(&mut app.editor_workflows);
    state.poll();
    let mut open = true;
    let mut chosen = None;
    let mut changed = false;
    // DotCraft's header (title, Close) instead of egui's title bar.
    let frame = egui::Frame::window(&ctx.global_style()).inner_margin(egui::Margin::same(4));
    egui::Window::new(tr("Editor Workflows")).id(egui::Id::new("editor_workflows")).title_bar(false).frame(frame)
        .default_width(580.0).max_width((ctx.content_rect().width()-32.0).max(240.0))
        .max_height((ctx.content_rect().height()-48.0).max(180.0)).show(ctx, |ui| {
        if kit::header(ui, tr("Editor Workflows"), &[], "", &[(rc_kit::ico::CLOSE, tr("Close"), true)]).action == Some(0) { open = false; }
        egui::ScrollArea::vertical().id_salt("editor_workflows_scroll").auto_shrink([false, true]).show(ui, |ui| {
            let active = app.docs.active_doc().map(|d| d.path.clone());
            if section(ui, rc_kit::ico::TEMPLATE, tr("Workspace Templates"), &state.templates.len().to_string(), true) {
                kit::content(ui, |ui| {
                    kit::note(ui, tr("Save C# snippets here, then type their abbreviation and press Tab. Postfix: .if, .not, .null, .notnull, .return, .await."));
                    ui.add_space(4.0);
                    kit::field(ui, tr("Abbreviation"), &mut state.key, "", 180.0, true);
                    ui.add_space(2.0);
                    ui.add(egui::TextEdit::multiline(&mut state.body).code_editor().char_limit(16384).desired_rows(5).desired_width(f32::INFINITY).margin(vec2(6.0, 4.0)));
                    ui.add_space(2.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        if kit::button(ui, Some(rc_kit::ico::SAVE), tr("Save Template"), app.workspace.is_some()).clicked() && app.workspace.is_some() { changed |= state.save_template(); }
                        if kit::button(ui, Some(rc_kit::ico::DELETE), tr("Delete Template"), true).clicked() { changed |= Arc::make_mut(&mut state.templates).remove(&state.key).is_some(); }
                        let editable = app.docs.active_doc().is_some_and(|d| !d.editor.read_only && d.editor.lang() == rc_syntax::Lang::CSharp);
                        if kit::button(ui, Some(rc_kit::ico::ADD), tr("Insert Template"), editable).clicked() && editable && let Some(doc) = app.docs.active_doc_mut() {
                            let caret = doc.editor.caret();
                            let stem = doc.path.file_stem().unwrap_or_default().to_string_lossy();
                            doc.editor.insert_snippet(caret, caret, &rc_editor::templates::fill(&state.body, &stem));
                        }
                    });
                    if let Some(error) = &state.error { kit::message(ui, error, true); }
                });
                if section(ui, rc_kit::ico::PREVIEW, tr("Snippet Preview"), "", false) {
                    kit::content(ui, |ui| {
                        let parsed = rc_editor::snippet::parse(&rc_editor::templates::fill(&state.body, "Example"));
                        ui.label(egui::RichText::new(parsed.text).monospace().size(12.5));
                    });
                }
                ui.spacing_mut().item_spacing.y = 0.0;
                for (key, body) in state.templates.iter() {
                    let first = body.lines().next().unwrap_or_default();
                    if kit::row(ui, 1, Some(rc_kit::ico::TEMPLATE), key, first, state.key == *key).clicked() { state.key = key.clone(); state.body = body.clone(); }
                }
                ui.spacing_mut().item_spacing.y = 4.0;
            }
            if section(ui, rc_kit::ico::PIN, tr("Favorite Files"), &state.favorites.len().to_string(), true) {
                kit::toolbar(ui, |ui| {
                    let can = active.is_some() && app.workspace.is_some();
                    if kit::button(ui, Some(rc_kit::ico::PIN), tr("Toggle Current Favorite"), can).clicked() && can && let Some(path) = active.clone() {
                        if state.favorites.contains(&path) { state.favorites.retain(|p| p != &path); }
                        else if state.favorites.len() < 500 { state.favorites.push(path); }
                        changed = true;
                    }
                });
                let mut remove = None;
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                for path in &state.favorites {
                    ui.horizontal(|ui| {
                        if path_row(ui, path, ui.available_width() - 32.0).clicked() { chosen = Some(path.clone()); }
                        if theme::icon_button(ui, rc_kit::ico::CLOSE_SMALL, true, tr("Remove")).clicked() { remove = Some(path.clone()); }
                    });
                }
                ui.spacing_mut().item_spacing = vec2(8.0, 4.0);
                if let Some(path) = remove { state.favorites.retain(|p| p != &path); changed = true; }
            }
            if section(ui, rc_kit::ico::TW_HIERARCHY, tr("Related Files"), "", true) {
                kit::content(ui, |ui| kit::note(ui, tr("Find matching C#, Razor, XAML, designer, and test filenames in the workspace index.")));
                kit::toolbar(ui, |ui| {
                    let can = active.is_some() && app.files.index.is_some();
                    if kit::button(ui, Some(rc_kit::ico::SEARCH), tr("Find Related Files"), can).clicked() && can
                        && let (Some(index), Some(file)) = (&app.files.index, active.clone()) {
                            state.find_related(index.snapshot(), file, app.waker.clone());
                        }
                    if state.pending.is_some() {
                        ui.add(egui::Spinner::new().size(14.0));
                        if kit::button(ui, Some(rc_kit::ico::STOP), tr("Cancel"), true).clicked() { state.cancel_related(); }
                    }
                });
                ui.spacing_mut().item_spacing.y = 0.0;
                for path in &state.related {
                    if path_row(ui, path, ui.available_width()).clicked() { chosen = Some(path.clone()); }
                }
            }
        });
    });
    state.visible = open;
    if !open { state.cancel_related(); }
    app.editor_workflows = state;
    if changed { app.session_dirty = true; }
    if let Some(path) = chosen { app.open_file(&path); }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workspace_template_and_favorite_controls_round_trip_through_sessions() {
        use egui_kittest::{Harness, kittest::Queryable};
        let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = tempfile::tempdir().unwrap();
        let file = home.path().canonicalize().unwrap().join("Order.cs");
        std::fs::write(&file, "class Order {}\n").unwrap();
        let mut app = crate::screens::test_app(home.path(), true);
        app.workspace = Some(crate::app::WorkspaceState { ws: rc_core::Workspace::open(home.path()).unwrap(), solution: None });
        app.open_file(&file);
        app.editor_workflows.visible = true;
        app.editor_workflows.key = "greet".into();
        app.editor_workflows.body = "Console.WriteLine(${1:name});$0".into();
        let mut first = true;
        let mut h = Harness::builder().with_size(egui::vec2(900.0, 800.0)).build_ui_state(move |ui, app| {
            if std::mem::take(&mut first) { rc_kit::theme::install_fonts(ui.ctx()); return; }
            show(app, ui.ctx());
        }, app);
        h.run_steps(3);
        h.get_by_label(tr("Save Template")).click(); h.run_steps(3);
        h.get_by_label(tr("Toggle Current Favorite")).click(); h.run_steps(3);
        let session = h.state().session();
        assert_eq!(session.custom_templates.get("greet").unwrap(), "Console.WriteLine(${1:name});$0");
        assert_eq!(session.favorites, vec![file.clone()]);
        session.save(home.path()).unwrap();
        let restored = rc_core::Session::load(home.path());
        assert_eq!(restored.custom_templates, session.custom_templates);
        assert_eq!(restored.favorites, session.favorites);
        h.get_by_label(tr("Toggle Current Favorite")).click(); h.run_steps(2);
        assert!(h.state().session().favorites.is_empty());
        let related = home.path().canonicalize().unwrap().join("Order.razor");
        std::fs::write(&related, "<h1>Order</h1>").unwrap();
        h.state_mut().editor_workflows.related = vec![related.clone()];
        h.run_steps(2);
        h.get_by_label(&related.display().to_string()).scroll_to_me(); h.run_steps(8);
        h.get_by_label(&related.display().to_string()).click(); h.run_steps(3);
        assert_eq!(h.state().docs.active_doc().unwrap().path, related);

    }

    #[test]
    fn related_worker_filters_index_and_discards_cancelled_results() {
        let root = Path::new("/workspace");
        let snapshot = Arc::new(rc_search::IndexSnapshot::from_rel_paths(root, ["Order.cs", "OrderTests.cs", "Order.razor.cs", "Orders.cs"]));
        let mut state = EditorWorkflows::default();
        state.find_related(snapshot.clone(), root.join("Order.cs"), rc_core::Waker::noop());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while state.pending.is_some() && std::time::Instant::now() < deadline { state.poll(); std::thread::sleep(std::time::Duration::from_millis(5)); }
        assert_eq!(state.related, vec![root.join("Order.razor.cs"), root.join("OrderTests.cs")]);
        state.find_related(snapshot, root.join("Orders.cs"), rc_core::Waker::noop());
        state.cancel_related(); state.poll();
        assert!(state.related.is_empty());
    }

    #[test]
    fn related_filename_families_and_template_bounds() {
        for name in ["Order.cs", "Order.razor", "Order.razor.cs", "Order.xaml.cs", "Order.Designer.cs", "OrderTests.cs", "Order.Tests.cs"] { assert_eq!(related_key(Path::new(name)), "order", "{name}"); }
        assert_ne!(related_key(Path::new("Orders.cs")), "order");
        let mut state = EditorWorkflows::default();
        state.key = "myprop".into(); state.body = "public ${1:int} ${2:Value} { get; set; }$0".into();
        assert!(state.save_template());
        state.key = "invalid key".into(); assert!(!state.save_template());
        assert_eq!(state.templates.len(), 1);
    }
}
