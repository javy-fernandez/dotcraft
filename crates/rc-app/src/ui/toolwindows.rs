//! Contents of the tool windows. Each window has the New UI header (title on the left,
//! actions and "hide" on the right) and an empty state that says what will show up there.

use crate::app::DotCraftApp;
use crate::toolwindow::ToolWindow;
use crate::ui::explorer;
use egui::{Align2, FontId, Id, Rect, Sense, Ui, pos2, vec2};
use rc_kit::i18n::tr;
use rc_kit::{Ico, icons, theme};

pub const HEADER_H: f32 = 36.0;

pub fn show(app: &mut DotCraftApp, ui: &mut Ui, tw: ToolWindow) {
    match tw {
        ToolWindow::Explorer => {
            let Some(w) = &app.workspace else { return };
            let (root, name) = (w.ws.root.clone(), w.ws.name());
            let active = app.docs.active_doc().map(|d| d.path.clone());
            let actions = explorer::show(ui, &mut app.explorer, w.solution.as_ref(), &root, &name, active.as_deref());
            if ui.ctx().data_mut(|d| d.remove_temp::<bool>(Id::new("explorer_hide")).unwrap_or(false)) {
                app.toggle_tool(ToolWindow::Explorer);
            }
            if !actions.is_empty() {
                app.session_dirty = true;
            }
            let ctx = ui.ctx().clone();
            app.handle_explorer(&ctx, actions);
        }
        ToolWindow::Problems => {
            if header(ui, tw.title()) {
                app.toggle_tool(tw);
            }
            problems(app, ui);
        }
        ToolWindow::Structure => {
            if header(ui, tw.title()) {
                app.toggle_tool(tw);
            }
            structure(app, ui);
        }
        ToolWindow::Find if app.find_results.is_some() => {
            let title = app.find_results.as_ref().map(|(t, r)| format!("{t} ({})", r.len())).unwrap_or_default();
            if header(ui, &title) {
                app.toggle_tool(tw);
            }
            find_results(app, ui);
        }
        _ => {
            if header(ui, tw.title()) {
                app.toggle_tool(tw);
            }
            let (title, hint) = empty_text(tw);
            empty_state(ui, tw.icon(), title, hint);
        }
    }
}

/// A clickable list row: icon, main text, grey secondary text.
fn list_row(ui: &mut Ui, depth: usize, icon: Ico, text: &str, secondary: &str, selected: bool) -> egui::Response {
    let p = rc_kit::pal(ui);
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(240.0), 24.0), Sense::click());
    if selected {
        ui.painter().rect_filled(r.shrink2(vec2(6.0, 0.0)), 4.0, p.selection);
    } else if resp.hovered() {
        ui.painter().rect_filled(r.shrink2(vec2(6.0, 0.0)), 4.0, p.row_hover);
    }
    let x = r.left() + 12.0 + depth as f32 * 18.0;
    icons::paint(ui.painter(), Rect::from_center_size(pos2(x + 8.0, r.center().y), vec2(16.0, 16.0)), icon, &p, true);
    let g = ui.painter().layout_no_wrap(text.to_string(), FontId::proportional(13.0), p.text);
    ui.painter().galley(pos2(x + 24.0, (r.center().y - g.size().y / 2.0).round()), g.clone(), p.text);
    if !secondary.is_empty() {
        ui.painter().text(pos2(x + 32.0 + g.size().x, r.center().y), Align2::LEFT_CENTER, secondary, FontId::proportional(12.0), p.muted);
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, text));
    resp
}

fn problems(app: &mut DotCraftApp, ui: &mut Ui) {
    if app.insight.problems.is_empty() {
        let (title, hint) = empty_text(ToolWindow::Problems);
        empty_state(ui, ToolWindow::Problems.icon(), title, hint);
        return;
    }
    let root = app.workspace.as_ref().map(|w| w.ws.root.clone());
    let mut open: Option<(std::path::PathBuf, usize)> = None;
    let entries: Vec<(std::path::PathBuf, Vec<rc_editor::Diagnostic>)> = app.insight.problems.iter().map(|(p, d)| (p.clone(), d.clone())).collect();
    egui::ScrollArea::vertical().id_salt("problems").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for (path, diags) in &entries {
            let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            let dir = root.as_ref().and_then(|r| path.parent()?.strip_prefix(r).ok()).map(|d| d.display().to_string()).unwrap_or_default();
            let count = rc_kit::i18n::trf("{n} problems", &[("n", &diags.len())]);
            list_row(ui, 0, rc_kit::icons::for_file(&name), &name, &format!("{dir}  {count}"), false);
            for d in diags {
                let icon = if d.severity == rc_editor::Severity::Error { rc_kit::ico::ERROR } else { rc_kit::ico::WARNING };
                // The line:column is read from the open editor or the file.
                let (line, col) = app.docs.list.iter().find(|x| &x.path == path).map(|x| x.editor.buffer.line_col(d.start)).unwrap_or((0, 0));
                let code = d.code.clone().unwrap_or_default();
                let resp = list_row(ui, 1, icon, &d.message, &format!("{code} :{}", line + 1), false);
                if resp.double_clicked() || resp.clicked() {
                    open = Some((path.clone(), d.start));
                }
                let _ = col;
            }
        }
    });
    if let Some((path, pos)) = open {
        app.open_file(&path);
        if let Some(doc) = app.docs.active_doc_mut() {
            let pos = pos.min(doc.editor.buffer.len_chars());
            doc.editor.set_selection(rc_text::Selection::point(pos));
        }
    }
}

fn structure(app: &mut DotCraftApp, ui: &mut Ui) {
    let Some(doc) = app.docs.active_doc() else {
        let (title, hint) = empty_text(ToolWindow::Structure);
        empty_state(ui, ToolWindow::Structure.icon(), title, hint);
        return;
    };
    let Some(tree) = app.insight.structure.get(&doc.editor.id).cloned() else {
        let (title, hint) = empty_text(ToolWindow::Structure);
        empty_state(ui, ToolWindow::Structure.icon(), title, hint);
        return;
    };
    let caret_line = doc.editor.caret_line_col().0;
    let mut goto = None;
    egui::ScrollArea::vertical().id_salt("structure").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        fn walk(ui: &mut Ui, nodes: &[crate::insight::SymbolNode], depth: usize, caret_line: usize, goto: &mut Option<(usize, usize)>) {
            for n in nodes {
                let selected = n.line == caret_line;
                if list_row(ui, depth, crate::convert::symbol_icon(n.kind), &n.name, &n.detail, selected).clicked() {
                    *goto = Some((n.line, n.col));
                }
                walk(ui, &n.children, depth + 1, caret_line, goto);
            }
        }
        walk(ui, &tree, 0, caret_line, &mut goto);
    });
    if let Some((line, col)) = goto
        && let Some(d) = app.docs.active_doc_mut()
    {
        d.editor.goto(line, col);
        app.focus_editor = true;
    }
}

fn find_results(app: &mut DotCraftApp, ui: &mut Ui) {
    let Some((_, targets)) = app.find_results.clone() else { return };
    let root = app.workspace.as_ref().map(|w| w.ws.root.clone());
    let mut open = None;
    egui::ScrollArea::vertical().id_salt("find_results").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let mut last: Option<&std::path::Path> = None;
        for t in &targets {
            if last != Some(t.path.as_path()) {
                let name = t.path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                let dir = root.as_ref().and_then(|r| t.path.parent()?.strip_prefix(r).ok()).map(|d| d.display().to_string()).unwrap_or_default();
                list_row(ui, 0, rc_kit::icons::for_file(&name), &name, &dir, false);
                last = Some(t.path.as_path());
            }
            let resp = list_row(ui, 1, rc_kit::ico::FIND_NEXT, t.preview.trim(), &format!("{}:{}", t.line + 1, t.col + 1), false);
            if resp.clicked() {
                open = Some(t.clone());
            }
        }
    });
    if let Some(t) = open {
        app.open_at(&t.path, t.line, t.col);
    }
}

/// Tool window header; returns true when "Hide" was clicked.
pub fn header(ui: &mut Ui, title: &str) -> bool {
    let p = rc_kit::pal(ui);
    let r = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), HEADER_H));
    ui.allocate_rect(r, Sense::hover());
    ui.painter().text(pos2(r.left() + 12.0, r.center().y), Align2::LEFT_CENTER, title, theme::bold(13.0), p.text);
    let br = Rect::from_min_size(pos2(r.right() - 8.0 - 26.0, r.center().y - 13.0), vec2(26.0, 26.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(br));
    theme::icon_button(&mut child, rc_kit::ico::HIDE, true, tr("Hide")).clicked()
}

fn empty_text(tw: ToolWindow) -> (&'static str, &'static str) {
    match tw {
        ToolWindow::Structure => (tr("No structure"), tr("Open a C# file to see its types and members.")),
        ToolWindow::Commit => (tr("No changes"), tr("Changed files will show up here, ready to commit.")),
        ToolWindow::Run => (tr("Nothing is running"), tr("Run a project to see its output here.")),
        ToolWindow::Debug => (tr("No debug session"), tr("Start debugging to see frames, variables and watches.")),
        ToolWindow::Tests => (tr("No tests run yet"), tr("Run the tests of a project or file to see the results.")),
        ToolWindow::Build => (tr("No build output"), tr("Build the solution to see errors and warnings.")),
        ToolWindow::Terminal => (tr("Terminal"), tr("A shell in the workspace folder.")),
        ToolWindow::Problems => (tr("No problems"), tr("Errors and warnings of the open files show up here.")),
        ToolWindow::Git => (tr("Git"), tr("Branches, history and changes of the repository.")),
        ToolWindow::Find => (tr("No search results"), tr("Find in Files shows its results here.")),
        ToolWindow::Notifications => (tr("No notifications"), tr("Messages from builds, tools and updates show up here.")),
        ToolWindow::Explorer => ("", ""),
    }
}

/// Centered icon, title and hint.
pub fn empty_state(ui: &mut Ui, icon: Ico, title: &str, hint: &str) {
    let p = rc_kit::pal(ui);
    let rect = ui.available_rect_before_wrap();
    let c = rect.center() - vec2(0.0, 20.0);
    icons::paint_sized(ui.painter(), Rect::from_center_size(c - vec2(0.0, 30.0), vec2(16.0, 16.0)), icon, &p, false, 16.0);
    ui.painter().text(c, Align2::CENTER_CENTER, title, theme::bold(13.0), p.text_muted);
    let g = ui.painter().layout(hint.to_string(), FontId::proportional(12.5), p.muted, (rect.width() - 40.0).max(120.0));
    let pos = pos2(c.x - g.size().x / 2.0, c.y + 14.0);
    ui.painter().galley(pos, g, p.muted);
    ui.allocate_rect(rect, Sense::hover());
}
