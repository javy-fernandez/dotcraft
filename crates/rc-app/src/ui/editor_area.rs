//! The editor island: the tab strip and the active editor, or the empty state with the most
//! useful shortcuts when no file is open.

use crate::app::DotCraftApp;
use egui::{Align2, FontId, Rect, Sense, Ui, pos2, vec2};
use rc_editor::{Action, EditorEvent, EditorView};
use rc_kit::i18n::tr;
use rc_kit::{ico, icons, theme};

const TABS_H: f32 = 38.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum TabCommand {
    Close(usize),
    CloseOthers(usize),
    CloseAll,
    CloseRight(usize),
    Pin(usize),
    CopyPath(usize),
    Reveal(usize),
    Locate(usize),
    Blame(usize),
    Diff(usize),
    Split(usize, bool),
    Float(usize),
}

pub fn show(app: &mut DotCraftApp, ui: &mut Ui, editor_actions: Vec<Action>) {
    let p = rc_kit::pal(ui);
    if app.vcs.diff.is_some() {
        crate::ui::git::diff_view(app, ui);
        return;
    }
    if app.docs.list.is_empty() {
        empty_state(app, ui);
        return;
    }
    let full = ui.max_rect();
    let mut commands = Vec::new();
    let tabs_bottom = tab_strip(app, ui, full, &mut commands);
    let tabs_rect = Rect::from_min_max(full.min, pos2(full.right(), tabs_bottom));
    // Separator under the tabs.
    ui.painter().hline(full.x_range(), tabs_rect.bottom() - 0.5, egui::Stroke::new(1.0, p.island_border));

    run_tab_commands(app, ui, commands);
    if app.docs.active.is_none() {
        return;
    }
    let body = Rect::from_min_max(pos2(full.left(), tabs_rect.bottom()), full.max);
    let focus = std::mem::take(&mut app.focus_editor);
    let Some(active) = main_doc(app) else { return };
    // Split: two panes, the focused one is the active document.
    let split = app.split.clone().filter(|s| s.main != s.other && app.docs.list.iter().any(|d| d.editor.id == s.other) && !app.floating.contains(&s.other));
    let Some(mut split) = split else {
        app.split = None;
        editor_pane(app, ui, active, body, editor_actions, focus, true);
        return;
    };
    let id_of = |app: &DotCraftApp, i: usize| app.docs.list[i].editor.id;
    let index_of = |app: &DotCraftApp, id: u64| app.docs.list.iter().position(|d| d.editor.id == id);
    // A tab chosen in the strip goes to the focused pane.
    let active_id = id_of(app, active);
    if active_id != split.main && active_id != split.other {
        if split.focus_second {
            split.other = active_id;
        } else {
            split.main = active_id;
        }
    }
    split.focus_second = active_id == split.other;
    let (Some(a), Some(b)) = (index_of(app, split.main), index_of(app, split.other)) else {
        app.split = None;
        return;
    };
    let p = rc_kit::pal(ui);
    let (ra, rb) = if split.vertical {
        let x = (body.left() + body.width() * split.ratio).round();
        (Rect::from_min_max(body.min, pos2(x - 1.0, body.bottom())), Rect::from_min_max(pos2(x + 1.0, body.top()), body.max))
    } else {
        let y = (body.top() + body.height() * split.ratio).round();
        (Rect::from_min_max(body.min, pos2(body.right(), y - 1.0)), Rect::from_min_max(pos2(body.left(), y + 1.0), body.max))
    };
    // The divider drags to resize.
    let divider = if split.vertical { Rect::from_min_max(pos2(ra.right() - 2.0, body.top()), pos2(rb.left() + 2.0, body.bottom())) } else { Rect::from_min_max(pos2(body.left(), ra.bottom() - 2.0), pos2(body.right(), rb.top() + 2.0)) };
    let dresp = ui.interact(divider, egui::Id::new("split_divider"), Sense::drag());
    let line = if dresp.hovered() || dresp.dragged() { p.accent } else { p.island_border };
    if split.vertical {
        ui.painter().vline(divider.center().x, body.y_range(), egui::Stroke::new(1.0, line));
        if dresp.hovered() || dresp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
    } else {
        ui.painter().hline(body.x_range(), divider.center().y, egui::Stroke::new(1.0, line));
        if dresp.hovered() || dresp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }
    }
    if dresp.dragged()
        && let Some(pos) = dresp.interact_pointer_pos()
    {
        split.ratio = if split.vertical { (pos.x - body.left()) / body.width() } else { (pos.y - body.top()) / body.height() }.clamp(0.15, 0.85);
    }
    app.split = Some(split.clone());
    let focused_second = split.focus_second;
    let (actions_a, actions_b) = if focused_second { (Vec::new(), editor_actions) } else { (editor_actions, Vec::new()) };
    for (idx, rect, actions, second) in [(a, ra, actions_a, false), (b, rb, actions_b, true)] {
        let header = Rect::from_min_size(rect.min, vec2(rect.width(), 26.0));
        let focused = second == focused_second;
        ui.painter().rect_filled(header, 0.0, if focused { theme::blend(p.island_bg, p.selection.gamma_multiply(0.35)) } else { p.island_bg });
        let name = app.docs.list[idx].name();
        icons::paint(ui.painter(), Rect::from_center_size(pos2(header.left() + 16.0, header.center().y), vec2(16.0, 16.0)), icons::for_file(&name), &p, true);
        ui.painter().text(pos2(header.left() + 30.0, header.center().y), Align2::LEFT_CENTER, &name, FontId::proportional(12.5), if focused { p.text } else { p.muted });
        if second {
            let cr = Rect::from_center_size(pos2(header.right() - 16.0, header.center().y), vec2(22.0, 22.0));
            let mut c = ui.new_child(egui::UiBuilder::new().max_rect(cr));
            if theme::icon_button(&mut c, ico::CLOSE_SMALL, true, tr("Unsplit")).clicked() {
                app.split = None;
            }
        }
        let pane = Rect::from_min_max(pos2(rect.left(), header.bottom()), rect.max);
        if editor_pane(app, ui, idx, pane, actions, focus && focused, focused) && !focused {
            // A click in the other pane focuses it.
            app.docs.activate(idx);
            if let Some(sp) = &mut app.split {
                sp.focus_second = second;
            }
        }
    }
}

/// The document the main pane shows: the active one, unless it is in its own window (then the
/// last one shown here).
fn main_doc(app: &mut DotCraftApp) -> Option<usize> {
    let active = app.docs.active?;
    let id = app.docs.list[active].editor.id;
    if !app.floating.contains(&id) {
        app.last_main = Some(id);
        return Some(active);
    }
    app.last_main
        .and_then(|m| app.docs.list.iter().position(|d| d.editor.id == m && !app.floating.contains(&m)))
        .or_else(|| app.docs.list.iter().position(|d| !app.floating.contains(&d.editor.id)))
}

/// One editor: conflict banner, find bar (when focused), the editor and its events. Returns
/// whether it was clicked or got the keyboard (so a split pane becomes the active one).
pub fn editor_pane(app: &mut DotCraftApp, ui: &mut Ui, active: usize, rect: Rect, editor_actions: Vec<Action>, focus: bool, focused: bool) -> bool {
    let p = rc_kit::pal(ui);
    let editor_rect = rect;
    // Conflict banner: the file changed on disk while it had unsaved edits.
    let mut editor_rect = editor_rect;
    if app.docs.list[active].conflict {
        let banner = Rect::from_min_size(editor_rect.min, vec2(editor_rect.width(), 34.0));
        ui.painter().rect_filled(banner, 0.0, rc_kit::theme::blend(p.island_bg, p.warn.gamma_multiply(0.18)));
        let mut b = ui.new_child(egui::UiBuilder::new().max_rect(banner.shrink2(vec2(12.0, 4.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        icons::image(&mut b, ico::WARNING, 16.0);
        b.label(tr("The file was changed on disk. Keep your version or load the one on disk?"));
        if b.button(tr("Load from Disk")).clicked() {
            let path = app.docs.list[active].path.clone();
            if let Ok((text, bom, mtime)) = crate::docs::read_text(&path) {
                let d = &mut app.docs.list[active];
                d.editor.reload(&text);
                d.bom = bom;
                d.mtime = mtime;
                d.conflict = false;
            }
        }
        if b.button(tr("Keep Mine")).clicked() {
            app.docs.list[active].conflict = false;
            if let Err(e) = app.docs.save(active) {
                app.notify(e, true);
            }
        }
        editor_rect.min.y = banner.bottom();
    }
    // Markdown: the rendered page beside or instead of the editor.
    let pane = editor_rect;
    let mut preview_clicked = false;
    if let Some(view) = app.docs.list[active].md_view {
        let (ed, preview) = crate::ui::markdown::layout(view, editor_rect);
        if let Some(r) = preview {
            preview_clicked = crate::ui::markdown::preview(app, ui, active, r);
        }
        crate::ui::markdown::switcher(app, ui, active, pane);
        match ed {
            Some(r) => editor_rect = r,
            None => return preview_clicked,
        }
    }
    if focused {
        let bar_h = crate::ui::search_popup::find_bar(app, ui, editor_rect);
        editor_rect.min.y += bar_h;
    }
    let mut ed_ui = ui.new_child(egui::UiBuilder::new().max_rect(editor_rect));
    ed_ui.set_clip_rect(editor_rect.intersect(ui.clip_rect()));
    let doc = &mut app.docs.list[active];
    let mut from_menu = Vec::new();
    for a in &editor_actions {
        from_menu.extend(rc_editor::run_action(&ed_ui, &mut doc.editor, &app.editor_config, *a));
    }
    let out = EditorView::new(&mut doc.editor, &app.editor_config, &app.keymap).focus(focus).show(&mut ed_ui);
    let touched = preview_clicked || out.response.clicked() || out.response.secondary_clicked() || out.events.contains(&EditorEvent::Focused);
    editor_context_menu(app, &out.response);
    let doc = &mut app.docs.list[active];
    // While paused, hovering shows the debugger's value instead of the symbol's documentation.
    if app.debugger.stopped() {
        let insight_events: Vec<_> = out.events.iter().filter(|e| !matches!(e, EditorEvent::HoverAt { .. })).cloned().collect();
        app.insight.on_editor_events(doc, &insight_events);
        for e in &out.events {
            match e {
                EditorEvent::HoverAt { pos, screen } => {
                    let expr = crate::debugger::expression_at(doc.editor.buffer.rope(), *pos);
                    app.debugger.hover(doc.editor.id, *pos, *screen, expr);
                }
                EditorEvent::HoverEnd | EditorEvent::Changed | EditorEvent::CaretMoved => app.debugger.hover_result = None,
                _ => {}
            }
        }
    } else {
        app.insight.on_editor_events(doc, &out.events);
    }
    if let Some(data) = out.accepted_completion {
        app.insight.completion_accepted(doc, data);
    }
    let mut run_test_line = None;
    // ⌘+click: the clicked spot goes into Back/Forward history.
    let navigated_from = out.events.iter().find_map(|e| matches!(e, EditorEvent::NavigateAt(_)).then(|| (doc.path.clone(), doc.editor.caret())));
    let caret_screen = out.caret_rect.map_or(editor_rect.center_top(), |r| r.left_bottom());
    app.caret_screen = Some(caret_screen);
    for ev in out.events {
        match ev {
            EditorEvent::GutterClicked { line } => {
                doc.editor.toggle_breakpoint(line);
                if !doc.editor.breakpoints.contains_key(&line) {
                    app.debugger.conditions.remove(&(doc.path.clone(), line));
                }
                app.session_dirty = true;
            }
            EditorEvent::GutterSecondaryClicked { line, .. } => {
                let path = doc.path.clone();
                let present = doc.editor.breakpoints.get(&line).copied();
                if present.is_none() {
                    doc.editor.breakpoints.insert(line, rc_editor::BreakpointKind::Normal);
                }
                let condition = app.debugger.conditions.get(&(path.clone(), line)).cloned().unwrap_or_default();
                let enabled = present != Some(rc_editor::BreakpointKind::Disabled);
                app.dialog = Some(crate::ui::dialogs::Dialog::BreakpointProperties { path, line, condition, enabled, focus: true });
                app.session_dirty = true;
            }
            EditorEvent::CaretMoved => app.session_dirty = true,
            // Editing a preview tab keeps it.
            EditorEvent::Changed => doc.preview = false,
            EditorEvent::GutterIconClicked { line, icon: rc_editor::GutterIcon::RunTest | rc_editor::GutterIcon::TestPassed | rc_editor::GutterIcon::TestFailed } => run_test_line = Some(line),
            _ => {}
        }
    }
    if let Some((d, _, screen, text)) = &app.debugger.hover_result
        && *d == doc.editor.id
    {
        let p = rc_kit::pal(ui);
        egui::Area::new(egui::Id::new("debug_hover")).order(egui::Order::Tooltip).fixed_pos(*screen + vec2(0.0, 14.0)).show(ui.ctx(), |ui| {
            egui::Frame::new().fill(p.popup_bg).stroke(egui::Stroke::new(1.0, p.popup_border)).corner_radius(6.0).inner_margin(egui::Margin::symmetric(8, 5)).show(ui, |ui| {
                ui.set_max_width(520.0);
                ui.label(egui::RichText::new(text).monospace().size(12.5).color(p.text));
            });
        });
    }
    let mut insight_actions = Vec::new();
    for a in out.actions {
        match a {
            Action::ToggleBreakpoint => {
                let line = doc.editor.caret_line_col().0;
                doc.editor.toggle_breakpoint(line);
                if !doc.editor.breakpoints.contains_key(&line) {
                    app.debugger.conditions.remove(&(doc.path.clone(), line));
                }
                app.session_dirty = true;
            }
            Action::NextError | Action::PreviousError => {
                doc.editor.next_diagnostic(a == Action::NextError);
            }
            Action::Escape => {
                if app.find_bar.is_some() {
                    app.find_bar = None;
                    doc.editor.search_matches.clear();
                    doc.editor.current_match = None;
                }
            }
            Action::Find | Action::Replace => {
                let sel = doc.editor.selected_text();
                let bar = app.find_bar.get_or_insert_with(Default::default);
                if !sel.is_empty() && !sel.contains('\n') {
                    bar.query = sel;
                }
                if a == Action::Replace && bar.replace.is_none() {
                    bar.replace = Some(String::new());
                }
                bar.focus = true;
            }
            Action::FindNext | Action::FindPrevious => {
                let n = doc.editor.search_matches.len();
                if n > 0 {
                    let cur = doc.editor.current_match.unwrap_or(0);
                    let next = if a == Action::FindNext { (cur + 1) % n } else { (cur + n - 1) % n };
                    let (s, e) = doc.editor.search_matches[next];
                    let keep = doc.editor.search_matches.clone();
                    doc.editor.select_range(s, e);
                    doc.editor.search_matches = keep;
                    doc.editor.current_match = Some(next);
                }
            }
            // Global shortcuts typed in a floating window or a split pane.
            other if other.is_ide() => app.pending.push(other),
            other => insight_actions.push(other),
        }
    }
    insight_actions.extend(from_menu);
    if let Some(line) = run_test_line {
        app.run_test_at(line);
    }
    if let Some((path, pos)) = navigated_from {
        app.push_nav_at(path, pos);
    }
    for a in insight_actions {
        app.run_insight_action(a, caret_screen);
    }
    touched
}

/// Right-click in the editor (Rider): code actions and navigation for C#, editing, formatting.
fn editor_context_menu(app: &mut DotCraftApp, resp: &egui::Response) {
    use crate::native_menu::{MenuCommand, forward_edit};
    let csharp = app.docs.active_doc().is_some_and(crate::insight::is_csharp);
    let shortcut = |app: &DotCraftApp, a: Action| app.keymap.shortcut_text(a);
    let mut actions = Vec::new();
    let mut edit = None;
    let mut usings = false;
    resp.context_menu(|ui| {
        ui.set_min_width(260.0);
        ui.set_max_width(320.0);
        let mut entry = |ui: &mut Ui, icon: Option<rc_kit::Ico>, text: &str, action: Action| {
            if theme::menu_entry(ui, icon, text, shortcut(app, action).as_deref(), false, true).clicked() {
                actions.push(action);
                ui.close();
            }
        };
        if csharp {
            entry(ui, Some(ico::BULB), tr("Show Context Actions"), Action::ShowIntentions);
            entry(ui, None, tr("Go to Declaration"), Action::GotoDeclaration);
            entry(ui, None, tr("Go to Implementation"), Action::GotoImplementation);
            entry(ui, Some(ico::TW_FIND), tr("Find Usages"), Action::FindUsages);
            entry(ui, Some(ico::EDIT), tr("Rename…"), Action::Rename);
            ui.separator();
        }
        for (icon, text, cmd) in [(ico::CUT, tr("Cut"), MenuCommand::Cut), (ico::COPY, tr("Copy"), MenuCommand::Copy), (ico::PASTE, tr("Paste"), MenuCommand::Paste)] {
            if theme::menu_item(ui, Some(icon), text, false).clicked() {
                edit = Some(cmd);
                ui.close();
            }
        }
        ui.separator();
        entry(ui, Some(ico::REFORMAT), tr("Reformat Code"), Action::Reformat);
        entry(ui, None, tr("Comment with Line Comment"), Action::CommentLine);
        if csharp && theme::menu_item(ui, Some(ico::NAMESPACE), tr("Add Missing Usings"), false).clicked() {
            usings = true;
            ui.close();
        }
        ui.separator();
        entry(ui, Some(ico::BREAKPOINT), tr("Toggle Line Breakpoint"), Action::ToggleBreakpoint);
    });
    if let Some(cmd) = edit {
        forward_edit(&resp.ctx, cmd);
    }
    if usings && let Some(i) = app.docs.active {
        app.insight.add_missing_usings(&app.docs.list[i]);
    }
    app.pending.extend(actions);
}

/// Whether `path` belongs to a test project (its nearest `.csproj`), cached per project.
fn in_test_project(path: &std::path::Path) -> bool {
    use std::collections::HashMap;
    use std::sync::Mutex;
    static CACHE: Mutex<Option<HashMap<std::path::PathBuf, bool>>> = Mutex::new(None);
    let mut dir = path.parent();
    while let Some(d) = dir {
        let project = std::fs::read_dir(d).ok().and_then(|rd| rd.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x == "csproj")));
        if let Some(p) = project {
            let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
            return *cache.get_or_insert_with(HashMap::new).entry(p.clone()).or_insert_with(|| rc_dotnet::ProjectInfo::load(&p).is_ok_and(|i| i.is_test));
        }
        dir = d.parent();
    }
    false
}

/// Rider's file colours, by kind of file: tests green, Razor/HTML/CSS orange, configuration
/// and project files blue; C# keeps the plain tab.
fn file_color(path: &std::path::Path) -> Option<egui::Color32> {
    let name = path.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let in_tests = in_test_project(path);
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if ext == "cs" && (in_tests || name.ends_with("tests.cs")) {
        Some(egui::Color32::from_rgb(0x5F, 0xB8, 0x65))
    } else if matches!(ext.as_str(), "razor" | "cshtml" | "html" | "htm" | "css" | "scss" | "js" | "ts") {
        Some(egui::Color32::from_rgb(0xE0, 0x8E, 0x3C))
    } else if matches!(ext.as_str(), "json" | "xml" | "csproj" | "fsproj" | "sln" | "slnx" | "props" | "targets" | "config" | "yaml" | "yml" | "editorconfig") {
        Some(egui::Color32::from_rgb(0x4A, 0x8F, 0xE0))
    } else {
        None
    }
}

/// The tab strip: one scrolling row with a list of every tab (Rider's hidden tabs), or several
/// rows (Settings › Editor). Tabs drag to reorder; returns the strip's bottom.
fn tab_strip(app: &mut DotCraftApp, ui: &mut Ui, full: Rect, commands: &mut Vec<TabCommand>) -> f32 {
    let multirow = app.settings.tabs_multirow;
    let colors = app.settings.tab_colors;
    let drag_id = egui::Id::new("tab_drag");
    let list_w = if multirow { 0.0 } else { 34.0 };
    // One row: the strip's height, tabs centred. Several rows: from the top, as tall as needed.
    let area = if multirow {
        Rect::from_min_max(full.min + vec2(6.0, 5.0), pos2(full.right() - 6.0, full.bottom()))
    } else {
        Rect::from_min_max(full.min + vec2(6.0, 0.0), pos2(full.right() - 6.0 - list_w, full.top() + TABS_H))
    };
    let mut rects: Vec<Rect> = Vec::new();
    let layout = if multirow { egui::Layout::top_down(egui::Align::Min) } else { egui::Layout::left_to_right(egui::Align::Center) };
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area).layout(layout));
    child.set_clip_rect(area.intersect(ui.clip_rect()));
    let mut draw_tabs = |ui: &mut Ui, rects: &mut Vec<Rect>| {
        ui.spacing_mut().item_spacing = vec2(2.0, 4.0);
        for i in 0..app.docs.list.len() {
            let doc = &app.docs.list[i];
            if app.floating.contains(&doc.editor.id) {
                continue;
            }
            let selected = app.docs.active == Some(i);
            let mut title = doc.name();
            if doc.is_modified() {
                title.push_str(" *");
            }
            let style = theme::TabStyle { italic: doc.preview, pinned: doc.pinned, color: if colors { file_color(&doc.path) } else { None }, draggable: true };
            let (resp, close) = theme::tab_ex(ui, selected, Some(icons::for_file(&doc.name())), &title, true, style);
            rects.push(resp.rect);
            if selected && app.focus_editor {
                resp.scroll_to_me(None);
            }
            if resp.drag_started() {
                ui.ctx().data_mut(|d| d.insert_temp(drag_id, i));
            }
            let resp = resp.on_hover_text(doc.path.display().to_string());
            if close || resp.middle_clicked() {
                commands.push(TabCommand::Close(i));
            } else if resp.double_clicked() {
                // Double click keeps a preview tab open (Rider, VS Code).
                app.docs.list[i].preview = false;
            } else if resp.clicked() {
                app.docs.activate(i);
                app.focus_editor = true;
                app.session_dirty = true;
            }
            tab_menu(app, &resp, i, commands);
        }
    };
    if multirow {
        child.horizontal_wrapped(|ui| draw_tabs(ui, &mut rects));
    } else {
        // The mouse wheel scrolls the row sideways.
        if ui.rect_contains_pointer(area) {
            ui.input_mut(|i| {
                if i.smooth_scroll_delta.x == 0.0 && i.smooth_scroll_delta.y != 0.0 {
                    i.smooth_scroll_delta = vec2(i.smooth_scroll_delta.y, 0.0);
                }
            });
        }
        egui::ScrollArea::horizontal().id_salt("editor_tabs").scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(&mut child, |ui| {
            ui.horizontal_centered(|ui| draw_tabs(ui, &mut rects));
        });
    }
    let bottom = if multirow { rects.iter().map(|r| r.bottom()).fold(full.top() + TABS_H - 5.0, f32::max) + 5.0 } else { full.top() + TABS_H };
    // Dragging a tab: a marker where it would go, the move on release.
    if let Some(from) = ui.ctx().data(|d| d.get_temp::<usize>(drag_id)) {
        let pointer = ui.ctx().pointer_interact_pos();
        let target = pointer.map(|p| {
            rects
                .iter()
                .enumerate()
                .filter(|(_, r)| p.y >= r.top() - 2.0 && p.y <= r.bottom() + 2.0 || !multirow)
                .find(|(_, r)| p.x < r.center().x)
                .map(|(i, _)| i)
                .unwrap_or(rects.len())
        });
        if let Some(t) = target
            && let Some(r) = rects.get(t.min(rects.len().saturating_sub(1)))
        {
            let x = if t < rects.len() { r.left() - 1.0 } else { r.right() + 1.0 };
            ui.painter().vline(x, r.y_range(), egui::Stroke::new(2.0, rc_kit::pal(ui).accent));
        }
        if !ui.input(|i| i.pointer.any_down()) {
            ui.ctx().data_mut(|d| d.remove::<usize>(drag_id));
            if let Some(t) = target {
                let to = if t > from { t - 1 } else { t };
                // Pinned tabs stay before the others.
                let pinned = app.docs.list.iter().filter(|d| d.pinned).count();
                let to = if app.docs.list[from].pinned { to.min(pinned.saturating_sub(1)) } else { to.max(pinned) };
                app.docs.move_tab(from, to.min(app.docs.list.len() - 1));
                app.session_dirty = true;
            }
        }
    }
    // Every open tab, for the ones scrolled out of sight.
    if !multirow {
        let br = Rect::from_center_size(pos2(full.right() - 6.0 - 15.0, full.top() + TABS_H / 2.0), vec2(26.0, 26.0));
        let mut b = ui.new_child(egui::UiBuilder::new().max_rect(br));
        let resp = theme::icon_button(&mut b, ico::CHEVRON_DOWN, true, tr("Show All Tabs"));
        egui::Popup::menu(&resp).show(|ui| {
            ui.set_min_width(260.0);
            let mut pick = None;
            for (i, d) in app.docs.list.iter().enumerate() {
                if theme::menu_item(ui, Some(icons::for_file(&d.name())), &d.name(), app.docs.active == Some(i)).on_hover_text(d.path.display().to_string()).clicked() {
                    pick = Some(i);
                    ui.close();
                }
            }
            if let Some(i) = pick {
                app.docs.activate(i);
                app.focus_editor = true;
            }
        });
    }
    bottom
}

fn tab_menu(app: &mut DotCraftApp, resp: &egui::Response, i: usize, commands: &mut Vec<TabCommand>) {
    resp.context_menu(|ui| {
        ui.set_min_width(220.0);
        let shortcut = app.keymap.shortcut_text(Action::CloseTab).unwrap_or_default();
        if theme::menu_entry(ui, Some(ico::CLOSE), tr("Close"), Some(&shortcut), false, true).clicked() {
            commands.push(TabCommand::Close(i));
            ui.close();
        }
        if theme::menu_item(ui, Some(ico::CLOSE), tr("Close Other Tabs"), false).clicked() {
            commands.push(TabCommand::CloseOthers(i));
            ui.close();
        }
        if theme::menu_item(ui, Some(ico::CLOSE), tr("Close Tabs to the Right"), false).clicked() {
            commands.push(TabCommand::CloseRight(i));
            ui.close();
        }
        if theme::menu_item(ui, Some(ico::CLOSE), tr("Close All Tabs"), false).clicked() {
            commands.push(TabCommand::CloseAll);
            ui.close();
        }
        ui.separator();
        let pin = if app.docs.list[i].pinned { tr("Unpin Tab") } else { tr("Pin Tab") };
        if theme::menu_item(ui, Some(ico::PIN), pin, false).clicked() {
            commands.push(TabCommand::Pin(i));
            ui.close();
        }
        let others = app.docs.list.len() > 1;
        if theme::menu_entry(ui, Some(ico::SPLIT_RIGHT), tr("Split Right"), None, false, others).clicked() {
            commands.push(TabCommand::Split(i, true));
            ui.close();
        }
        if theme::menu_entry(ui, Some(ico::SPLIT_DOWN), tr("Split Down"), None, false, others).clicked() {
            commands.push(TabCommand::Split(i, false));
            ui.close();
        }
        if theme::menu_entry(ui, Some(ico::EXTERNAL_LINK), tr("Open in New Window"), None, false, true).clicked() {
            commands.push(TabCommand::Float(i));
            ui.close();
        }
        ui.separator();
        if theme::menu_entry(ui, None, tr("Show Tabs in Multiple Rows"), None, app.settings.tabs_multirow, true).clicked() {
            app.settings.tabs_multirow = !app.settings.tabs_multirow;
            let _ = app.settings.save();
            ui.close();
        }
        ui.separator();
        if theme::menu_item(ui, Some(ico::COPY), tr("Copy Path"), false).clicked() {
            commands.push(TabCommand::CopyPath(i));
            ui.close();
        }
        if app.vcs.repo.is_some() {
            let on = app.vcs.blame_for.as_deref() == Some(app.docs.list[i].path.as_path());
            if theme::menu_entry(ui, Some(ico::HISTORY), tr("Annotate with Git Blame"), None, on, true).clicked() {
                commands.push(TabCommand::Blame(i));
                ui.close();
            }
            if theme::menu_item(ui, Some(ico::DIFF), tr("Show Diff"), false).clicked() {
                commands.push(TabCommand::Diff(i));
                ui.close();
            }
        }
        if theme::menu_item(ui, Some(ico::LOCATE), tr("Select in Explorer"), false).clicked() {
            commands.push(TabCommand::Locate(i));
            ui.close();
        }
        let reveal = if cfg!(target_os = "macos") { tr("Reveal in Finder") } else { tr("Open in Explorer") };
        if theme::menu_item(ui, Some(ico::OPEN), reveal, false).clicked() {
            commands.push(TabCommand::Reveal(i));
            ui.close();
        }
    });
}

fn run_tab_commands(app: &mut DotCraftApp, ui: &Ui, commands: Vec<TabCommand>) {
    for c in commands {
        match c {
            TabCommand::Close(i) => app.close_tab(i),
            TabCommand::CloseOthers(i) => {
                let keep = app.docs.list[i].path.clone();
                let mut j = app.docs.list.len();
                while j > 0 {
                    j -= 1;
                    if app.docs.list[j].path != keep && !app.docs.list[j].pinned {
                        app.close_tab(j);
                    }
                }
            }
            TabCommand::CloseRight(i) => {
                while app.docs.list.len() > i + 1 {
                    let j = app.docs.list.len() - 1;
                    app.close_tab(j);
                }
            }
            TabCommand::CloseAll => {
                let mut j = app.docs.list.len();
                while j > 0 {
                    j -= 1;
                    if !app.docs.list[j].pinned {
                        app.close_tab(j);
                    }
                }
            }
            TabCommand::Pin(i) => {
                let d = &mut app.docs.list[i];
                d.pinned = !d.pinned;
                // Pinned tabs go first.
                let doc = app.docs.list.remove(i);
                let pos = if doc.pinned { app.docs.list.iter().filter(|d| d.pinned).count() } else { app.docs.list.iter().filter(|d| d.pinned).count() };
                app.docs.list.insert(pos, doc);
                app.docs.active = Some(pos);
                app.session_dirty = true;
            }
            TabCommand::Split(i, vertical) => {
                // The tab goes to the new pane; the main pane keeps the current (or another) file.
                let other = app.docs.list[i].editor.id;
                let current = app.docs.active.map(|a| app.docs.list[a].editor.id).filter(|&c| c != other);
                let main = current.or_else(|| app.docs.list.iter().map(|d| d.editor.id).find(|&id| id != other && !app.floating.contains(&id)));
                if let Some(main) = main {
                    app.split = Some(crate::app::Split { vertical, main, other, focus_second: true, ratio: 0.5 });
                    app.docs.activate(i);
                }
            }
            TabCommand::Float(i) => {
                let id = app.docs.list[i].editor.id;
                if !app.floating.contains(&id) {
                    app.floating.push(id);
                }
                if app.split.as_ref().is_some_and(|s| s.main == id || s.other == id) {
                    app.split = None;
                }
            }
            TabCommand::CopyPath(i) => ui.ctx().copy_text(app.docs.list[i].path.display().to_string()),
            TabCommand::Reveal(i) => crate::app::reveal_in_file_manager(&app.docs.list[i].path),
            TabCommand::Blame(i) => {
                let (path, text) = (app.docs.list[i].path.clone(), app.docs.list[i].editor.buffer.rope().to_string());
                app.vcs.toggle_blame(&path, text, &mut app.docs);
            }
            TabCommand::Diff(i) => {
                let path = app.docs.list[i].path.clone();
                if let Some(rel) = app.vcs.repo.as_ref().and_then(|r| r.rel_path(&path)) {
                    app.save_all_docs();
                    app.vcs.open_diff(&rel);
                }
            }
            TabCommand::Locate(i) => {
                app.docs.activate(i);
                app.locate_active();
            }
        }
    }
}

/// No open file: the most useful shortcuts, like IntelliJ's empty editor.
fn empty_state(app: &mut DotCraftApp, ui: &mut Ui) {
    let p = rc_kit::pal(ui);
    let rect = ui.max_rect();
    let lines: Vec<(&str, Option<String>)> = vec![
        (tr("Search Everywhere"), Some("⇧⇧".into())),
        (tr("Go to File"), app.keymap.shortcut_text(Action::GotoFile)),
        (tr("Recent Files"), app.keymap.shortcut_text(Action::RecentFiles)),
        (tr("Find in Files"), app.keymap.shortcut_text(Action::FindInFiles)),
        (tr("Explorer"), app.keymap.shortcut_text(Action::ToggleProjectWindow)),
    ];
    let row_h = 26.0;
    let top = rect.center().y - lines.len() as f32 * row_h / 2.0;
    for (i, (label, shortcut)) in lines.into_iter().enumerate() {
        let y = top + i as f32 * row_h;
        let label_g = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(13.0), p.text_muted);
        let sc = shortcut.unwrap_or_default();
        let sc_g = ui.painter().layout_no_wrap(sc, FontId::proportional(13.0), p.accent);
        let total = label_g.size().x + 12.0 + sc_g.size().x;
        let x = rect.center().x - total / 2.0;
        ui.painter().galley(pos2(x, y), label_g.clone(), p.text_muted);
        ui.painter().galley(pos2(x + label_g.size().x + 12.0, y), sc_g, p.accent);
    }
    ui.painter().text(pos2(rect.center().x, top + 5.0 * row_h + 16.0), Align2::CENTER_TOP, tr("Drop files here to open them"), FontId::proportional(13.0), p.disabled);
    let _ = ui.allocate_rect(rect, Sense::hover());
    // Files dropped on the window open in tabs.
    let dropped: Vec<std::path::PathBuf> = ui.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
    for path in dropped {
        if path.is_file() {
            app.open_file(&path);
        }
    }
}

/// Documents in windows of their own ("Open in New Window"): each an OS window with its
/// editor; closing it brings the tab back to the main window.
pub fn floating_windows(app: &mut DotCraftApp, ctx: &egui::Context) {
    let ids = app.floating.clone();
    for id in ids {
        let Some(idx) = app.docs.list.iter().position(|d| d.editor.id == id) else {
            app.floating.retain(|f| *f != id);
            continue;
        };
        let name = app.docs.list[idx].name();
        let modified = if app.docs.list[idx].is_modified() { " *" } else { "" };
        let builder = egui::ViewportBuilder::default().with_title(format!("{name}{modified} – DotCraft")).with_inner_size([960.0, 720.0]);
        let closed = ctx.show_viewport_immediate(egui::ViewportId::from_hash_of(("dotcraft_doc", id)), builder, |ui, _| {
            let p = rc_kit::pal(ui);
            let rect = ui.max_rect();
            ui.painter().rect_filled(rect, 0.0, p.island_bg);
            let focused = app.docs.active == Some(idx);
            if editor_pane(app, ui, idx, rect.shrink(4.0), Vec::new(), false, focused) && !focused {
                app.docs.activate(idx);
            }
            ui.input(|i| i.viewport().close_requested())
        });
        if closed {
            app.floating.retain(|f| *f != id);
        }
    }
}
