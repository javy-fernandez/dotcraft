//! Debug tool window (Rider layout): toolbar with the stepping actions, then "Threads &
//! Variables" (frames on the left; watches and variables of the selected frame on the right)
//! and "Console" (program output).

use crate::app::DotCraftApp;
use crate::debugger::Debugger;
use crate::toolwindow::ToolWindow;
use crate::ui::run_windows::{console, header, open_link};
use crate::ui::toolwindows::empty_state;
use egui::{FontId, Rect, Sense, Ui, pos2, vec2};
use rc_dap::{EvalValue, Variable, WatchValue};
use rc_editor::Action;
use rc_kit::i18n::{tr, trf};
use rc_kit::{Ico, ico, icons, theme};

const ROW_H: f32 = 22.0;
const INDENT: f32 = 16.0;

pub fn show(app: &mut DotCraftApp, ui: &mut Ui) {
    let active = app.debugger.active();
    let stopped = app.debugger.stopped();
    let has = app.debugger.session.is_some();
    let km = &app.keymap;
    let tip = |a: Action, t: &str| match km.shortcut_text(a) {
        Some(s) => format!("{t} ({s})"),
        None => t.to_string(),
    };
    let (t_resume, t_pause, t_over, t_into, t_out, t_cursor, t_eval, t_stop, t_restart) = (
        tip(Action::Resume, tr("Resume Program")),
        tr("Pause Program").to_string(),
        tip(Action::StepOver, tr("Step Over")),
        tip(Action::StepInto, tr("Step Into")),
        tip(Action::StepOut, tr("Step Out")),
        tip(Action::RunToCursor, tr("Run to Cursor")),
        tip(Action::Evaluate, tr("Evaluate Expression")),
        tip(Action::StopRun, tr("Stop")),
        tr("Restart").to_string(),
    );
    let actions: [(Ico, &str, bool); 9] = [
        (ico::RERUN, &t_restart, has),
        (ico::RESUME, &t_resume, stopped),
        (ico::PAUSE, &t_pause, active && !stopped),
        (ico::STOP, &t_stop, active),
        (ico::STEP_OVER, &t_over, stopped),
        (ico::STEP_INTO, &t_into, stopped),
        (ico::STEP_OUT, &t_out, stopped),
        (ico::RUN_TO_CURSOR, &t_cursor, stopped),
        (ico::EVALUATE, &t_eval, stopped),
    ];
    let title = if app.debugger.name.is_empty() { tr("Debug").to_string() } else { format!("{}: {}", tr("Debug"), app.debugger.name) };
    let clicked = header(ui, &title, &actions);
    let ctx = ui.ctx().clone();
    match clicked {
        Some(0) => app.run_ide_action(&ctx, Action::Debug),
        Some(1) => app.run_ide_action(&ctx, Action::Resume),
        Some(2) => {
            if let Some(s) = &mut app.debugger.session {
                s.pause();
            }
        }
        Some(3) => app.debugger.stop(),
        Some(4) => app.run_ide_action(&ctx, Action::StepOver),
        Some(5) => app.run_ide_action(&ctx, Action::StepInto),
        Some(6) => app.run_ide_action(&ctx, Action::StepOut),
        Some(7) => app.run_ide_action(&ctx, Action::RunToCursor),
        Some(8) => app.run_ide_action(&ctx, Action::Evaluate),
        Some(usize::MAX) => app.toggle_tool(ToolWindow::Debug),
        _ => {}
    }
    if !has {
        let (t, h) = (tr("No debug session"), tr("Start debugging to see frames, variables and watches."));
        if let Some((done, total)) = app.debugger.downloading {
            let pct = total.map(|t| format!(" {}%", done * 100 / t.max(1))).unwrap_or_default();
            empty_state(ui, ico::DEBUG, &format!("{}{pct}", tr("Downloading netcoredbg…")), h);
        } else {
            empty_state(ui, ico::DEBUG, t, h);
        }
        return;
    }
    // Tabs and the state of the session.
    let p = rc_kit::pal(ui);
    let state = app.debugger.state_label();
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        if theme::tab(ui, !app.debugger.show_console, Some(ico::VARIABLES), tr("Threads & Variables"), false).0.clicked() {
            app.debugger.show_console = false;
        }
        if theme::tab(ui, app.debugger.show_console, Some(ico::FILE_SHELL), tr("Console"), false).0.clicked() {
            app.debugger.show_console = true;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(12.0);
            let failed = app.debugger.session.as_ref().is_some_and(|s| matches!(s.state(), rc_dap::SessionState::Failed(_)));
            ui.label(egui::RichText::new(state).size(12.0).color(if failed { p.error } else { p.muted }));
        });
    });
    ui.add_space(2.0);
    if app.debugger.show_console {
        let Some(c) = &app.debugger.console else { return };
        let events = console(ui, &c.term);
        for e in events {
            if let rc_term::ViewEvent::LinkClicked(link) = e {
                open_link(app, link, None);
            }
        }
        return;
    }
    let avail = ui.available_rect_before_wrap();
    let split = (avail.width() * 0.34).clamp(200.0, 420.0);
    let left = Rect::from_min_size(avail.min, vec2(split, avail.height()));
    let right = Rect::from_min_max(pos2(left.right() + 1.0, avail.top()), avail.max);
    ui.painter().vline(left.right() + 0.5, avail.y_range(), egui::Stroke::new(1.0, p.island_border));
    let mut lu = ui.new_child(egui::UiBuilder::new().max_rect(left));
    frames(&mut lu, &mut app.debugger);
    let mut ru = ui.new_child(egui::UiBuilder::new().max_rect(right));
    variables(&mut ru, &mut app.debugger);
    ui.advance_cursor_after_rect(avail);
}

fn frames(ui: &mut Ui, dbg: &mut Debugger) {
    let p = rc_kit::pal(ui);
    let Some(s) = &mut dbg.session else { return };
    let Some(stop) = s.stop_info().cloned() else {
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(egui::RichText::new(tr("The program is running")).color(p.muted));
        });
        return;
    };
    // Thread selector.
    let threads = s.threads().to_vec();
    let mut thread_id = ui.ctx().data(|d| d.get_temp::<i64>(egui::Id::new("dbg_thread"))).filter(|t| threads.iter().any(|x| x.id == *t)).unwrap_or(stop.thread_id);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        icons::image(ui, if thread_id == stop.thread_id { ico::THREAD_AT_BREAKPOINT } else { ico::THREADS }, 16.0);
        let name = threads.iter().find(|t| t.id == thread_id).map(|t| t.name.clone()).unwrap_or_else(|| trf("Thread {id}", &[("id", &thread_id)]));
        egui::ComboBox::from_id_salt("dbg_threads").selected_text(name).width(ui.available_width() - 16.0).show_ui(ui, |ui| {
            for t in &threads {
                ui.selectable_value(&mut thread_id, t.id, format!("{} ({})", t.name, t.id));
            }
        });
    });
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("dbg_thread"), thread_id));
    if s.frames(thread_id).is_none() {
        s.request_frames(thread_id);
    }
    let selected = s.selected_frame().map(|f| f.id);
    let list = s.frames(thread_id).cloned().unwrap_or_default();
    let mut pick = None;
    let mut more = false;
    egui::ScrollArea::vertical().id_salt("dbg_frames").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for f in &list.frames {
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
            if Some(f.id) == selected {
                ui.painter().rect_filled(r, 0.0, p.selection);
            } else if resp.hovered() {
                ui.painter().rect_filled(r, 0.0, p.row_hover);
            }
            let user = f.path().is_some();
            icons::paint(ui.painter(), Rect::from_center_size(pos2(r.left() + 18.0, r.center().y), vec2(16.0, 16.0)), ico::FRAME, &p, user);
            let mut job = egui::text::LayoutJob::default();
            let color = if user { p.text } else { p.muted };
            job.append(&f.name, 0.0, egui::TextFormat::simple(FontId::proportional(12.5), color));
            if let Some(file) = f.path().and_then(|p| p.file_name()) {
                job.append(&format!("  {}:{}", file.to_string_lossy(), f.line), 0.0, egui::TextFormat::simple(FontId::proportional(12.0), p.muted));
            }
            job.wrap.max_width = r.width() - 34.0;
            job.wrap.max_rows = 1;
            job.wrap.overflow_character = Some('…');
            let g = ui.painter().layout_job(job);
            ui.painter().galley(pos2(r.left() + 32.0, r.center().y - g.size().y / 2.0), g, color);
            if resp.clicked() {
                pick = Some(f.id);
            }
            resp.on_hover_text(&f.name);
        }
        if list.loading {
            ui.add(egui::Spinner::new().size(12.0));
        } else if list.has_more() {
            more = ui.link(tr("Load more frames")).clicked();
        }
    });
    if let Some(id) = pick {
        s.select_frame(id);
    }
    if more {
        s.request_frames(thread_id);
    }
}

fn variables(ui: &mut Ui, dbg: &mut Debugger) {
    let p = rc_kit::pal(ui);
    // "Evaluate or add a watch" field (Enter adds a watch, as the + button).
    let mut add = false;
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        let te = ui.add(egui::TextEdit::singleline(&mut dbg.new_watch).hint_text(tr("Add a watch (Enter)")).font(egui::TextStyle::Monospace).desired_width(ui.available_width() - 40.0));
        if te.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            add = true;
            te.request_focus();
        }
        if theme::icon_button(ui, ico::ADD, !dbg.new_watch.trim().is_empty(), tr("Add Watch")).clicked() {
            add = true;
        }
    });
    if add && !dbg.new_watch.trim().is_empty() {
        let e = std::mem::take(&mut dbg.new_watch);
        if let Some(s) = &mut dbg.session {
            s.add_watch(e.trim().to_string());
        }
    }
    let Some(s) = &mut dbg.session else { return };
    let stopped = s.state().is_stopped();
    let mut rows: Vec<Row> = Vec::new();
    for w in s.watches() {
        let (value, ty, reference, error) = match &w.value {
            WatchValue::Value(v) => (v.result.clone(), v.type_name.clone(), v.variables_reference, false),
            WatchValue::Error(e) => (e.clone(), None, 0, true),
            WatchValue::Evaluating => ("…".into(), None, 0, false),
            WatchValue::NotEvaluated => (tr("Cannot evaluate while running").into(), None, 0, true),
        };
        rows.push(Row { depth: 0, key: format!("w:{}", w.expression), name: w.expression.clone(), value, ty, reference, error, watch: Some(w.id), scope: false });
    }
    if stopped && let Some(frame) = s.selected_frame().map(|f| f.id) {
        match s.scopes(frame) {
            None => s.request_scopes(frame),
            Some(scopes) => {
                let scopes = scopes.to_vec();
                for sc in scopes {
                    // Locals are shown flat (Rider); other scopes as expandable groups.
                    let flat = scopes_flat(&sc.name);
                    if flat {
                        children(s, &dbg.expanded, sc.variables_reference, &format!("s:{}", sc.name), 0, &mut rows);
                    } else {
                        let key = format!("s:{}", sc.name);
                        rows.push(Row { depth: 0, key: key.clone(), name: sc.name.clone(), value: String::new(), ty: None, reference: sc.variables_reference, error: false, watch: None, scope: true });
                        if dbg.expanded.contains(&key) {
                            children(s, &dbg.expanded, sc.variables_reference, &key, 1, &mut rows);
                        }
                    }
                }
            }
        }
    }
    let mut toggle = None;
    let mut remove = None;
    let mut add_watch = None;
    egui::ScrollArea::vertical().id_salt("dbg_vars").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        if rows.is_empty() {
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                let msg = if stopped { tr("No variables") } else { tr("Variables are shown when the program is paused") };
                ui.label(egui::RichText::new(msg).color(p.muted));
            });
        }
        for row in &rows {
            let resp = paint_row(ui, row, dbg.expanded.contains(&row.key));
            if resp.clicked() && row.reference > 0 {
                toggle = Some(row.key.clone());
            }
            resp.context_menu(|ui| {
                if let Some(id) = row.watch {
                    if theme::menu_item(ui, Some(ico::REMOVE), tr("Remove Watch"), false).clicked() {
                        remove = Some(id);
                    }
                } else if !row.scope && theme::menu_item(ui, Some(ico::ADD_TO_WATCH), tr("Add to Watches"), false).clicked() {
                    add_watch = Some(row.name.clone());
                }
                if !row.scope && theme::menu_item(ui, Some(ico::COPY), tr("Copy Value"), false).clicked() {
                    ui.ctx().copy_text(row.value.clone());
                }
            });
        }
    });
    if let Some(k) = toggle
        && !dbg.expanded.remove(&k)
    {
        dbg.expanded.insert(k);
    }
    if let Some(id) = remove {
        s.remove_watch(id);
    }
    if let Some(e) = add_watch {
        s.add_watch(e);
    }
}

fn scopes_flat(name: &str) -> bool {
    name.eq_ignore_ascii_case("locals")
}

struct Row {
    depth: usize,
    key: String,
    name: String,
    value: String,
    ty: Option<String>,
    reference: i64,
    error: bool,
    watch: Option<u64>,
    scope: bool,
}

/// Children of `reference` (requested when not loaded), recursing into expanded rows.
fn children(s: &mut rc_dap::DebugSession, expanded: &std::collections::BTreeSet<String>, reference: i64, parent: &str, depth: usize, rows: &mut Vec<Row>) {
    if depth > 12 {
        return;
    }
    let Some(vars) = s.variables(reference).map(|v| v.to_vec()) else {
        s.request_variables(reference);
        rows.push(Row { depth, key: format!("{parent}/…"), name: "…".into(), value: String::new(), ty: None, reference: 0, error: false, watch: None, scope: false });
        return;
    };
    for v in vars {
        push_var(s, expanded, &v, parent, depth, rows);
    }
}

fn push_var(s: &mut rc_dap::DebugSession, expanded: &std::collections::BTreeSet<String>, v: &Variable, parent: &str, depth: usize, rows: &mut Vec<Row>) {
    let key = format!("{parent}/{}", v.name);
    rows.push(Row { depth, key: key.clone(), name: v.name.clone(), value: v.value.clone(), ty: v.type_name.clone(), reference: v.variables_reference, error: false, watch: None, scope: false });
    if v.variables_reference > 0 && expanded.contains(&key) {
        children(s, expanded, v.variables_reference, &key, depth + 1, rows);
    }
}

fn var_icon(row: &Row) -> Ico {
    if row.watch.is_some() {
        return ico::WATCH;
    }
    if row.scope {
        return ico::VARIABLES;
    }
    let ty = row.ty.as_deref().unwrap_or("");
    if ty.ends_with(']') || ty.starts_with("System.Collections") || ty.contains("List<") || ty.contains("Dictionary<") {
        ico::VAR_ARRAY
    } else if row.reference > 0 {
        ico::VAR_OBJECT
    } else if row.name == "this" {
        ico::VAR_SPECIAL
    } else {
        ico::VAR_PRIMITIVE
    }
}

fn paint_row(ui: &mut Ui, row: &Row, open: bool) -> egui::Response {
    let p = rc_kit::pal(ui);
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, 0.0, p.row_hover);
    }
    let x0 = r.left() + 8.0 + row.depth as f32 * INDENT;
    if row.reference > 0 {
        let chevron = if open { ico::CHEVRON_DOWN } else { ico::CHEVRON_RIGHT };
        icons::paint(ui.painter(), Rect::from_center_size(pos2(x0 + 8.0, r.center().y), vec2(16.0, 16.0)), chevron, &p, true);
    }
    icons::paint(ui.painter(), Rect::from_center_size(pos2(x0 + 26.0, r.center().y), vec2(16.0, 16.0)), var_icon(row), &p, true);
    let mut job = egui::text::LayoutJob::default();
    let font = FontId::monospace(12.5);
    let sc = theme::scheme(p.dark);
    let name_color = if row.watch.is_some() || row.scope { p.text } else { sc.field };
    job.append(&row.name, 0.0, egui::TextFormat::simple(font.clone(), name_color));
    if !row.scope && !row.value.is_empty() {
        job.append(" = ", 0.0, egui::TextFormat::simple(font.clone(), p.text));
        // netcoredbg shows objects as `{Full.Type.Name}`: show the short type once (Rider).
        let object = row.value.starts_with('{') && row.value.ends_with('}');
        if object {
            job.append(&format!("{{{}}}", short_type(&row.value[1..row.value.len() - 1])), 0.0, egui::TextFormat::simple(font.clone(), p.muted));
        } else if let Some(t) = row.ty.as_deref().filter(|t| !t.is_empty() && row.reference > 0) {
            job.append(&format!("{{{}}} ", short_type(t)), 0.0, egui::TextFormat::simple(font.clone(), p.muted));
        }
        let vc = if row.error {
            p.error
        } else if row.value.starts_with('"') {
            sc.string
        } else if row.value.chars().next().is_some_and(|c| c.is_ascii_digit() || c == '-') {
            sc.number
        } else if matches!(row.value.as_str(), "true" | "false" | "null") {
            sc.keyword
        } else {
            p.text
        };
        if !object {
            job.append(&row.value, 0.0, egui::TextFormat::simple(font, vc));
        }
    }
    job.wrap.max_width = (r.right() - x0 - 40.0).max(40.0);
    job.wrap.max_rows = 1;
    job.wrap.overflow_character = Some('…');
    let g = ui.painter().layout_job(job);
    let truncated = g.rows.first().is_some_and(|r| r.ends_with_newline) || g.size().x >= (r.right() - x0 - 41.0);
    ui.painter().galley(pos2(x0 + 38.0, r.center().y - g.size().y / 2.0), g, p.text);
    if truncated && !row.value.is_empty() { resp.on_hover_text(format!("{} = {}", row.name, row.value)) } else { resp }
}

/// `System.Collections.Generic.List<Shop.Core.OrderLine>` → `List<OrderLine>`.
fn short_type(t: &str) -> String {
    let mut out = String::new();
    let mut ident = String::new();
    for c in t.chars() {
        if c.is_alphanumeric() || c == '_' || c == '.' || c == '`' {
            ident.push(c);
        } else {
            out.push_str(ident.rsplit('.').next().unwrap_or(""));
            ident.clear();
            out.push(c);
        }
    }
    out.push_str(ident.rsplit('.').next().unwrap_or(""));
    out
}

/// The result of an evaluation, expandable like the Variables tree (Evaluate dialog).
pub fn eval_tree(ui: &mut Ui, dbg: &mut Debugger, expression: &str, v: &EvalValue) {
    let Some(s) = &mut dbg.session else { return };
    let mut rows = Vec::new();
    let key = format!("e:{expression}");
    rows.push(Row { depth: 0, key: key.clone(), name: expression.to_string(), value: v.result.clone(), ty: v.type_name.clone(), reference: v.variables_reference, error: false, watch: None, scope: false });
    if v.variables_reference > 0 && dbg.expanded.contains(&key) {
        children(s, &dbg.expanded, v.variables_reference, &key, 1, &mut rows);
    }
    let mut toggle = None;
    egui::ScrollArea::vertical().id_salt("eval_tree").max_height(260.0).auto_shrink([false, true]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for row in &rows {
            let mut row_icon = clone_row(row);
            if row.depth == 0 {
                row_icon.watch = None;
            }
            if paint_row(ui, &row_icon, dbg.expanded.contains(&row.key)).clicked() && row.reference > 0 {
                toggle = Some(row.key.clone());
            }
        }
    });
    if let Some(k) = toggle
        && !dbg.expanded.remove(&k)
    {
        dbg.expanded.insert(k);
    }
}

fn clone_row(r: &Row) -> Row {
    Row { depth: r.depth, key: r.key.clone(), name: r.name.clone(), value: r.value.clone(), ty: r.ty.clone(), reference: r.reference, error: r.error, watch: r.watch, scope: r.scope }
}

#[cfg(test)]
mod tests {
    #[test]
    fn short_types_drop_namespaces() {
        assert_eq!(super::short_type("System.Collections.Generic.List<Shop.Core.OrderLine>"), "List<OrderLine>");
        assert_eq!(super::short_type("Shop.Core.Order[]"), "Order[]");
        assert_eq!(super::short_type("System.Collections.Generic.Dictionary<string, int>"), "Dictionary<string, int>");
    }
}
