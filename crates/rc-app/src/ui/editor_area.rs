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
}

pub fn show(app: &mut DotCraftApp, ui: &mut Ui, editor_actions: Vec<Action>) {
    let p = rc_kit::pal(ui);
    if app.docs.list.is_empty() {
        empty_state(app, ui);
        return;
    }
    let full = ui.max_rect();
    let tabs_rect = Rect::from_min_size(full.min, vec2(full.width(), TABS_H));
    let mut commands = Vec::new();
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(tabs_rect.shrink2(vec2(6.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.set_clip_rect(tabs_rect.intersect(ui.clip_rect()));
    egui::ScrollArea::horizontal().id_salt("editor_tabs").scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(&mut child, |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.horizontal_centered(|ui| {
            for i in 0..app.docs.list.len() {
                let doc = &app.docs.list[i];
                let selected = app.docs.active == Some(i);
                let mut title = doc.name();
                if doc.is_modified() {
                    title.push_str(" *");
                }
                let (resp, close) = theme::tab(ui, selected, Some(icons::for_file(&doc.name())), &title, true);
                if selected && app.focus_editor {
                    resp.scroll_to_me(None);
                }
                let resp = resp.on_hover_text(doc.path.display().to_string());
                if close || resp.middle_clicked() {
                    commands.push(TabCommand::Close(i));
                } else if resp.clicked() {
                    app.docs.activate(i);
                    app.focus_editor = true;
                    app.session_dirty = true;
                }
                resp.context_menu(|ui| {
                    ui.set_min_width(220.0);
                    let shortcut = app.keymap.shortcut_text(Action::CloseTab).unwrap_or_default();
                    if ui.add(egui::Button::new(tr("Close")).shortcut_text(shortcut)).clicked() {
                        commands.push(TabCommand::Close(i));
                        ui.close();
                    }
                    if ui.button(tr("Close Other Tabs")).clicked() {
                        commands.push(TabCommand::CloseOthers(i));
                        ui.close();
                    }
                    if ui.button(tr("Close Tabs to the Right")).clicked() {
                        commands.push(TabCommand::CloseRight(i));
                        ui.close();
                    }
                    if ui.button(tr("Close All Tabs")).clicked() {
                        commands.push(TabCommand::CloseAll);
                        ui.close();
                    }
                    ui.separator();
                    let pin = if app.docs.list[i].pinned { tr("Unpin Tab") } else { tr("Pin Tab") };
                    if ui.button(pin).clicked() {
                        commands.push(TabCommand::Pin(i));
                        ui.close();
                    }
                    ui.separator();
                    if ui.button(tr("Copy Path")).clicked() {
                        commands.push(TabCommand::CopyPath(i));
                        ui.close();
                    }
                    if ui.button(tr("Select in Explorer")).clicked() {
                        commands.push(TabCommand::Locate(i));
                        ui.close();
                    }
                    let reveal = if cfg!(target_os = "macos") { tr("Reveal in Finder") } else { tr("Open in Explorer") };
                    if ui.button(reveal).clicked() {
                        commands.push(TabCommand::Reveal(i));
                        ui.close();
                    }
                });
            }
        });
    });
    // Separator under the tabs.
    ui.painter().hline(full.x_range(), tabs_rect.bottom() - 0.5, egui::Stroke::new(1.0, p.island_border));

    run_tab_commands(app, ui, commands);
    let Some(active) = app.docs.active else { return };
    let editor_rect = Rect::from_min_max(pos2(full.left(), tabs_rect.bottom()), full.max);
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
    let bar_h = crate::ui::search_popup::find_bar(app, ui, editor_rect);
    editor_rect.min.y += bar_h;
    let mut ed_ui = ui.new_child(egui::UiBuilder::new().max_rect(editor_rect));
    ed_ui.set_clip_rect(editor_rect.intersect(ui.clip_rect()));
    let focus = std::mem::take(&mut app.focus_editor);
    let doc = &mut app.docs.list[active];
    let mut from_menu = Vec::new();
    for a in &editor_actions {
        from_menu.extend(rc_editor::run_action(&ed_ui, &mut doc.editor, &app.editor_config, *a));
    }
    let out = EditorView::new(&mut doc.editor, &app.editor_config, &app.keymap).focus(focus).show(&mut ed_ui);
    app.insight.on_editor_events(doc, &out.events);
    if let Some(data) = out.accepted_completion {
        app.insight.completion_accepted(doc, data);
    }
    let mut run_test_line = None;
    let caret_screen = out.caret_rect.map_or(editor_rect.center_top(), |r| r.left_bottom());
    app.caret_screen = Some(caret_screen);
    for ev in out.events {
        match ev {
            EditorEvent::GutterClicked { line } => {
                doc.editor.toggle_breakpoint(line);
                app.session_dirty = true;
            }
            EditorEvent::CaretMoved => app.session_dirty = true,
            EditorEvent::GutterIconClicked { line, icon: rc_editor::GutterIcon::RunTest | rc_editor::GutterIcon::TestPassed | rc_editor::GutterIcon::TestFailed } => run_test_line = Some(line),
            _ => {}
        }
    }
    let mut insight_actions = Vec::new();
    for a in out.actions {
        match a {
            Action::ToggleBreakpoint => {
                let line = doc.editor.caret_line_col().0;
                doc.editor.toggle_breakpoint(line);
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
            other => insight_actions.push(other),
        }
    }
    insight_actions.extend(from_menu);
    if let Some(line) = run_test_line {
        app.run_test_at(line);
    }
    for a in insight_actions {
        app.run_insight_action(a, caret_screen);
    }
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
            TabCommand::CopyPath(i) => ui.ctx().copy_text(app.docs.list[i].path.display().to_string()),
            TabCommand::Reveal(i) => crate::app::reveal_in_file_manager(&app.docs.list[i].path),
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
