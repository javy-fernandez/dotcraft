//! Structural Search and Replace: the pattern dialog and the results in the Find tool window.

use std::collections::HashMap;

use egui::{Align2, Color32, Context, FontId, Rect, Sense, TextEdit, Ui, pos2, vec2};
use rc_kit::i18n::{tr, trf};
use rc_kit::{ico, icons, theme};
use rc_search::structural::{Count, NodeKind};

use super::{dialogs, kit};
use crate::app::DotCraftApp;
use crate::toolwindow::ToolWindow;

/// Every frame: takes finished searches and replacements, reruns after a replacement and shows
/// the pattern dialog when it is open.
pub fn show(app: &mut DotCraftApp, ctx: &Context) {
    if let Some(done) = app.structural.update(&mut app.docs) {
        report(app, done);
    }
    if std::mem::take(&mut app.structural.rerun) {
        search(app);
    }
    if app.structural.dialog {
        dialog(app, ctx);
    }
}

/// Starts a search over the workspace's C# files, open documents as edited.
pub fn search(app: &mut DotCraftApp) {
    let Some(snapshot) = app.files.index.as_ref().map(|index| index.snapshot()) else {
        app.structural.error = Some(tr("Open a workspace to search C# files").to_string());
        return;
    };
    let open: HashMap<_, _> = app
        .docs
        .list
        .iter()
        .filter(|doc| doc.path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("cs")))
        .map(|doc| (doc.path.clone(), doc.editor.buffer.rope().to_string()))
        .collect();
    app.structural.search(snapshot, open);
}

fn report(app: &mut DotCraftApp, done: crate::structural_search::Replaced) {
    let text = trf("Replaced {n} matches in {files} files", &[("n", &done.matches), ("files", &done.files)]);
    match done.error {
        Some(error) => {
            app.notify(format!("{text}. {error}"), true);
            app.structural.error = Some(error);
        }
        None => app.notify(text, false),
    }
}

fn kind_label(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Any => tr("Any syntax"),
        NodeKind::Identifier => tr("Identifier"),
        NodeKind::Expression => tr("Expression"),
        NodeKind::Statement => tr("Statement"),
        NodeKind::Type => tr("Type"),
    }
}

fn count_label(count: Count) -> &'static str {
    match count {
        Count::One => tr("Exactly one"),
        Count::ZeroOrOne => tr("Zero or one"),
        Count::OneOrMore => tr("One or more"),
        Count::Any => tr("Any number"),
    }
}

/// The pattern dialog: search and replace templates and each placeholder's constraints.
fn dialog(app: &mut DotCraftApp, ctx: &Context) {
    let width = (ctx.content_rect().width() - 48.0).clamp(360.0, 680.0);
    let s = &mut app.structural;
    let mut find = false;
    let mut cancel = false;
    let ((), escape) = dialogs::modal(ctx, "structural_search", width, |ui| {
        dialogs::title(ui, tr("Structural Search and Replace"));
        let p = rc_kit::pal(ui);
        let caption = |ui: &mut Ui, text: &str| ui.label(egui::RichText::new(text).size(12.0).color(p.muted));
        let label = caption(ui, tr("Search template"));
        let query = ui.add(TextEdit::multiline(&mut s.query).code_editor().desired_rows(3).desired_width(f32::INFINITY).hint_text("Console.WriteLine($args$)")).labelled_by(label.id);
        if std::mem::take(&mut s.focus) {
            query.request_focus();
        }
        ui.add_space(8.0);
        let label = caption(ui, tr("Replace template"));
        ui.add(TextEdit::multiline(&mut s.replacement).code_editor().desired_rows(3).desired_width(f32::INFINITY).hint_text("Log.Information($args$)")).labelled_by(label.id);
        ui.add_space(4.0);
        kit::note(ui, tr("Write $name$ for a placeholder, $name$; or { $name$ } for statements. Leave the replacement empty to only search."));
        let names = s.placeholder_names();
        if !names.is_empty() {
            ui.add_space(10.0);
            ui.label(egui::RichText::new(tr("Placeholders")).font(theme::bold(12.5)));
            ui.add_space(4.0);
            egui::Grid::new("structural_constraints").num_columns(4).spacing([10.0, 8.0]).show(ui, |ui| {
                for name in &names {
                    let constraint = s.constraints.entry(name.clone()).or_default();
                    ui.label(egui::RichText::new(format!("${name}$")).font(FontId::monospace(12.5)));
                    let kinds: Vec<String> = NodeKind::ALL.iter().map(|kind| kind_label(*kind).to_string()).collect();
                    if let Some(i) = kit::dropdown(ui, ("structural_kind", name), kind_label(constraint.kind), &kinds, true) {
                        constraint.kind = NodeKind::ALL[i];
                    }
                    let counts: Vec<String> = Count::ALL.iter().map(|count| count_label(*count).to_string()).collect();
                    if let Some(i) = kit::dropdown(ui, ("structural_count", name), count_label(constraint.count), &counts, true) {
                        constraint.count = Count::ALL[i];
                    }
                    let regex = constraint.text_regex.get_or_insert_with(String::new);
                    ui.add(TextEdit::singleline(regex).hint_text(tr("Text matches regex")).desired_width(180.0).margin(vec2(6.0, 4.0)));
                    ui.end_row();
                }
            });
        }
        if let Some(error) = &s.error {
            ui.add_space(6.0);
            kit::message(ui, error, true);
        }
        ui.add_space(14.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            find = ui.add_enabled_ui(!s.query.trim().is_empty(), |ui| theme::primary_button(ui, tr("Find"))).inner.clicked();
            cancel = theme::secondary_button(ui, tr("Cancel")).clicked();
        });
        if ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command) && !s.query.trim().is_empty() {
            find = true;
        }
    });
    if find {
        match s.compile() {
            Ok(_) => {
                s.dialog = false;
                s.shown = true;
                search(app);
                app.dock.show(ToolWindow::Find);
            }
            Err(error) => s.error = Some(error),
        }
    } else if cancel || escape {
        s.dialog = false;
        s.error = None;
        app.focus_editor = true;
    }
}

/// The results in the Find tool window: matches grouped by file, each with a checkbox and
/// its replacement; a click opens the match.
pub fn window(app: &mut DotCraftApp, ui: &mut Ui) {
    let p = rc_kit::pal(ui);
    let s = &app.structural;
    let working = s.busy || s.applying;
    let can_replace = !working && s.replacing && s.selected() > 0;
    let actions = [(ico::EDIT, tr("Edit Pattern…"), !s.applying), (ico::REFRESH, tr("Search Again"), !working), (ico::STOP, tr("Stop"), s.busy)];
    let clicked = crate::ui::run_windows::header(ui, tr("Structural Search"), &actions);
    let mut replace = false;
    let mut select = None;
    kit::toolbar(ui, |ui| {
        if s.replacing {
            replace = kit::button(ui, Some(ico::REPLACE), &trf("Replace {n} Selected", &[("n", &s.selected())]), can_replace).clicked();
        }
        if kit::button(ui, Some(ico::CHECKMARK), tr("Select All"), !working && !s.hits.is_empty()).clicked() {
            select = Some(true);
        }
        if kit::button(ui, None, tr("Select None"), !working && !s.hits.is_empty()).clicked() {
            select = Some(false);
        }
        let status = if s.applying {
            tr("Replacing…").to_string()
        } else if s.busy {
            tr("Searching…").to_string()
        } else {
            trf("{n} matches in {files} files", &[("n", &s.hits.len()), ("files", &s.files)]) + if s.truncated { " +" } else { "" }
        };
        kit::note(ui, &status);
    });
    kit::content(ui, |ui| {
        ui.label(egui::RichText::new(s.query.trim()).font(FontId::monospace(12.0)).color(p.muted));
        if let Some(error) = &s.error {
            kit::message(ui, error, true);
        }
    });
    let mut open = None;
    let mut toggle = None;
    let s = &app.structural;
    egui::ScrollArea::vertical().id_salt("structural_results").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        if s.hits.is_empty() && !working && s.error.is_none() {
            ui.add_space(8.0);
            kit::content(ui, |ui| kit::note(ui, tr("Nothing found")));
        }
        let mut i = 0;
        while i < s.hits.len() {
            let rel = &s.hits[i].rel;
            let count = s.hits[i..].iter().take_while(|hit| &hit.rel == rel).count();
            let (dir, name) = rel.rsplit_once('/').unwrap_or(("", rel));
            kit::row(ui, 0, Some(icons::for_file(name)), name, &format!("{dir}  {count}"), false);
            for (index, hit) in s.hits.iter().enumerate().skip(i).take(count) {
                if let Some(action) = hit_row(ui, hit, s.replacing, !working) {
                    match action {
                        HitClick::Toggle => toggle = Some(index),
                        HitClick::Open => open = Some((hit.path.clone(), hit.line, hit.col)),
                    }
                }
            }
            i += count;
        }
    });
    let s = &mut app.structural;
    if let Some(index) = toggle {
        s.hits[index].selected = !s.hits[index].selected;
    }
    if let Some(on) = select {
        s.hits.iter_mut().for_each(|hit| hit.selected = on);
    }
    match clicked {
        Some(0) => {
            s.dialog = true;
            s.focus = true;
            s.error = None;
        }
        Some(1) => search(app),
        Some(2) => app.structural.stop(),
        Some(usize::MAX) => app.toggle_tool(ToolWindow::Find),
        _ => {}
    }
    if replace {
        match app.structural.apply(&mut app.docs) {
            Ok(Some(done)) => report(app, done),
            Ok(None) => {}
            Err(error) => app.structural.error = Some(error),
        }
    }
    if let Some((path, line, col)) = open {
        app.open_at(&path, line, col);
    }
}

enum HitClick {
    Toggle,
    Open,
}

/// One match: checkbox, line number, the matched code and, when replacing, what it becomes.
fn hit_row(ui: &mut Ui, hit: &crate::structural_search::Hit, replacing: bool, enabled: bool) -> Option<HitClick> {
    let p = rc_kit::pal(ui);
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(240.0), 24.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r.shrink2(vec2(6.0, 0.0)), 4.0, p.row_hover);
    }
    let cb = Rect::from_center_size(pos2(r.left() + 40.0, r.center().y), vec2(14.0, 14.0));
    let on = hit.selected;
    ui.painter().rect(cb, 3.0, if on { p.accent } else { p.control_bg }, egui::Stroke::new(1.0, if on { p.accent } else { p.control_border }), egui::StrokeKind::Inside);
    if on {
        let stroke = egui::Stroke::new(1.6, Color32::WHITE);
        ui.painter().line_segment([cb.left_center() + vec2(3.0, 0.0), cb.center_bottom() + vec2(-1.0, -3.5)], stroke);
        ui.painter().line_segment([cb.center_bottom() + vec2(-1.0, -3.5), cb.right_top() + vec2(-3.0, 3.5)], stroke);
    }
    let x = r.left() + 56.0;
    ui.painter().text(pos2(x, r.center().y), Align2::LEFT_CENTER, format!("{}", hit.line + 1), FontId::monospace(12.0), p.muted);
    // The match, then (when replacing) an arrow and what it becomes; each side gets at least
    // half the room so long matches do not hide the replacement.
    let left = x + 44.0;
    let room = (r.right() - left - 12.0).max(80.0);
    let line = |text: &str, color: Color32, background: Color32, width: f32| {
        let mut job = egui::text::LayoutJob::single_section(one_line(text), egui::TextFormat { font_id: FontId::monospace(12.5), color, background, ..Default::default() });
        job.wrap.max_width = width.max(40.0);
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        job.wrap.overflow_character = Some('…');
        ui.painter().layout_job(job)
    };
    let arrow = 36.0;
    let before_width = if replacing { (room - arrow) / 2.0 } else { room };
    let before = line(&hit.before, p.text, theme::scheme(p.dark).search_result, before_width);
    let after = replacing.then(|| line(&hit.after, p.text, Color32::TRANSPARENT, room - arrow - before.size().x));
    let end = left + before.size().x;
    ui.painter().galley(pos2(left, r.center().y - before.size().y / 2.0), before, p.text);
    if let Some(after) = after {
        ui.painter().text(pos2(end + arrow / 2.0, r.center().y), Align2::CENTER_CENTER, "→", FontId::proportional(12.5), p.muted);
        ui.painter().galley(pos2(end + arrow, r.center().y - after.size().y / 2.0), after, p.text);
    }
    let accessible = format!("{}:{} {}", hit.rel, hit.line + 1, one_line(&hit.before));
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, on, &accessible));
    if !resp.clicked() {
        return None;
    }
    let on_box = ui.input(|i| i.pointer.interact_pos()).is_some_and(|pt| pt.x < cb.right() + 6.0);
    Some(if on_box && enabled { HitClick::Toggle } else { HitClick::Open })
}

/// `text` on one line, its line breaks and indentation folded into single spaces.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;

    #[test]
    fn headless_search_lists_matches_and_replaces_the_selected_ones() {
        let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let file = workspace.path().join("A.cs");
        std::fs::write(&file, "class A {\n    void M() {\n        Console.WriteLine(value);\n        Console.WriteLine(other); // keep\n    }\n}\n").unwrap();
        let mut app = crate::screens::test_app(home.path(), true);
        app.open_workspace(workspace.path());
        app.structural.query = "Console.WriteLine($value$)".into();
        app.structural.replacement = "Log($value$)".into();
        let snapshot = std::sync::Arc::new(rc_search::IndexSnapshot::from_rel_paths(&app.workspace.as_ref().unwrap().ws.root, ["A.cs"]));
        app.structural.search(snapshot, HashMap::new());
        app.structural.shown = true;
        app.dock.show(ToolWindow::Find);
        let mut harness = crate::screens::harness(app, egui::vec2(1100.0, 720.0));
        for _ in 0..200 {
            harness.run_steps(2);
            if !harness.state().structural.busy {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(harness.state().structural.hits.len(), 2);
        // Untick the second match, then replace the first.
        harness.state_mut().structural.hits[1].selected = false;
        harness.run_steps(2);
        harness.get_by_label("Replace 1 Selected").click();
        for _ in 0..200 {
            harness.run_steps(2);
            if !harness.state().structural.applying && !harness.state().structural.busy {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(harness.state().structural.error.is_none(), "{:?}", harness.state().structural.error);
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("Log(value);") && text.contains("Console.WriteLine(other); // keep"), "{text}");
        // The search ran again: only the unticked match is left.
        assert_eq!(harness.state().structural.hits.len(), 1);
    }
}
