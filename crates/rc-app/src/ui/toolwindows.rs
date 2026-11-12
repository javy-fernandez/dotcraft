//! Contents of the tool windows. Each window has the New UI header (title on the left,
//! actions and "hide" on the right) and an empty state that says what will show up there.

use crate::app::DotCraftApp;
use crate::toolwindow::ToolWindow;
use crate::ui::{explorer, kit};
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
            app.explorer.single_click = app.settings.single_click_open;
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
        ToolWindow::Build => crate::ui::run_windows::build(app, ui),
        ToolWindow::Run => crate::ui::run_windows::run(app, ui),
        ToolWindow::Debug => crate::ui::debug_window::show(app, ui),
        ToolWindow::Nuget => crate::ui::nuget::window(app, ui),
        ToolWindow::Database => crate::ui::database_window::show(app, ui),
        ToolWindow::Monitoring => crate::ui::monitoring::show(app, ui),
        ToolWindow::Todo => todo_window(app, ui),
        ToolWindow::Hierarchy => hierarchy_window(app, ui),
        ToolWindow::LocalHistory => local_history_window(app, ui),
        ToolWindow::HttpClient => http_client_window(app, ui),
        ToolWindow::Endpoints => endpoints_window(app, ui),
        ToolWindow::Coverage => coverage_window(app, ui),
        ToolWindow::Bookmarks => {
            if header(ui, tw.title()) {
                app.toggle_tool(tw);
            }
            bookmarks(app, ui);
        }
        ToolWindow::Tests => crate::ui::run_windows::tests(app, ui),
        ToolWindow::Terminal => crate::ui::run_windows::terminal(app, ui),
        ToolWindow::Commit => crate::ui::git::commit_window(app, ui),
        ToolWindow::Git => crate::ui::git::git_window(app, ui),
        ToolWindow::PullRequests => crate::ui::pull_requests::window(app, ui),
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
        ToolWindow::Find if app.find_results.is_none() && app.find_in_files.is_some() => {
            if header(ui, tr("Find in Files")) {
                app.toggle_tool(tw);
            }
            find_in_files(app, ui);
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

/// A coverage bar: the covered part green, the rest red.
fn coverage_bar(ui: &Ui, rect: Rect, percent: f64) {
    let p = rc_kit::pal(ui);
    ui.painter().rect_filled(rect, 2.0, egui::Color32::from_rgb(0xE5, 0x53, 0x53).gamma_multiply(0.75));
    let w = rect.width() * (percent / 100.0).clamp(0.0, 1.0) as f32;
    ui.painter().rect_filled(Rect::from_min_size(rect.min, vec2(w, rect.height())), 2.0, egui::Color32::from_rgb(0x5F, 0xB8, 0x65));
    let _ = p;
}

/// Coverage: total, then assemblies and their classes with a bar each; double click opens.
fn coverage_window(app: &mut DotCraftApp, ui: &mut Ui) {
    let p = rc_kit::pal(ui);
    let r = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), HEADER_H));
    ui.allocate_rect(r, Sense::hover());
    let t = ui.painter().text(pos2(r.left() + 12.0, r.center().y), Align2::LEFT_CENTER, tr("Coverage"), theme::bold(13.0), p.text);
    let mut x = t.right() + 14.0;
    if let Some(rep) = &app.coverage.report
        && let Some(pc) = rep.total.percent()
    {
        let s = ui.painter().text(pos2(x, r.center().y), Align2::LEFT_CENTER, rc_kit::i18n::trf("{pc}% of lines ({covered}/{total})", &[("pc", &format!("{pc:.0}")), ("covered", &rep.total.covered), ("total", &rep.total.total)]), FontId::proportional(12.0), p.muted);
        x = s.right() + 12.0;
    }
    if app.coverage.running {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(300));
        ui.painter().text(pos2(x, r.center().y), Align2::LEFT_CENTER, &app.coverage.status, FontId::proportional(12.0), p.muted);
    }
    let mut clicked = None;
    let running = app.coverage.running;
    let has = app.coverage.report.is_some();
    let actions = [
        (rc_kit::ico::HIDE, tr("Hide"), true),
        (if running { rc_kit::ico::STOP } else { rc_kit::ico::RUN_COVERAGE }, if running { tr("Stop") } else { tr("Run All Tests with Coverage") }, app.workspace.is_some()),
        (rc_kit::ico::SHOW, tr("Show Coverage in Editor"), has),
    ];
    for (i, (icon, tip, enabled)) in actions.into_iter().enumerate() {
        let br = Rect::from_min_size(pos2(r.right() - 8.0 - 26.0 * (i + 1) as f32, r.center().y - 13.0), vec2(26.0, 26.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(br));
        let resp = if i == 2 { theme::icon_toggle(&mut child, icon, app.coverage.show_in_editor && has, tip) } else { theme::icon_button(&mut child, icon, enabled, tip) };
        if resp.clicked() && enabled {
            clicked = Some(i);
        }
    }
    match clicked {
        Some(0) => {
            app.toggle_tool(ToolWindow::Coverage);
            return;
        }
        Some(1) if running => app.coverage.stop(),
        Some(1) => app.run_coverage(),
        Some(2) => app.coverage.show_in_editor = !app.coverage.show_in_editor,
        _ => {}
    }
    for w in app.coverage.warnings.clone() {
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            rc_kit::icons::image(ui, rc_kit::ico::WARNING, 14.0);
            ui.label(egui::RichText::new(w).size(12.0).color(p.muted));
        });
    }
    if let Some(e) = app.coverage.error.clone() {
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            rc_kit::icons::image(ui, rc_kit::ico::ERROR, 14.0);
            ui.label(egui::RichText::new(e).size(12.5).color(p.error));
        });
    }
    let Some(rep) = &app.coverage.report else {
        if !running && app.coverage.error.is_none() {
            let (title, hint) = empty_text(ToolWindow::Coverage);
            if empty_state_with(ui, ToolWindow::Coverage.icon(), title, hint, app.workspace.is_some().then_some(tr("Run All Tests with Coverage"))) {
                app.run_coverage();
            }
        }
        return;
    };
    let mut toggle: Option<String> = None;
    let mut open: Option<(std::path::PathBuf, usize)> = None;
    egui::ScrollArea::vertical().id_salt("coverage").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for a in &rep.assemblies {
            let expanded = app.coverage.expanded.contains(&a.name);
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(420.0), 24.0), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(rect.shrink2(vec2(6.0, 0.0)), 4.0, p.row_hover);
            }
            icons::paint(ui.painter(), Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), vec2(16.0, 16.0)), if expanded { rc_kit::ico::CHEVRON_DOWN } else { rc_kit::ico::CHEVRON_RIGHT }, &p, true);
            icons::paint(ui.painter(), Rect::from_center_size(pos2(rect.left() + 36.0, rect.center().y), vec2(16.0, 16.0)), rc_kit::ico::PROJECT, &p, true);
            ui.painter().text(pos2(rect.left() + 50.0, rect.center().y), Align2::LEFT_CENTER, &a.name, theme::bold(13.0), p.text);
            coverage_row_stats(ui, rect, a.ratio.percent(), a.ratio.covered, a.ratio.total);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &a.name));
            if resp.clicked() {
                toggle = Some(a.name.clone());
            }
            if !expanded {
                continue;
            }
            for c in rep.classes.iter().filter(|c| c.assembly == a.name) {
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(420.0), 24.0), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(rect.shrink2(vec2(6.0, 0.0)), 4.0, p.row_hover);
                }
                icons::paint(ui.painter(), Rect::from_center_size(pos2(rect.left() + 54.0, rect.center().y), vec2(16.0, 16.0)), rc_kit::ico::CLASS, &p, true);
                let name = c.name.rsplit('.').next().unwrap_or(&c.name);
                ui.painter().text(pos2(rect.left() + 68.0, rect.center().y), Align2::LEFT_CENTER, name, FontId::proportional(13.0), p.text);
                coverage_row_stats(ui, rect, c.ratio.percent(), c.ratio.covered, c.ratio.total);
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name));
                if resp.double_clicked() {
                    open = Some((c.file.clone(), c.lines.iter().next().copied().unwrap_or(0)));
                }
            }
        }
    });
    if let Some(n) = toggle
        && !app.coverage.expanded.remove(&n)
    {
        app.coverage.expanded.insert(n);
    }
    if let Some((path, line)) = open {
        app.open_at(&path, line, 0);
    }
}

/// Right side of a coverage row: the percentage, its bar and "covered/total".
fn coverage_row_stats(ui: &Ui, rect: Rect, percent: Option<f64>, covered: u32, total: u32) {
    let p = rc_kit::pal(ui);
    let x = (rect.right() - 300.0).max(rect.left() + 260.0);
    let pc = percent.unwrap_or(0.0);
    ui.painter().text(pos2(x, rect.center().y), Align2::RIGHT_CENTER, format!("{pc:.0}%"), FontId::monospace(12.0), p.text);
    coverage_bar(ui, Rect::from_min_size(pos2(x + 10.0, rect.center().y - 4.0), vec2(140.0, 8.0)), pc);
    ui.painter().text(pos2(x + 160.0, rect.center().y), Align2::LEFT_CENTER, format!("{covered}/{total}"), FontId::proportional(12.0), p.muted);
}

/// A coloured HTTP method badge ("GET" green, "POST" blue…); returns its right edge.
fn method_badge(ui: &Ui, left_center: egui::Pos2, method: &str) -> f32 {
    let color = match method {
        "GET" => egui::Color32::from_rgb(0x5F, 0xB8, 0x65),
        "POST" => egui::Color32::from_rgb(0x35, 0x74, 0xF0),
        "PUT" | "PATCH" => egui::Color32::from_rgb(0xE0, 0x88, 0x55),
        "DELETE" => egui::Color32::from_rgb(0xE5, 0x53, 0x53),
        _ => rc_kit::pal(ui).muted,
    };
    let g = ui.painter().layout_no_wrap(method.to_string(), FontId::monospace(11.0), color);
    let r = Rect::from_min_size(pos2(left_center.x, left_center.y - 9.0), vec2(g.size().x.max(30.0) + 10.0, 18.0));
    ui.painter().rect(r, 4.0, color.gamma_multiply(0.15), egui::Stroke::NONE, egui::StrokeKind::Inside);
    ui.painter().galley(pos2(r.center().x - g.size().x / 2.0, r.center().y - g.size().y / 2.0), g, color);
    r.right()
}

/// Endpoints: routes by project; double click opens the handler, the context menu writes a
/// request in the project's `.http` file. The header filters by text and method, groups the
/// routes by method and keeps only route conflicts; a section imports OpenAPI JSON.
fn endpoints_window(app: &mut DotCraftApp, ui: &mut Ui) {
    let p = rc_kit::pal(ui);
    if let Some(result) = app.endpoints.import_result.take() {
        match result {
            Ok((path, line)) => app.open_at(&path, line, 0),
            Err(error) => app.notify(error, true),
        }
    }
    let r = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), HEADER_H));
    ui.allocate_rect(r, Sense::hover());
    let t = ui.painter().text(pos2(r.left() + 12.0, r.center().y), Align2::LEFT_CENTER, tr("Endpoints"), theme::bold(13.0), p.text);
    let mut status = if app.endpoints.scanning {
        tr("Scanning…").to_string()
    } else {
        let n: usize = app.endpoints.projects.iter().map(|(_, _, e)| e.len()).sum();
        rc_kit::i18n::trf("{n} items", &[("n", &n)])
    };
    if !app.endpoints.conflicts.is_empty() {
        status = format!("{status} · {}", rc_kit::i18n::trf("{n} route conflicts", &[("n", &app.endpoints.conflicts.len())]));
    }
    let actions_left = r.right() - 8.0 - 26.0 * 4.0;
    let sr = Rect::from_min_max(pos2(t.right() + 12.0, r.top()), pos2(t.right() + 104.0, r.bottom()));
    let mut job = egui::text::LayoutJob::simple_singleline(status, FontId::proportional(12.0), p.muted);
    job.wrap.max_width = sr.width();
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    let g = ui.painter().layout_job(job);
    ui.painter().galley(pos2(sr.left(), r.center().y - g.size().y / 2.0), g, p.muted);
    // Filter field and method drop-down.
    let fr = Rect::from_min_size(pos2(t.right() + 110.0, r.center().y - 12.0), vec2((actions_left - t.right() - 110.0 - 140.0).clamp(120.0, 320.0), 24.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(fr));
    child.add(egui::TextEdit::singleline(&mut app.endpoints.filter).hint_text(tr("Search routes, handlers and files")).desired_width(fr.width()));
    let dr = Rect::from_min_size(pos2(fr.right() + 6.0, r.center().y - 13.0), vec2(130.0, 26.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(dr));
    let methods: Vec<String> = std::iter::once(tr("All methods").to_string()).chain(["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "TRACE", "ANY"].map(String::from)).collect();
    let selected = if app.endpoints.method.is_empty() { tr("All methods").to_string() } else { app.endpoints.method.clone() };
    if let Some(i) = kit::dropdown(&mut child, "endpoint_method", &selected, &methods, true) {
        app.endpoints.method = if i == 0 { String::new() } else { methods[i].clone() };
    }
    // Actions from the right: Hide, Refresh (Cancel scan while scanning), the two toggles.
    let slot = |ui: &mut Ui, i: usize| ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(pos2(r.right() - 8.0 - 26.0 * (i + 1) as f32, r.center().y - 13.0), vec2(26.0, 26.0))));
    if theme::icon_button(&mut slot(ui, 0), rc_kit::ico::HIDE, true, tr("Hide")).clicked() {
        app.toggle_tool(ToolWindow::Endpoints);
        return;
    }
    if app.endpoints.scanning {
        if theme::icon_button(&mut slot(ui, 1), rc_kit::ico::STOP, true, tr("Cancel scan")).clicked() {
            app.endpoints.cancel_scan();
        }
    } else if theme::icon_button(&mut slot(ui, 1), rc_kit::ico::REFRESH, true, tr("Refresh")).clicked() {
        app.refresh_endpoints();
    }
    if theme::icon_toggle(&mut slot(ui, 2), rc_kit::ico::FILTER, app.endpoints.only_conflicts, tr("Route conflicts only")).clicked() {
        app.endpoints.only_conflicts = !app.endpoints.only_conflicts;
    }
    if theme::icon_toggle(&mut slot(ui, 3), rc_kit::ico::SORT, app.endpoints.group_by_method, tr("Group by method")).clicked() {
        app.endpoints.group_by_method = !app.endpoints.group_by_method;
    }
    // OpenAPI import, folded by default.
    let open_id = Id::new("endpoints_import_open");
    let mut import_open = ui.data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false);
    if kit::section(ui, rc_kit::ico::IMPORT, tr("Import OpenAPI JSON"), "", &mut import_open) {
        let importing = app.endpoints.importing;
        kit::content(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                kit::field(ui, tr("OpenAPI JSON file"), &mut app.endpoints.import_path, "", 280.0, !importing);
                kit::field(ui, tr("New HTTP output file"), &mut app.endpoints.output_path, "", 280.0, !importing);
            });
            kit::note(ui, tr("Imports OpenAPI 3.0/3.1 JSON with an explicit HTTP server. Existing files are never overwritten."));
            ui.horizontal(|ui| {
                let can = !importing && !app.endpoints.import_path.trim().is_empty() && !app.endpoints.output_path.trim().is_empty();
                if kit::button(ui, Some(rc_kit::ico::IMPORT), tr("Import requests"), can).clicked() && can {
                    app.endpoints.import_openapi(app.waker.clone());
                }
                if importing {
                    ui.add(egui::Spinner::new().size(14.0));
                    if kit::button(ui, Some(rc_kit::ico::STOP), tr("Cancel import"), true).clicked() {
                        app.endpoints.cancel_import();
                    }
                }
            });
        });
    }
    ui.data_mut(|d| d.insert_temp(open_id, import_open));
    if app.endpoints.projects.is_empty() && !app.endpoints.scanning {
        let (title, hint) = empty_text(ToolWindow::Endpoints);
        empty_state(ui, ToolWindow::Endpoints.icon(), title, hint);
        return;
    }
    let ep = &app.endpoints;
    let mut open: Option<(std::path::PathBuf, usize)> = None;
    let mut generate: Option<(std::path::PathBuf, rc_dotnet::endpoints::Endpoint)> = None;
    egui::ScrollArea::vertical().id_salt("endpoints").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for (name, project, eps) in &ep.projects {
            let mut shown: Vec<&rc_dotnet::endpoints::Endpoint> = eps
                .iter()
                .filter(|e| crate::web::endpoint_matches(e, &ep.filter, &ep.method))
                .filter(|e| !ep.only_conflicts || ep.conflicts.iter().any(|(_, a, b)| a == *e || b == *e))
                .collect();
            if shown.is_empty() {
                continue;
            }
            if ep.group_by_method {
                shown.sort_by_key(|e| (&e.method, &e.route));
            }
            list_row(ui, 0, rc_kit::ico::PROJECT, name, &rc_kit::i18n::trf("{n} items", &[("n", &shown.len())]), false);
            let indent = if ep.group_by_method { 48.0 } else { 30.0 };
            let mut group = "";
            for e in shown {
                if ep.group_by_method && group != e.method {
                    group = &e.method;
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(400.0), 24.0), Sense::hover());
                    method_badge(ui, pos2(rect.left() + 30.0, rect.center().y), group);
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, group));
                }
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(400.0), 24.0), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(rect.shrink2(vec2(6.0, 0.0)), 4.0, p.row_hover);
                }
                let x = method_badge(ui, pos2(rect.left() + indent, rect.center().y), &e.method);
                let g = ui.painter().layout_no_wrap(e.route.clone(), FontId::monospace(12.5), p.text);
                ui.painter().galley(pos2(x + 10.0, rect.center().y - g.size().y / 2.0), g.clone(), p.text);
                let file = e.file.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
                ui.painter().text(pos2(x + 24.0 + g.size().x, rect.center().y), Align2::LEFT_CENTER, format!("{}  {file}:{}", e.handler, e.line + 1), FontId::proportional(12.0), p.muted);
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{} {}", e.method, e.route)));
                if resp.double_clicked() {
                    open = Some((e.file.clone(), e.line));
                }
                resp.context_menu(|ui| {
                    if theme::menu_entry(ui, Some(rc_kit::ico::TW_HTTP), tr("Generate Request in HTTP Client"), None, false, !ep.importing).clicked() {
                        generate = Some((project.clone(), e.clone()));
                        ui.close();
                    }
                    if theme::menu_item(ui, Some(rc_kit::ico::OPEN), tr("Go to Handler"), false).clicked() {
                        open = Some((e.file.clone(), e.line));
                        ui.close();
                    }
                });
                // Conflicts: a warning line under the route.
                for (_, _, other) in ep.conflicts.iter().filter(|(_, first, _)| first == e) {
                    let text = rc_kit::i18n::trf("Route may conflict with {handler} at {file}:{line}", &[("handler", &other.handler), ("file", &other.file.display()), ("line", &(other.line + 1))]);
                    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width().max(400.0), 20.0), Sense::hover());
                    icons::paint(ui.painter(), Rect::from_center_size(pos2(rect.left() + indent + 8.0, rect.center().y), vec2(14.0, 14.0)), rc_kit::ico::DIALOG_WARNING, &p, true);
                    ui.painter().text(pos2(rect.left() + indent + 22.0, rect.center().y), Align2::LEFT_CENTER, text, FontId::proportional(12.0), p.warn);
                }
            }
        }
    });
    if let Some((path, line)) = open {
        app.open_at(&path, line, 0);
    }
    if let Some((project, endpoint)) = generate {
        app.endpoints.generate(project, endpoint, app.waker.clone());
    }
}

/// HTTP Client: environment, the request sent, then status, time, size and the body (or the
/// headers).
fn http_client_window(app: &mut DotCraftApp, ui: &mut Ui) {
    let p = rc_kit::pal(ui);
    let r = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), HEADER_H));
    ui.allocate_rect(r, Sense::hover());
    let t = ui.painter().text(pos2(r.left() + 12.0, r.center().y), Align2::LEFT_CENTER, tr("HTTP Client"), theme::bold(13.0), p.text);
    let http_path = app.docs.active_doc().map(|d| d.path.clone()).filter(|p| crate::web::is_http_file(p));
    app.http.discover_environments(http_path.as_deref(), false);
    // Environment of the active .http file.
    if http_path.is_some() || !app.http.environments.is_empty() {
        let er = Rect::from_min_size(pos2(t.right() + 14.0, r.center().y - 13.0), vec2(220.0, 26.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(er).layout(egui::Layout::left_to_right(egui::Align::Center)));
        let options: Vec<String> = std::iter::once(tr("No environment").to_string()).chain(app.http.environments.iter().cloned()).collect();
        let selected = app.http.env.clone().unwrap_or_else(|| tr("No environment").to_string());
        if let Some(i) = kit::dropdown(&mut child, "http_env", &selected, &options, true) {
            app.http.env = if i == 0 { None } else { Some(options[i].clone()) };
        }
        if app.http.environments_loading {
            child.add(egui::Spinner::new().size(14.0));
        }
    }
    // Actions from the right: Hide, Refresh environments, Cancel Request while sending.
    let slot = |ui: &mut Ui, i: usize| ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(pos2(r.right() - 8.0 - 26.0 * (i + 1) as f32, r.center().y - 13.0), vec2(26.0, 26.0))));
    if theme::icon_button(&mut slot(ui, 0), rc_kit::ico::HIDE, true, tr("Hide")).clicked() {
        app.toggle_tool(ToolWindow::HttpClient);
        return;
    }
    if theme::icon_button(&mut slot(ui, 1), rc_kit::ico::REFRESH, http_path.is_some(), tr("Refresh environments")).clicked() && http_path.is_some() {
        app.http.discover_environments(http_path.as_deref(), true);
    }
    if app.http.busy() && theme::icon_button(&mut slot(ui, 2), rc_kit::ico::STOP, true, tr("Cancel Request")).clicked() {
        app.http.cancel();
    }
    let Some(request) = app.http.request.clone() else {
        let (title, hint) = empty_text(ToolWindow::HttpClient);
        empty_state(ui, ToolWindow::HttpClient.icon(), title, hint);
        return;
    };
    // Request line and status.
    let line = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), 28.0));
    ui.allocate_rect(line, Sense::hover());
    let (method, url) = request.split_once(' ').unwrap_or(("", &request));
    let x = method_badge(ui, pos2(line.left() + 12.0, line.center().y), method);
    let u = ui.painter().text(pos2(x + 10.0, line.center().y), Align2::LEFT_CENTER, url, FontId::monospace(12.5), p.text);
    let mut sx = u.right() + 16.0;
    match &app.http.response {
        None if app.http.busy() => {
            ui.painter().text(pos2(sx, line.center().y), Align2::LEFT_CENTER, tr("Sending…"), FontId::proportional(12.0), p.muted);
        }
        Some(Err(e)) => {
            let g = ui.painter().layout(e.clone(), FontId::proportional(12.5), p.error, (line.width() - sx + line.left() - 12.0).max(200.0));
            ui.painter().galley(pos2(sx, line.top() + 6.0), g, p.error);
        }
        Some(Ok(resp)) => {
            let color = if resp.status < 300 { p.ok } else if resp.status < 400 { p.warn } else { p.error };
            let s = ui.painter().text(pos2(sx, line.center().y), Align2::LEFT_CENTER, format!("{} {}", resp.status, resp.reason), theme::bold(12.5), color);
            sx = s.right() + 14.0;
            let size = if resp.body.len() >= 1024 { format!("{:.1} KB", resp.body.len() as f64 / 1024.0) } else { format!("{} B", resp.body.len()) };
            ui.painter().text(pos2(sx, line.center().y), Align2::LEFT_CENTER, format!("{} ms · {size}", resp.elapsed.as_millis()), FontId::proportional(12.0), p.muted);
            // Body / Headers.
            let tabs = Rect::from_min_size(pos2(line.right() - 190.0, line.center().y - 12.0), vec2(180.0, 24.0));
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(tabs).layout(egui::Layout::left_to_right(egui::Align::Center)));
            if theme::tool_button(&mut child, rc_kit::ico::FILE_TEXT, tr("Body"), !app.http.show_headers).clicked() {
                app.http.show_headers = false;
            }
            if theme::tool_button(&mut child, rc_kit::ico::MENU, tr("Headers"), app.http.show_headers).clicked() {
                app.http.show_headers = true;
            }
        }
        None => {}
    }
    let Some(Ok(resp)) = &app.http.response else { return };
    let body_rect = ui.available_rect_before_wrap();
    ui.painter().hline(body_rect.x_range(), body_rect.top(), egui::Stroke::new(1.0, p.island_border));
    if app.http.show_headers {
        let headers = resp.headers.clone();
        egui::ScrollArea::vertical().id_salt("http_headers").auto_shrink([false, false]).show(ui, |ui| {
            for (k, v) in &headers {
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.label(egui::RichText::new(format!("{k}:")).monospace().color(p.muted));
                    ui.label(egui::RichText::new(v).monospace());
                });
            }
        });
    } else if let Some(body) = &mut app.http.body {
        let config = app.editor_config.clone();
        let mut eu = ui.new_child(egui::UiBuilder::new().max_rect(body_rect));
        eu.set_clip_rect(body_rect.intersect(ui.clip_rect()));
        rc_editor::EditorView::new(body, &config, &app.keymap).show(&mut eu);
        ui.allocate_rect(body_rect, Sense::hover());
    }
}

/// Local History of a file: its revisions, newest first; a click shows the diff with the
/// current text, the context menu restores one.
fn local_history_window(app: &mut DotCraftApp, ui: &mut Ui) {
    let p = rc_kit::pal(ui);
    let path = app.history_file.clone().or_else(|| app.docs.active_doc().map(|d| d.path.clone()));
    let name = path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let title = if name.is_empty() { tr("Local History").to_string() } else { format!("{} · {name}", tr("Local History")) };
    if header(ui, &title) {
        app.toggle_tool(ToolWindow::LocalHistory);
        return;
    }
    let revisions = path.as_deref().map(rc_core::local_history::revisions).unwrap_or_default();
    let Some(path) = path.filter(|_| !revisions.is_empty()) else {
        let (title, hint) = empty_text(ToolWindow::LocalHistory);
        empty_state(ui, ToolWindow::LocalHistory.icon(), title, hint);
        return;
    };
    let current = match app.docs.index_of(&path) {
        Some(i) => app.docs.list[i].editor.buffer.rope().to_string(),
        None => std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n"),
    };
    let mut show: Option<usize> = None;
    let mut restore: Option<usize> = None;
    egui::ScrollArea::vertical().id_salt("local_history").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for (i, r) in revisions.iter().enumerate() {
            let when = format_time(r.time);
            let label = match r.label.as_str() {
                "Saved" => tr("Saved"),
                "Original" => tr("Original"),
                "External change" => tr("External change"),
                "Before restore" => tr("Before restore"),
                other => other,
            };
            let resp = list_row(ui, 0, rc_kit::ico::HISTORY, &when, label, app.history_selected == Some(i));
            if resp.clicked() {
                show = Some(i);
            }
            resp.context_menu(|ui| {
                if theme::menu_item(ui, Some(rc_kit::ico::DIFF), tr("Show Diff"), false).clicked() {
                    show = Some(i);
                    ui.close();
                }
                if theme::menu_item(ui, Some(rc_kit::ico::UNDO), tr("Revert to This Version"), false).clicked() {
                    restore = Some(i);
                    ui.close();
                }
            });
        }
    });
    let _ = p;
    let dark = app.dark(ui.ctx());
    if let Some(i) = show
        && let Ok(old) = revisions[i].read()
    {
        app.history_selected = Some(i);
        let title = format!("{name}: {} ↔ {}", format_time(revisions[i].time), tr("Current"));
        app.vcs.show_diff(title, path.clone(), &old, &current, dark);
    }
    if let Some(i) = restore
        && let Ok(old) = revisions[i].read()
    {
        let _ = rc_core::local_history::record(&path, &current, "Before restore");
        app.open_file(&path);
        if let Some(d) = app.docs.active_doc_mut() {
            let len = d.editor.buffer.len_chars();
            // An edit, so ⌘Z brings the current text back.
            d.editor.replace(0, len, &old);
        }
        app.vcs.diff = None;
        app.notify(tr("Restored; ⌘Z undoes it"), false);
    }
}

/// "Today 14:05:12", "Yesterday 09:30:00" or "2026-11-04 18:20".
pub(crate) fn format_time(t: std::time::SystemTime) -> String {
    let secs = t.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let offset = local_offset_secs();
    let (day, tod) = ((secs + offset).div_euclid(86400), (secs + offset).rem_euclid(86400));
    let today = (now + offset).div_euclid(86400);
    let hms = format!("{:02}:{:02}:{:02}", tod / 3600, tod / 60 % 60, tod % 60);
    match today - day {
        0 => format!("{} {hms}", tr("Today")),
        1 => format!("{} {hms}", tr("Yesterday")),
        _ => {
            let (y, m, d) = civil_from_days(day);
            format!("{y}-{m:02}-{d:02} {}", &hms[..5])
        }
    }
}

/// The local time zone's offset from UTC, in seconds.
fn local_offset_secs() -> i64 {
    #[cfg(unix)]
    {
        // SAFETY: localtime_r writes into the struct we own.
        unsafe {
            let now = libc::time(std::ptr::null_mut());
            let mut tm: libc::tm = std::mem::zeroed();
            libc::localtime_r(&now, &mut tm);
            tm.tm_gmtoff as i64
        }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
        // SAFETY: the function fills the struct we own; Bias is in minutes, UTC = local + bias.
        unsafe {
            let mut tz: TIME_ZONE_INFORMATION = std::mem::zeroed();
            let mode = GetTimeZoneInformation(&mut tz);
            let dst = if mode == 2 { tz.DaylightBias } else { tz.StandardBias };
            -((tz.Bias + dst) as i64) * 60
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        0
    }
}

/// Days since 1970-01-01 → (year, month, day) (Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Call / Type Hierarchy: direction toggles in the header, a lazily loaded tree.
fn hierarchy_window(app: &mut DotCraftApp, ui: &mut Ui) {
    use crate::hierarchy::{Direction, HierarchyKind};
    let p = rc_kit::pal(ui);
    let r = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), HEADER_H));
    ui.allocate_rect(r, Sense::hover());
    let title = match app.hierarchy.as_ref().map(|h| h.kind) {
        Some(HierarchyKind::Calls) => tr("Call Hierarchy"),
        Some(HierarchyKind::Types) => tr("Type Hierarchy"),
        None => tr("Hierarchy"),
    };
    let t = ui.painter().text(pos2(r.left() + 12.0, r.center().y), Align2::LEFT_CENTER, title, theme::bold(13.0), p.text);
    let br = Rect::from_min_size(pos2(r.right() - 8.0 - 26.0, r.center().y - 13.0), vec2(26.0, 26.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(br));
    if theme::icon_button(&mut child, rc_kit::ico::HIDE, true, tr("Hide")).clicked() {
        app.toggle_tool(ToolWindow::Hierarchy);
        return;
    }
    let Some(h) = &mut app.hierarchy else {
        let (title, hint) = empty_text(ToolWindow::Hierarchy);
        empty_state(ui, ToolWindow::Hierarchy.icon(), title, hint);
        return;
    };
    let mut x = t.right() + 14.0;
    let labels = match h.kind {
        HierarchyKind::Calls => [(tr("Callers"), Direction::Up), (tr("Callees"), Direction::Down)],
        HierarchyKind::Types => [(tr("Base Types"), Direction::Up), (tr("Derived Types"), Direction::Down)],
    };
    let mut new_dir = None;
    for (label, dir) in labels {
        let w = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(12.5), p.text).size().x + 32.0;
        let br = Rect::from_min_size(pos2(x, r.center().y - 12.0), vec2(w, 24.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(br));
        let icon = if dir == Direction::Up { rc_kit::ico::UP } else { rc_kit::ico::DOWN };
        if theme::tool_button(&mut child, icon, label, h.direction == dir).clicked() {
            new_dir = Some(dir);
        }
        x += w + 6.0;
    }
    let mut load: Vec<(usize, serde_json::Value)> = Vec::new();
    if let Some(dir) = new_dir
        && dir != h.direction
    {
        h.set_direction(dir);
        h.nodes[0].expanded = true;
        load.push((0, h.nodes[0].item.clone()));
    }
    let mut open: Option<(std::path::PathBuf, usize, usize)> = None;
    let rows = h.rows();
    egui::ScrollArea::both().id_salt("hierarchy").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for id in rows {
            let n = &h.nodes[id];
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(260.0), 24.0), Sense::click());
            let selected = h.selected == Some(id);
            if selected {
                ui.painter().rect_filled(rect.shrink2(vec2(6.0, 0.0)), 4.0, p.selection);
            } else if resp.hovered() {
                ui.painter().rect_filled(rect.shrink2(vec2(6.0, 0.0)), 4.0, p.row_hover);
            }
            let x0 = rect.left() + 10.0 + n.depth as f32 * 18.0;
            let leaf = n.children.as_ref().is_some_and(|c| c.is_empty());
            if !leaf {
                let chevron = if n.expanded { rc_kit::ico::CHEVRON_DOWN } else { rc_kit::ico::CHEVRON_RIGHT };
                icons::paint(ui.painter(), Rect::from_center_size(pos2(x0 + 8.0, rect.center().y), vec2(16.0, 16.0)), chevron, &p, true);
            }
            let kind: rc_lsp::lsp_types::SymbolKind = serde_json::from_value(serde_json::json!(n.kind)).unwrap_or(rc_lsp::lsp_types::SymbolKind::METHOD);
            icons::paint(ui.painter(), Rect::from_center_size(pos2(x0 + 26.0, rect.center().y), vec2(16.0, 16.0)), crate::convert::symbol_icon(kind), &p, true);
            let g = ui.painter().layout_no_wrap(n.name.clone(), FontId::proportional(13.0), p.text);
            let lx = x0 + 40.0;
            ui.painter().galley(pos2(lx, (rect.center().y - g.size().y / 2.0).round()), g.clone(), p.text);
            let file = n.path.as_ref().and_then(|p| p.file_name()).map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
            let secondary = if n.detail.is_empty() { format!("{file}:{}", n.line + 1) } else { format!("{}  {file}:{}", n.detail, n.line + 1) };
            ui.painter().text(pos2(lx + g.size().x + 8.0, rect.center().y), Align2::LEFT_CENTER, secondary, FontId::proportional(12.0), p.muted);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &n.name));
            let on_chevron = ui.input(|i| i.pointer.interact_pos()).is_some_and(|pt| pt.x < x0 + 18.0);
            if resp.clicked() {
                h.selected = Some(id);
            }
            if (resp.clicked() && on_chevron && !leaf) || (resp.double_clicked() && !leaf && n.children.is_none()) {
                let n = &mut h.nodes[id];
                n.expanded = !n.expanded;
                if n.expanded && n.children.is_none() {
                    load.push((id, n.item.clone()));
                }
            }
            if resp.double_clicked()
                && let Some(path) = &h.nodes[id].path
            {
                open = Some((path.clone(), h.nodes[id].line, h.nodes[id].col));
            }
        }
    });
    let (kind, dir, generation) = (h.kind, h.direction, h.generation);
    for (id, item) in load {
        app.insight.hierarchy_children(kind, dir, generation, id, &item);
    }
    if let Some((path, line, col)) = open {
        app.open_at(&path, line, col);
    }
}

/// TODO: Project / Current File tabs, a refresh button, the comments by file.
fn todo_window(app: &mut DotCraftApp, ui: &mut Ui) {
    let p = rc_kit::pal(ui);
    let now = ui.input(|i| i.time);
    let active = app.docs.active_doc().map(|d| d.path.clone());
    let Some(todos) = &mut app.todos else {
        if header(ui, "TODO") {
            app.toggle_tool(ToolWindow::Todo);
        }
        let (title, hint) = empty_text(ToolWindow::Todo);
        empty_state(ui, ToolWindow::Todo.icon(), title, hint);
        return;
    };
    // Rescan when the window shows again after a while (files changed meanwhile).
    if !todos.scanning && todos.scanned_at.is_none_or(|t| now - t > 30.0) {
        todos.refresh(now);
    }
    let r = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), HEADER_H));
    ui.allocate_rect(r, Sense::hover());
    let t = ui.painter().text(pos2(r.left() + 12.0, r.center().y), Align2::LEFT_CENTER, "TODO", theme::bold(13.0), p.text);
    let mut x = t.right() + 14.0;
    let mut hide = false;
    let mut refresh = false;
    for (label, current) in [(tr("Project"), false), (tr("Current File"), true)] {
        let w = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(12.5), p.text).size().x + 32.0;
        let br = Rect::from_min_size(pos2(x, r.center().y - 12.0), vec2(w, 24.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(br));
        if theme::tool_button(&mut child, if current { rc_kit::ico::FILE_TEXT } else { rc_kit::ico::TW_PROJECT }, label, todos.current_file == current).clicked() {
            todos.current_file = current;
        }
        x += w + 6.0;
    }
    if todos.scanning {
        ui.painter().text(pos2(x + 6.0, r.center().y), Align2::LEFT_CENTER, tr("Scanning…"), FontId::proportional(12.0), p.muted);
    } else {
        ui.painter().text(pos2(x + 6.0, r.center().y), Align2::LEFT_CENTER, rc_kit::i18n::trf("{n} items", &[("n", &todos.count())]), FontId::proportional(12.0), p.muted);
    }
    for (i, (icon, tip)) in [(rc_kit::ico::HIDE, tr("Hide")), (rc_kit::ico::REFRESH, tr("Refresh"))].into_iter().enumerate() {
        let br = Rect::from_min_size(pos2(r.right() - 8.0 - 26.0 * (i + 1) as f32, r.center().y - 13.0), vec2(26.0, 26.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(br));
        if theme::icon_button(&mut child, icon, true, tip).clicked() {
            if i == 0 { hide = true } else { refresh = true }
        }
    }
    if refresh {
        todos.refresh(now);
    }
    let files: Vec<crate::todo::TodoFile> = todos.files.iter().filter(|f| !todos.current_file || active.as_ref() == Some(&f.path)).cloned().collect();
    let mut open: Option<(std::path::PathBuf, usize, usize)> = None;
    if files.is_empty() && !todos.scanning {
        let (title, hint) = empty_text(ToolWindow::Todo);
        empty_state(ui, ToolWindow::Todo.icon(), title, hint);
    } else {
        egui::ScrollArea::vertical().id_salt("todo").auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for f in &files {
                let name = f.path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                let dir = f.rel.rsplit_once('/').map_or("", |(d, _)| d);
                list_row(ui, 0, rc_kit::icons::for_file(&name), &name, &format!("{dir}  {}", rc_kit::i18n::trf("{n} items", &[("n", &f.items.len())])), false);
                for item in &f.items {
                    let resp = list_row(ui, 1, rc_kit::ico::TW_TODO, &item.text, &format!(":{}", item.line + 1), false);
                    if resp.double_clicked() || resp.clicked() {
                        open = Some((f.path.clone(), item.line, item.col));
                    }
                }
            }
        });
    }
    if hide {
        app.toggle_tool(ToolWindow::Todo);
    }
    if let Some((path, line, col)) = open {
        app.open_at(&path, line, col);
    }
}

/// Bookmarks by file: double click opens, the context menu removes.
fn bookmarks(app: &mut DotCraftApp, ui: &mut Ui) {
    let marks = app.docs.all_bookmarks();
    if marks.is_empty() {
        let (title, hint) = empty_text(ToolWindow::Bookmarks);
        empty_state(ui, ToolWindow::Bookmarks.icon(), title, hint);
        return;
    }
    let root = app.workspace.as_ref().map(|w| w.ws.root.clone());
    let mut open: Option<(std::path::PathBuf, usize)> = None;
    let mut remove: Option<(std::path::PathBuf, usize)> = None;
    egui::ScrollArea::vertical().id_salt("bookmarks").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let mut last: Option<&std::path::Path> = None;
        for b in &marks {
            if last != Some(b.path.as_path()) {
                let name = b.path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                let dir = root.as_ref().and_then(|r| b.path.parent()?.strip_prefix(r).ok()).map(|d| d.display().to_string()).unwrap_or_default();
                list_row(ui, 0, rc_kit::icons::for_file(&name), &name, &dir, false);
                last = Some(b.path.as_path());
            }
            let text = match app.docs.index_of(&b.path) {
                Some(i) => app.docs.list[i].editor.buffer.line_text(b.line),
                None => std::fs::read_to_string(&b.path).ok().and_then(|t| t.lines().nth(b.line).map(str::to_string)).unwrap_or_default(),
            };
            let label = match b.mnemonic {
                Some(c) => format!("{c}  {}", text.trim()),
                None => text.trim().to_string(),
            };
            let resp = list_row(ui, 1, rc_kit::ico::BOOKMARK, &label, &format!(":{}", b.line + 1), false);
            if resp.double_clicked() {
                open = Some((b.path.clone(), b.line));
            }
            resp.context_menu(|ui| {
                if theme::menu_item(ui, Some(rc_kit::ico::BOOKMARK), tr("Go to Bookmark"), false).clicked() {
                    open = Some((b.path.clone(), b.line));
                    ui.close();
                }
                if theme::menu_item(ui, Some(rc_kit::ico::DELETE), tr("Remove Bookmark"), false).clicked() {
                    remove = Some((b.path.clone(), b.line));
                    ui.close();
                }
            });
        }
    });
    if let Some((path, line)) = open {
        app.open_at(&path, line, 0);
    }
    if let Some((path, line)) = remove {
        app.docs.set_bookmark(&path, line, None);
        app.session_dirty = true;
    }
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
    let Some(tree) = app.insight.structure.get(&doc.editor.id).filter(|tree| !tree.is_empty()).cloned() else {
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

#[cfg(test)]
mod structure_window_tests {
    use super::*;
    use crate::insight::SymbolNode;
    use egui_kittest::{Harness, kittest::Queryable};
    use rc_lsp::lsp_types::SymbolKind;

    #[test]
    fn shop_order_structure_shows_empty_state_and_navigates_members() {
        let _home = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        let shop = crate::screens::fixtures().join("Shop");
        let order = shop.join("src/Shop.Core/Order.cs");
        let source = std::fs::read_to_string(&order).unwrap();
        let line_of = |needle: &str| source.lines().position(|line| line.contains(needle)).unwrap();
        let class_line = line_of("public sealed class Order");
        let add_line = line_of("public Order Add(");
        let sample_line = line_of("public static Order Sample(");

        let mut app = crate::screens::test_app(home.path(), true);
        app.settings.roslyn_path = Some("/nonexistent".into());
        app.open_workspace(&shop.join("Shop.sln"));
        app.open_file(&order);
        let doc_id = app.docs.active_doc().unwrap().editor.id;
        app.insight.structure.insert(doc_id, Vec::new());

        let mut first = true;
        let mut harness = Harness::builder().with_size(vec2(500.0, 600.0)).build_ui_state(move |ui, app: &mut DotCraftApp| {
            if std::mem::take(&mut first) {
                rc_kit::theme::install_fonts(ui.ctx());
                return;
            }
            show(app, ui, ToolWindow::Structure);
        }, app);
        harness.run_steps(3);
        harness.get_by_label(tr("No structure"));

        harness.state_mut().insight.structure.insert(doc_id, vec![SymbolNode {
            name: "Order".into(),
            detail: String::new(),
            kind: SymbolKind::CLASS,
            line: class_line,
            col: 0,
            children: vec![
                SymbolNode { name: "Add".into(), detail: String::new(), kind: SymbolKind::METHOD, line: add_line, col: 0, children: Vec::new() },
                SymbolNode { name: "Sample".into(), detail: String::new(), kind: SymbolKind::METHOD, line: sample_line, col: 0, children: Vec::new() },
            ],
        }]);
        harness.run_steps(2);
        assert!(harness.query_by_label(tr("No structure")).is_none());
        harness.get_by_label("Order");
        harness.get_by_label("Sample");
        harness.get_by_label("Add").click();
        harness.run_steps(2);
        assert_eq!(harness.state().docs.active_doc().unwrap().editor.caret_line_col().0, add_line);
        assert!(harness.state().focus_editor);
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

fn find_in_files(app: &mut DotCraftApp, ui: &mut Ui) {
    let p = rc_kit::pal(ui);
    let Some(mut f) = app.find_in_files.take() else { return };
    f.poll();
    if f.searching() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    }
    let mut rerun = false;
    let mut replace_all = false;
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        let te = ui.add(egui::TextEdit::singleline(&mut f.query).hint_text(tr("Text to find")).desired_width(280.0));
        if std::mem::take(&mut f.focus) {
            te.request_focus();
        }
        rerun |= te.changed();
        for (flag, icon, tip) in [(&mut f.case, rc_kit::ico::MATCH_CASE, tr("Match Case")), (&mut f.word, rc_kit::ico::WHOLE_WORDS, tr("Words")), (&mut f.regex, rc_kit::ico::REGEX, tr("Regex"))] {
            if theme::icon_tab(ui, icon, tip, *flag).clicked() {
                *flag = !*flag;
                rerun = true;
            }
        }
        ui.add_space(8.0);
        ui.label(egui::RichText::new(tr("File mask:")).size(12.5).color(p.text_muted));
        rerun |= ui.add(egui::TextEdit::singleline(&mut f.mask).hint_text("*.cs").desired_width(120.0)).changed();
        if ui.add(egui::Button::selectable(f.show_replace, tr("Replace"))).clicked() {
            f.show_replace = !f.show_replace;
        }
    });
    if f.show_replace {
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            ui.add(egui::TextEdit::singleline(&mut f.replace).hint_text(tr("Replace with")).desired_width(280.0));
            replace_all = ui.add_enabled(f.match_count() > 0 && f.summary.is_some(), egui::Button::new(tr("Replace All"))).clicked();
        });
    }
    let status = match (&f.error, &f.summary) {
        (Some(e), _) => e.clone(),
        (None, Some(s)) => rc_kit::i18n::trf("{n} matches in {files} files", &[("n", &s.matches), ("files", &s.files_matched)]) + if s.truncated { " +" } else { "" },
        (None, None) if f.query.is_empty() => String::new(),
        (None, None) => tr("Searching…").to_string(),
    };
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.label(egui::RichText::new(status).size(12.5).color(if f.error.is_some() { p.error } else { p.text_muted }));
    });
    let root = f.root().to_path_buf();
    let mut open = None;
    egui::ScrollArea::vertical().id_salt("fif_results").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for file in &f.results {
            let name = file.path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            let dir = file.rel.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
            list_row(ui, 0, rc_kit::icons::for_file(&name), &name, &format!("{dir}  {}", file.match_count()), false);
            for l in &file.lines {
                // The line with its matches highlighted.
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(240.0), 24.0), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(r.shrink2(vec2(6.0, 0.0)), 4.0, p.row_hover);
                }
                let x = r.left() + 30.0;
                ui.painter().text(pos2(x, r.center().y), Align2::LEFT_CENTER, format!("{}", l.line + 1), FontId::monospace(12.0), p.muted);
                let text = l.text.trim_end();
                let lead = text.len() - text.trim_start().len();
                let lead_chars = text[..lead].chars().count() as u32;
                let mut job = egui::text::LayoutJob::default();
                let chars: Vec<char> = text.trim_start().chars().collect();
                let mut k = 0usize;
                for rg in &l.ranges {
                    let (a, b) = ((rg.start.saturating_sub(lead_chars)) as usize, (rg.end.saturating_sub(lead_chars)) as usize);
                    let (a, b) = (a.min(chars.len()), b.min(chars.len()));
                    if a > k {
                        job.append(&chars[k..a].iter().collect::<String>(), 0.0, egui::TextFormat { font_id: FontId::monospace(12.5), color: p.text, ..Default::default() });
                    }
                    if b > a {
                        job.append(&chars[a..b].iter().collect::<String>(), 0.0, egui::TextFormat { font_id: FontId::monospace(12.5), color: p.text, background: rc_kit::theme::scheme(p.dark).search_result, ..Default::default() });
                    }
                    k = b.max(k);
                }
                if k < chars.len() {
                    job.append(&chars[k..].iter().collect::<String>(), 0.0, egui::TextFormat { font_id: FontId::monospace(12.5), color: p.text, ..Default::default() });
                }
                let g = ui.painter().layout_job(job);
                ui.painter().galley(pos2(x + 44.0, r.center().y - g.size().y / 2.0), g, p.text);
                let accessible = format!("{}:{} {}", file.rel, l.line + 1, text.trim());
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &accessible));
                if resp.clicked() {
                    open = Some((file.path.clone(), l.line as usize, l.ranges.first().map_or(0, |r| r.start as usize)));
                }
            }
        }
    });
    if rerun {
        f.search();
    }
    if replace_all {
        if let Ok(q) = rc_search::Query::new(f.options()) {
            let mut files = 0;
            for file in &f.results {
                if let Some(i) = app.docs.index_of(&file.path) {
                    // Open files change through their editor (undoable).
                    let doc = &mut app.docs.list[i];
                    let edits = rc_search::compute_edits(&doc.editor.buffer.rope().to_string(), &q, &f.replace);
                    doc.editor.apply_text_edits(edits.into_iter().map(|e| rc_text::Change::new(e.range.start, e.range.end, e.text)).collect());
                    files += 1;
                } else if rc_search::replace_in_file(&file.path, &q, &f.replace).is_ok() {
                    files += 1;
                }
            }
            app.notify(rc_kit::i18n::trf("Replaced in {n} files", &[("n", &files)]), false);
        }
        f.search();
    }
    let _ = root;
    app.find_in_files = Some(f);
    if let Some((path, line, col)) = open {
        app.open_at(&path, line, col);
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
        ToolWindow::Nuget => ("NuGet", ""),
        ToolWindow::Database => (tr("Database"), ""),
        ToolWindow::Monitoring => ("", ""),
        ToolWindow::HttpClient => (tr("No requests sent"), tr("Open a .http file and click ▶ next to a request, or generate one from the Endpoints window.")),
        ToolWindow::Coverage => (tr("No coverage yet"), tr("Run the tests with coverage to see which lines they reach.")),
        ToolWindow::Endpoints => (tr("No endpoints"), tr("Controllers and minimal APIs of the solution show up here.")),
        ToolWindow::LocalHistory => (tr("No local history"), tr("A copy of the file is kept each time it is saved or changed by another app.")),
        ToolWindow::Hierarchy => (tr("No hierarchy"), tr("Press ⌃⌥H on a method for its calls, or ⌃H on a type for its base and derived types.")),
        ToolWindow::Todo => (tr("No TODO items"), tr("TODO and FIXME comments of the solution show up here.")),
        ToolWindow::Bookmarks => (tr("No bookmarks"), tr("Press F3 on a line to bookmark it, or ⌥F3 to give it a letter or digit.")),
        ToolWindow::Explorer => ("", ""),
        ToolWindow::PullRequests => ("", ""),
    }
}

/// Centered icon, title and hint.
pub fn empty_state(ui: &mut Ui, icon: Ico, title: &str, hint: &str) {
    empty_state_with(ui, icon, title, hint, None);
}

/// Empty state with a button under the hint (e.g. "Run All Tests"); true when clicked.
pub fn empty_state_with(ui: &mut Ui, icon: Ico, title: &str, hint: &str, button: Option<&str>) -> bool {
    let p = rc_kit::pal(ui);
    let rect = ui.available_rect_before_wrap();
    let c = rect.center() - vec2(0.0, 20.0);
    icons::paint_sized(ui.painter(), Rect::from_center_size(c - vec2(0.0, 30.0), vec2(16.0, 16.0)), icon, &p, false, 16.0);
    ui.painter().text(c, Align2::CENTER_CENTER, title, theme::bold(13.0), p.text_muted);
    let g = ui.painter().layout(hint.to_string(), FontId::proportional(12.5), p.muted, (rect.width() - 40.0).max(120.0));
    let pos = pos2(c.x - g.size().x / 2.0, c.y + 14.0);
    let below = pos.y + g.size().y + 14.0;
    ui.painter().galley(pos, g, p.muted);
    let mut clicked = false;
    if let Some(label) = button {
        let r = Rect::from_center_size(pos2(c.x, below + 14.0), vec2(220.0, 28.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r).layout(egui::Layout::top_down(egui::Align::Center)));
        clicked = theme::primary_button(&mut child, label).clicked();
    }
    ui.allocate_rect(rect, Sense::hover())
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, title));
    clicked
}

#[cfg(test)]
mod web_window_tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name();
            if matches!(name.to_str(), Some("bin" | "obj")) {
                continue;
            }
            let target = to.join(name);
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }

    fn pump(harness: &mut Harness<'_, DotCraftApp>, done: impl Fn(&DotCraftApp) -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !done(harness.state()) {
            assert!(std::time::Instant::now() < deadline, "web UI worker timed out");
            harness.run_steps(2);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn http_window_selects_refreshes_environments_and_cancels() {
        let _home = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("requests.http");
        let environment = dir.path().join(rc_dotnet::http_file::ENV_FILE);
        std::fs::write(&path, "GET {{host}}/items").unwrap();
        std::fs::write(&environment, r#"{"dev":{"host":"http://localhost"}}"#).unwrap();
        let mut app = crate::screens::test_app(dir.path(), true);
        app.docs.open(&path).unwrap();
        let mut first = true;
        let mut harness = Harness::builder().with_size(vec2(1000.0, 600.0)).build_ui_state(move |ui, app: &mut DotCraftApp| {
            if std::mem::take(&mut first) { rc_kit::theme::install_fonts(ui.ctx()); return; }
            // Cancellation button dispatch is tested separately from the socket lifecycle test.
            if !app.http.busy() { app.http.poll(); }
            http_client_window(app, ui);
        }, app);
        harness.run_steps(2);
        pump(&mut harness, |app| app.http.environments == ["dev"]);
        harness.get_by_label(tr("No environment")).click(); harness.run_steps(2);
        harness.get_by_label("dev").click(); harness.run_steps(2);
        assert_eq!(harness.state().http.env.as_deref(), Some("dev"));
        std::fs::write(&environment, r#"{"prod":{"host":"http://localhost"}}"#).unwrap();
        harness.get_by_label(tr("Refresh environments")).click(); harness.run_steps(2);
        pump(&mut harness, |app| app.http.environments == ["prod"]);
        assert!(harness.state().http.env.is_none());
        harness.state_mut().http.send(&path, "GET {{missing}}/items", 0).unwrap();
        harness.run_steps(2);
        harness.get_by_label(tr("Cancel Request")).click(); harness.run_steps(2);
        assert!(!harness.state().http.busy());
        assert_eq!(harness.state().http.response.as_ref().unwrap().as_ref().unwrap_err(), tr("Cancelled"));
    }

    #[test]
    fn endpoints_window_filters_groups_conflicts_and_imports_on_click() {
        let _home = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap(); let dir = tmp.path().canonicalize().unwrap(); let dir = &dir; let source = dir.join("api.json"); let output = dir.join("imported.http");
        std::fs::write(&source, r#"{"openapi":"3.1.0","servers":[{"url":"http://localhost:5000"}],"paths":{"/ready":{"get":{}}}}"#).unwrap();
        let mut app = crate::screens::test_app(dir, true);
        let endpoints = rc_dotnet::endpoints::scan_source(&dir.join("Program.cs"), r#"app.MapGet("/orders/{id}", GetOne); app.MapGet("/orders/{key}", GetTwo); app.MapPost("/create", Create);"#);
        let issues = rc_dotnet::endpoints::route_issues(&endpoints);
        app.endpoints.conflicts = issues.into_iter().map(|i| ("Api".into(), endpoints[i.first].clone(), endpoints[i.second].clone())).collect();
        app.endpoints.projects = vec![("Api".into(), dir.join("Api.csproj"), endpoints)];
        app.endpoints.import_path = source.display().to_string(); app.endpoints.output_path = output.display().to_string();
        let mut first = true;
        let mut harness = Harness::builder().with_size(vec2(1000.0, 800.0)).build_ui_state(move |ui, app: &mut DotCraftApp| {
            if std::mem::take(&mut first) { rc_kit::theme::install_fonts(ui.ctx()); return; }
            app.endpoints.poll(); endpoints_window(app, ui);
        }, app);
        harness.run_steps(3);
        harness.get_by_label("POST /create");
        harness.get_by_label(tr("Route conflicts only")).click(); harness.run_steps(2);
        assert!(harness.query_by_label("POST /create").is_none()); harness.get_by_label("GET /orders/{id}");
        harness.get_by_label(tr("Group by method")).click(); harness.run_steps(2);
        assert!(harness.state().endpoints.group_by_method); harness.get_by_label("GET");
        harness.state_mut().endpoints.filter = "GetTwo".into(); harness.run_steps(2);
        harness.get_by_label("GET /orders/{key}"); assert!(harness.query_by_label("GET /orders/{id}").is_none());
        harness.get_by_label(tr("Import OpenAPI JSON")).click(); harness.run_steps(2);
        assert!(!output.exists());
        harness.get_by_label(tr("Import requests")).click(); harness.run_steps(2);
        pump(&mut harness, |app| app.docs.active_doc().is_some_and(|doc| doc.path == output));
        let generated = std::fs::read_to_string(&output).unwrap();
        assert_eq!(rc_dotnet::http_file::parse(&generated).requests.len(), 1);
        assert!(!harness.state().http.busy(), "import must not send a request");
    }

    #[test]
    fn shop_endpoints_scan_show_handlers_and_generate_request() {
        let _home = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let temp = tempfile::tempdir().unwrap();
        let shop = temp.path().join("Shop");
        copy_tree(&crate::screens::fixtures().join("Shop"), &shop);
        let mut app = crate::screens::test_app(temp.path(), true);
        app.settings.roslyn_path = Some("/nonexistent".into());
        app.open_workspace(&shop.join("Shop.sln"));

        let mut first = true;
        let mut harness = Harness::builder().with_size(vec2(1100.0, 800.0)).build_ui_state(move |ui, app: &mut DotCraftApp| {
            if std::mem::take(&mut first) {
                rc_kit::theme::install_fonts(ui.ctx());
                return;
            }
            app.endpoints.poll();
            endpoints_window(app, ui);
        }, app);
        harness.run_steps(3);
        pump(&mut harness, |app| !app.endpoints.scanning);
        let project = harness.state().endpoints.projects.iter().find(|(name, _, _)| name == "Shop.Api").unwrap();
        assert_eq!(project.2.len(), 4);
        let sample = project.2.iter().find(|endpoint| endpoint.route == "/Orders/sample").unwrap().clone();
        for route in ["GET /Orders/sample", "POST /Orders/quote", "GET /WeatherForecast", "GET /health"] {
            harness.get_by_label(route);
        }
        harness.state_mut().endpoints.method = "POST".into();
        harness.run_steps(2);
        assert!(harness.query_by_label("GET /health").is_none());
        harness.get_by_label("POST /Orders/quote");
        harness.state_mut().endpoints.method.clear();
        harness.run_steps(2);

        harness.get_by_label("GET /Orders/sample").click_secondary();
        harness.run_steps(2);
        harness.get_by_label(tr("Go to Handler")).click();
        harness.run_steps(2);
        let doc = harness.state().docs.active_doc().unwrap();
        assert_eq!(doc.path, sample.file);
        assert_eq!(doc.editor.caret_line_col().0, sample.line);

        let requests = shop.join("src/Shop.Api/Shop.Api.http");
        let before = std::fs::read_to_string(&requests).unwrap();
        harness.get_by_label("GET /health").click_secondary();
        harness.run_steps(2);
        harness.get_by_label(tr("Generate Request in HTTP Client")).click();
        pump(&mut harness, |app| app.docs.active_doc().is_some_and(|doc| doc.path == requests));
        let after = std::fs::read_to_string(&requests).unwrap();
        assert!(after.starts_with(&before));
        assert!(after.contains("GET {{Shop.Api_HostAddress}}/health"));
    }
}
