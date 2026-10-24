//! Build, Run, Tests and Terminal tool windows.

use crate::app::DotCraftApp;
use crate::runner::{RunnerOutput, TestState};
use crate::toolwindow::ToolWindow;
use crate::ui::toolwindows::{HEADER_H, empty_state};
use egui::{Align2, FontId, Rect, Sense, Ui, pos2, vec2};
use rc_dotnet::TestOutcome;
use rc_kit::i18n::{tr, trf};
use rc_kit::{Ico, ico, icons, theme};

/// Header with a title on the left and icon actions on the right; returns the clicked action
/// index (or `usize::MAX` for "Hide").
fn header(ui: &mut Ui, title: &str, actions: &[(Ico, &str, bool)]) -> Option<usize> {
    let p = rc_kit::pal(ui);
    let r = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), HEADER_H));
    ui.allocate_rect(r, Sense::hover());
    ui.painter().text(pos2(r.left() + 12.0, r.center().y), Align2::LEFT_CENTER, title, theme::bold(13.0), p.text);
    let title_w = ui.painter().layout_no_wrap(title.to_string(), theme::bold(13.0), p.text).size().x;
    let mut clicked = None;
    let mut x = r.left() + 12.0 + title_w + 10.0;
    for (i, (icon, tip, enabled)) in actions.iter().enumerate() {
        let br = Rect::from_min_size(pos2(x, r.center().y - 13.0), vec2(26.0, 26.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(br));
        if theme::icon_button(&mut child, *icon, *enabled, tip).clicked() {
            clicked = Some(i);
        }
        x += 28.0;
    }
    let br = Rect::from_min_size(pos2(r.right() - 34.0, r.center().y - 13.0), vec2(26.0, 26.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(br));
    if theme::icon_button(&mut child, ico::HIDE, true, tr("Hide")).clicked() {
        clicked = Some(usize::MAX);
    }
    clicked
}

fn console(ui: &mut Ui, term: &rc_term::TermSession) -> Vec<rc_term::ViewEvent> {
    let config = rc_term::TermConfig::default();
    rc_term::TerminalView::new(term, &config).show(ui).events
}

pub fn handle_outputs(app: &mut DotCraftApp, outputs: Vec<RunnerOutput>) {
    for o in outputs {
        match o {
            RunnerOutput::ShowTool(tw) => app.dock.show(tw),
            RunnerOutput::Notify(m, err) => app.notify(m, err),
            RunnerOutput::BuildDiagnostics(list) => app.set_build_problems(list),
        }
    }
}

fn open_link(app: &mut DotCraftApp, link: rc_term::Link, cwd: Option<std::path::PathBuf>) {
    match link {
        rc_term::Link::Url(u) => open_url(&u),
        rc_term::Link::File { path, line, column } => {
            let path = std::path::PathBuf::from(path);
            let p = if path.is_absolute() { path } else { cwd.unwrap_or_default().join(path) };
            if p.is_file() {
                app.open_at(&p, line.unwrap_or(1).saturating_sub(1) as usize, column.unwrap_or(1).saturating_sub(1) as usize);
            }
        }
    }
}

pub fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

// ---------------------------------------------------------------------------------- Build

pub fn build(app: &mut DotCraftApp, ui: &mut Ui) {
    let running = app.runner.build.as_ref().is_some_and(|b| b.handle.is_some());
    let target = app.workspace.as_ref().map(|w| w.ws.key_path().to_path_buf());
    let actions = [(ico::BUILD, tr("Build Solution"), !running && target.is_some()), (ico::REBUILD, tr("Rebuild Solution"), !running && target.is_some()), (ico::STOP, tr("Cancel Build"), running)];
    match header(ui, tr("Build"), &actions) {
        Some(0) => app.pending.push(rc_editor::Action::Build),
        Some(1) => {
            if let Some(t) = target {
                let outs = app.runner.build(&t, rc_dotnet::BuildAction::Rebuild);
                handle_outputs(app, outs);
            }
        }
        Some(2) => app.runner.cancel_build(),
        Some(usize::MAX) => app.toggle_tool(ToolWindow::Build),
        _ => {}
    }
    let Some(b) = &app.runner.build else {
        empty_state(ui, ToolWindow::Build.icon(), tr("No build output"), tr("Build the solution to see errors and warnings."));
        return;
    };
    let p = rc_kit::pal(ui);
    // Summary line.
    let (icon, text) = match (&b.result, &b.failed) {
        (_, Some(e)) => (ico::ERROR, e.clone()),
        (Some(r), _) if r.cancelled => (ico::WARNING, tr("Build cancelled").to_string()),
        (Some(r), _) if r.success => (ico::SUCCESS, trf("Build succeeded with {warnings} warnings in {secs} s", &[("warnings", &r.warnings), ("secs", &format!("{:.1}", r.duration.as_secs_f32()))])),
        (Some(r), _) => (ico::ERROR, trf("Build failed: {errors} errors, {warnings} warnings", &[("errors", &r.errors), ("warnings", &r.warnings)])),
        (None, None) => (ico::BUILD, trf("{title} in progress… {secs} s", &[("title", &b.title), ("secs", &b.started.elapsed().as_secs())])),
    };
    if b.handle.is_some() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
    }
    let r = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), 26.0));
    ui.allocate_rect(r, Sense::hover());
    icons::paint(ui.painter(), Rect::from_center_size(pos2(r.left() + 20.0, r.center().y), vec2(16.0, 16.0)), icon, &p, true);
    ui.painter().text(pos2(r.left() + 36.0, r.center().y), Align2::LEFT_CENTER, text, FontId::proportional(13.0), p.text);
    // Diagnostics list (clickable) above the raw output.
    let diags = b.diagnostics.clone();
    let mut open = None;
    if !diags.is_empty() {
        let h = (diags.len() as f32 * 24.0 + 4.0).min(ui.available_height() * 0.45);
        egui::ScrollArea::vertical().id_salt("build_diags").max_height(h).auto_shrink([false, true]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for d in &diags {
                let icon = if d.severity == rc_dotnet::Severity::Error { ico::ERROR } else { ico::WARNING };
                let (rr, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(rr.shrink2(vec2(6.0, 0.0)), 4.0, p.row_hover);
                }
                icons::paint(ui.painter(), Rect::from_center_size(pos2(rr.left() + 30.0, rr.center().y), vec2(16.0, 16.0)), icon, &p, true);
                let file = d.file.as_ref().and_then(|f| f.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let loc = match (d.line, d.column) {
                    (Some(l), Some(c)) => format!("{file}({l},{c})"),
                    (Some(l), None) => format!("{file}({l})"),
                    _ => file,
                };
                let g = ui.painter().layout_no_wrap(format!("{} {}", d.code, d.message), FontId::proportional(13.0), p.text);
                ui.painter().galley(pos2(rr.left() + 46.0, rr.center().y - g.size().y / 2.0), g.clone(), p.text);
                ui.painter().text(pos2(rr.left() + 54.0 + g.size().x, rr.center().y), Align2::LEFT_CENTER, loc, FontId::proportional(12.0), p.muted);
                if resp.clicked()
                    && let Some(f) = &d.file
                {
                    open = Some((f.clone(), d.line.unwrap_or(1).saturating_sub(1) as usize, d.column.unwrap_or(1).saturating_sub(1) as usize));
                }
            }
        });
        ui.separator();
    }
    let cwd = app.workspace.as_ref().map(|w| w.ws.root.clone());
    let events = console(ui, &app.runner.build.as_ref().expect("checked").console.term);
    if let Some((f, l, c)) = open {
        app.open_at(&f, l, c);
    }
    for e in events {
        if let rc_term::ViewEvent::LinkClicked(link) = e {
            open_link(app, link, cwd.clone());
        }
    }
}

// ---------------------------------------------------------------------------------- Run

pub fn run(app: &mut DotCraftApp, ui: &mut Ui) {
    let p = rc_kit::pal(ui);
    let active = app.runner.active_run;
    let running = app.runner.runs.get(active).is_some_and(|r| r.running());
    let url = app.runner.runs.get(active).and_then(|r| r.urls.first().cloned());
    let actions = [(ico::RERUN, tr("Rerun"), !app.runner.runs.is_empty()), (ico::STOP, tr("Stop"), running), (ico::EXTERNAL_LINK, tr("Open in Browser"), url.is_some())];
    match header(ui, tr("Run"), &actions) {
        Some(0) => {
            if let Some(cfg) = app.runner.runs.get(active).map(|r| r.config.clone())
                && let Some(i) = app.runner.configs.iter().position(|c| *c == cfg)
            {
                app.runner.selected = i;
                let outs = app.runner.run_selected();
                handle_outputs(app, outs);
            }
        }
        Some(1) => {
            if let Some(h) = app.runner.runs.get(active).and_then(|r| r.handle.as_ref()) {
                h.stop();
            }
        }
        Some(2) => {
            if let Some(u) = url {
                open_url(&u.replace("0.0.0.0", "localhost").replace("[::]", "localhost").replace("*", "localhost"));
            }
        }
        Some(usize::MAX) => app.toggle_tool(ToolWindow::Run),
        _ => {}
    }
    if app.runner.runs.is_empty() {
        empty_state(ui, ToolWindow::Run.icon(), tr("Nothing is running"), tr("Run a project to see its output here."));
        return;
    }
    // One tab per run configuration.
    let mut close = None;
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        for (i, r) in app.runner.runs.iter().enumerate() {
            let (resp, closed) = theme::tab(ui, i == active, Some(r.config.icon()), &r.name, true);
            if resp.clicked() {
                app.runner.active_run = i;
            }
            if closed {
                close = Some(i);
            }
        }
    });
    let _ = p;
    if let Some(i) = close {
        app.runner.close_run(i);
        return;
    }
    let cwd = app.runner.runs.get(app.runner.active_run).and_then(|r| match &r.config.kind {
        crate::runner::ConfigKind::App { project, .. } => project.parent().map(|d| d.to_path_buf()),
        _ => None,
    });
    let Some(r) = app.runner.runs.get(app.runner.active_run) else { return };
    let events = console(ui, &r.console.term);
    for e in events {
        if let rc_term::ViewEvent::LinkClicked(link) = e {
            open_link(app, link, cwd.clone());
        }
    }
}

// ---------------------------------------------------------------------------------- Tests

pub fn tests(app: &mut DotCraftApp, ui: &mut Ui) {
    let running = app.runner.tests.as_ref().is_some_and(|t| t.running);
    let has = app.runner.tests.is_some();
    let show_passed = app.runner.tests.as_ref().is_none_or(|t| t.show_passed);
    let actions = [
        (ico::RERUN, tr("Rerun"), has && !running),
        (ico::RESTART_FAILED, tr("Rerun Failed Tests"), has && !running),
        (ico::STOP, tr("Stop"), running),
        (if show_passed { ico::SHOW_PASSED } else { ico::SHOW_IGNORED }, tr("Show Passed"), has),
    ];
    match header(ui, tr("Tests"), &actions) {
        Some(0) => {
            let outs = app.runner.rerun_tests();
            handle_outputs(app, outs);
        }
        Some(1) => {
            let outs = app.runner.rerun_failed();
            handle_outputs(app, outs);
        }
        Some(2) => {
            if let Some(t) = &mut app.runner.tests
                && let Some(h) = t.handle.take()
            {
                h.cancel();
            }
        }
        Some(3) => {
            if let Some(t) = &mut app.runner.tests {
                t.show_passed = !t.show_passed;
            }
        }
        Some(usize::MAX) => app.toggle_tool(ToolWindow::Tests),
        _ => {}
    }
    let Some(t) = &app.runner.tests else {
        empty_state(ui, ToolWindow::Tests.icon(), tr("No tests run yet"), tr("Run the tests of a project or file to see the results."));
        return;
    };
    let p = rc_kit::pal(ui);
    if t.running {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(300));
    }
    // Summary.
    let summary = if t.running {
        trf("Running… {passed} passed, {failed} failed", &[("passed", &t.passed), ("failed", &t.failed)])
    } else if let Some(e) = &t.failure {
        e.clone()
    } else {
        let secs = t.duration.map_or(0.0, |d| d.as_secs_f32());
        trf("{passed} passed, {failed} failed, {skipped} skipped · {secs} s", &[("passed", &t.passed), ("failed", &t.failed), ("skipped", &t.skipped), ("secs", &format!("{secs:.1}"))])
    };
    let icon = if t.running { ico::TEST_PAUSED } else if t.failed > 0 || t.failure.is_some() { ico::TEST_FAILED } else { ico::TEST_PASSED };
    let r = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), 26.0));
    ui.allocate_rect(r, Sense::hover());
    icons::paint(ui.painter(), Rect::from_center_size(pos2(r.left() + 20.0, r.center().y), vec2(16.0, 16.0)), icon, &p, true);
    ui.painter().text(pos2(r.left() + 36.0, r.center().y), Align2::LEFT_CENTER, summary, FontId::proportional(13.0), p.text);

    // Tree on the left (class → tests), details or console on the right.
    let avail = ui.available_rect_before_wrap();
    let left_w = (avail.width() * 0.45).clamp(240.0, 520.0);
    let left = Rect::from_min_size(avail.min, vec2(left_w, avail.height()));
    let right = Rect::from_min_max(pos2(left.right() + 1.0, avail.top()), avail.max);
    ui.painter().vline(left.right() + 0.5, avail.y_range(), egui::Stroke::new(1.0, p.island_border));
    let mut classes: std::collections::BTreeMap<String, Vec<(String, TestState, std::time::Duration)>> = std::collections::BTreeMap::new();
    for (name, results) in &t.results {
        let state = if results.iter().any(|r| r.outcome == TestOutcome::Failed) {
            TestState::Failed
        } else if results.iter().all(|r| r.outcome == TestOutcome::Skipped) {
            TestState::Skipped
        } else {
            TestState::Passed
        };
        if state == TestState::Passed && !t.show_passed {
            continue;
        }
        let class = results.first().map(|r| r.class_name.clone()).unwrap_or_default();
        let dur = results.iter().map(|r| r.duration).sum();
        classes.entry(class).or_default().push((name.clone(), state, dur));
    }
    let mut select = None;
    let mut open_test: Option<String> = None;
    let mut lu = ui.new_child(egui::UiBuilder::new().max_rect(left));
    lu.set_clip_rect(left.intersect(ui.clip_rect()));
    egui::ScrollArea::vertical().id_salt("tests_tree").auto_shrink([false, false]).show(&mut lu, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for (class, tests) in &classes {
            let failed = tests.iter().any(|t| t.1 == TestState::Failed);
            let short = class.rsplit('.').next().unwrap_or(class);
            row(ui, 0, if failed { ico::TEST_FAILED } else { ico::TEST_PASSED }, short, &class[..class.len() - short.len()].trim_end_matches('.').to_string(), false);
            for (name, state, dur) in tests {
                let icon = match state {
                    TestState::Failed => ico::TEST_FAILED,
                    TestState::Skipped => ico::TEST_IGNORED,
                    TestState::Running => ico::TEST_PAUSED,
                    TestState::Passed => ico::TEST_PASSED,
                };
                let method = name.rsplit('.').next().unwrap_or(name);
                let resp = row(ui, 1, icon, method, &format!("{} ms", dur.as_millis()), t.selected.as_deref() == Some(name.as_str()));
                if resp.clicked() {
                    select = Some(name.clone());
                }
                if resp.double_clicked() {
                    open_test = Some(name.clone());
                }
            }
        }
        if classes.is_empty() && !t.running {
            ui.add_space(12.0);
            ui.label(egui::RichText::new(tr("No results")).color(p.muted));
        }
    });
    let selected_results: Option<Vec<rc_dotnet::TestResult>> = t.selected.as_ref().and_then(|s| t.results.get(s).cloned());
    let mut ru = ui.new_child(egui::UiBuilder::new().max_rect(right.shrink2(vec2(8.0, 0.0))));
    ru.set_clip_rect(right.intersect(ui.clip_rect()));
    let mut open_frame = None;
    match selected_results {
        Some(results) if results.iter().any(|r| r.error_message.is_some() || !r.stack_trace.is_empty() || !r.stdout.is_empty()) => {
            egui::ScrollArea::vertical().id_salt("test_details").auto_shrink([false, false]).show(&mut ru, |ui| {
                for r in &results {
                    ui.label(egui::RichText::new(&r.display_name).font(theme::bold(13.0)));
                    if let Some(m) = &r.error_message {
                        ui.label(egui::RichText::new(m).font(FontId::monospace(12.5)).color(p.error));
                    }
                    for f in &r.stack_trace {
                        let text = egui::RichText::new(&f.text).font(FontId::monospace(12.0));
                        if let (Some(file), Some(line)) = (&f.file, f.line) {
                            if ui.add(egui::Label::new(text.color(p.accent)).sense(Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                open_frame = Some((file.clone(), line.saturating_sub(1) as usize));
                            }
                        } else {
                            ui.label(text.color(p.muted));
                        }
                    }
                    if !r.stdout.trim().is_empty() {
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new(r.stdout.trim()).font(FontId::monospace(12.0)));
                    }
                    ui.add_space(10.0);
                }
            });
        }
        _ => {
            let term = &t.console.term;
            let config = rc_term::TermConfig::default();
            rc_term::TerminalView::new(term, &config).show(&mut ru);
        }
    }
    ui.advance_cursor_after_rect(avail);
    if let Some(s) = select
        && let Some(t) = &mut app.runner.tests
    {
        t.selected = Some(s);
    }
    if let Some((f, l)) = open_frame {
        app.open_at(&f, l, 0);
    }
    if let Some(name) = open_test {
        app.goto_test(&name);
    }
}

fn row(ui: &mut Ui, depth: usize, icon: Ico, text: &str, secondary: &str, selected: bool) -> egui::Response {
    let p = rc_kit::pal(ui);
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(200.0), 24.0), Sense::click());
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

// ---------------------------------------------------------------------------------- Terminal

pub fn terminal(app: &mut DotCraftApp, ui: &mut Ui) {
    if app.terminals.tabs().is_empty() {
        app.open_terminal(None);
    }
    let actions = [(ico::ADD, tr("New Terminal Tab"), true)];
    match header(ui, tr("Terminal"), &actions) {
        Some(0) => app.open_terminal(None),
        Some(usize::MAX) => app.toggle_tool(ToolWindow::Terminal),
        _ => {}
    }
    let titles = app.terminals.titles();
    let active = app.terminals.active_index();
    let mut close = None;
    if titles.len() > 1 {
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            for (i, title) in titles.iter().enumerate() {
                let (resp, closed) = theme::tab(ui, i == active, Some(ico::FILE_SHELL), title, true);
                if resp.clicked() {
                    app.terminals.set_active(i);
                }
                if closed {
                    close = Some(app.terminals.tabs()[i].id());
                }
            }
        });
    }
    if let Some(id) = close {
        app.terminals.close(id);
        return;
    }
    let cwd = app.terminals.active().and_then(|t| t.session.cwd().map(|p| p.to_path_buf()));
    let focus = std::mem::take(&mut app.focus_terminal);
    let Some(tab) = app.terminals.active() else { return };
    let config = rc_term::TermConfig::default();
    let out = rc_term::TerminalView::new(&tab.session, &config).focus(focus).show(ui);
    for e in out.events {
        if let rc_term::ViewEvent::LinkClicked(link) = e {
            open_link(app, link, cwd.clone());
        }
    }
}
