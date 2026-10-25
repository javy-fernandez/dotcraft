//! Search Everywhere (⇧⇧) and its Go to File / Type / Symbol tabs: a popup under the toolbar
//! with tabs, an input and ranked results.

use crate::app::DotCraftApp;
use crate::search::{SeTab, SearchEverywhere, action_items};
use egui::{Align2, Color32, Context, FontId, Id, Key, Rect, Sense, Ui, pos2, vec2};
use rc_kit::i18n::tr;
use rc_kit::{Ico, ico, icons, theme};
use std::path::PathBuf;

/// One row of results.
#[derive(Clone)]
enum Hit {
    File { path: PathBuf, rel: String, indices: Vec<u32>, line: Option<u32>, col: Option<u32> },
    Symbol(crate::search::SymbolHit),
    Action(&'static str, rc_editor::Action),
}

const ROW_H: f32 = 28.0;

fn collect(app: &DotCraftApp, se: &SearchEverywhere) -> Vec<(Option<&'static str>, Hit)> {
    let mut out: Vec<(Option<&'static str>, Hit)> = Vec::new();
    let text = se.text.trim();
    let want = |t: SeTab| se.tab == SeTab::All || se.tab == t;
    // Files: recent files first when nothing is typed.
    if want(SeTab::Files) {
        let limit = if se.tab == SeTab::All { 8 } else { 60 };
        let mut files: Vec<Hit> = Vec::new();
        if text.is_empty() {
            for p in app.docs.recent.iter().take(limit) {
                let rel = app.workspace.as_ref().and_then(|w| p.strip_prefix(&w.ws.root).ok()).map(|r| r.display().to_string()).unwrap_or_else(|| p.display().to_string());
                files.push(Hit::File { path: p.clone(), rel, indices: Vec::new(), line: None, col: None });
            }
        } else if let Some(index) = &app.files.index {
            let snap = index.snapshot();
            let q = rc_search::FileQuery::parse(text);
            let recent: Vec<String> = app.docs.recent.iter().filter_map(|p| snap.relative(p)).collect();
            let boost = |e: &rc_search::FileEntry| if recent.iter().any(|r| r == e.rel()) { 5_000 } else { 0 };
            let res = rc_search::search_files(&snap, &q, &boost, limit);
            for m in res.matches {
                files.push(Hit::File { path: snap.abs_path(&m.entry), rel: m.entry.rel().to_string(), indices: m.indices, line: q.line, col: q.column });
            }
        }
        for (i, f) in files.into_iter().enumerate() {
            out.push(((i == 0 && se.tab == SeTab::All).then_some(tr("Files")), f));
        }
    }
    if want(SeTab::Types) || want(SeTab::Symbols) {
        let mut first_t = true;
        let mut first_s = true;
        let limit = if se.tab == SeTab::All { 8 } else { 80 };
        let mut types = 0;
        let mut syms = 0;
        for s in &se.symbols {
            let is_type = s.is_type();
            if is_type && want(SeTab::Types) && types < limit {
                out.push(((std::mem::take(&mut first_t) && se.tab == SeTab::All).then_some(tr("Types")), Hit::Symbol(s.clone())));
                types += 1;
            } else if !is_type && want(SeTab::Symbols) && se.tab != SeTab::Types && syms < limit {
                out.push(((std::mem::take(&mut first_s) && se.tab == SeTab::All).then_some(tr("Symbols")), Hit::Symbol(s.clone())));
                syms += 1;
            }
        }
    }
    if want(SeTab::Actions) && !(se.tab == SeTab::All && text.is_empty()) {
        let items = action_items();
        let mut ranked: Vec<(i32, &'static str, rc_editor::Action)> = items.into_iter().filter_map(|(l, a)| rc_editor::popup::fuzzy_score(l, text).map(|s| (s, l, a))).collect();
        ranked.sort_by_key(|r| std::cmp::Reverse(r.0));
        for (i, (_, l, a)) in ranked.into_iter().take(if se.tab == SeTab::All { 5 } else { 40 }).enumerate() {
            out.push(((i == 0 && se.tab == SeTab::All).then_some(tr("Actions")), Hit::Action(l, a)));
        }
    }
    out
}

pub fn show(app: &mut DotCraftApp, ctx: &Context) {
    let Some(mut se) = app.search_everywhere.take() else { return };
    se.update_symbols(&app.insight);
    if !se.symbols.is_empty() || !se.text.is_empty() {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
    let hits = collect(app, &se);
    se.selected = se.selected.min(hits.len().saturating_sub(1));
    let p = rc_kit::theme::palette(ctx.global_style().visuals.dark_mode);
    let screen = ctx.content_rect();
    let width = 720.0f32.min(screen.width() - 80.0);
    let pos = pos2(screen.center().x - width / 2.0, screen.top() + 70.0);
    let mut close = false;
    let mut choose: Option<Hit> = None;
    // Keys.
    ctx.input_mut(|i| {
        if i.consume_key(egui::Modifiers::NONE, Key::ArrowDown) {
            se.selected = (se.selected + 1).min(hits.len().saturating_sub(1));
        }
        if i.consume_key(egui::Modifiers::NONE, Key::ArrowUp) {
            se.selected = se.selected.saturating_sub(1);
        }
        if i.consume_key(egui::Modifiers::NONE, Key::Tab) {
            let k = SeTab::ALL.iter().position(|t| *t == se.tab).unwrap_or(0);
            se.tab = SeTab::ALL[(k + 1) % SeTab::ALL.len()];
            se.selected = 0;
        }
        if i.consume_key(egui::Modifiers::SHIFT, Key::Tab) {
            let k = SeTab::ALL.iter().position(|t| *t == se.tab).unwrap_or(0);
            se.tab = SeTab::ALL[(k + SeTab::ALL.len() - 1) % SeTab::ALL.len()];
            se.selected = 0;
        }
        if i.consume_key(egui::Modifiers::NONE, Key::Escape) {
            close = true;
        }
        if i.consume_key(egui::Modifiers::NONE, Key::Enter) {
            choose = hits.get(se.selected).map(|h| h.1.clone());
        }
    });
    let area = egui::Area::new(Id::new("search_everywhere")).order(egui::Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
        egui::Frame::NONE
            .fill(p.popup_bg)
            .stroke(egui::Stroke::new(1.0, p.popup_border))
            .corner_radius(10)
            .shadow(egui::Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(if p.dark { 110 } else { 40 }) })
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.set_width(width);
                // Tabs.
                ui.horizontal(|ui| {
                    for t in SeTab::ALL {
                        let selected = se.tab == t;
                        let g = ui.painter().layout_no_wrap(t.title().to_string(), if selected { theme::bold(13.0) } else { FontId::proportional(13.0) }, p.text);
                        let (r, resp) = ui.allocate_exact_size(vec2(g.size().x + 20.0, 28.0), Sense::click());
                        if resp.hovered() && !selected {
                            ui.painter().rect_filled(r, 6.0, p.hover);
                        }
                        ui.painter().galley(pos2(r.center().x - g.size().x / 2.0, r.center().y - g.size().y / 2.0), g, if selected { p.text } else { p.text_muted });
                        if selected {
                            ui.painter().hline(r.x_range().shrink(6.0), r.bottom() - 1.0, egui::Stroke::new(2.0, p.accent));
                        }
                        if resp.clicked() {
                            se.tab = t;
                            se.selected = 0;
                            se.focus = true;
                        }
                    }
                });
                ui.add_space(6.0);
                // Input.
                let ir = Rect::from_min_size(ui.cursor().min, vec2(width, 34.0));
                ui.painter().rect(ir, 6.0, p.control_bg, egui::Stroke::new(1.0, p.accent), egui::StrokeKind::Inside);
                icons::paint(ui.painter(), Rect::from_center_size(pos2(ir.left() + 18.0, ir.center().y), vec2(16.0, 16.0)), ico::SEARCH, &p, true);
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(pos2(ir.left() + 34.0, ir.top() + 4.0), pos2(ir.right() - 8.0, ir.bottom() - 4.0))));
                let hint = match se.tab {
                    SeTab::Files => tr("Type a file name, path or name:line"),
                    SeTab::Types => tr("Type a class, interface or struct name"),
                    SeTab::Symbols => tr("Type a member name"),
                    SeTab::Actions => tr("Type an action name"),
                    SeTab::All => tr("Search files, types, symbols and actions"),
                };
                let te = child.add(egui::TextEdit::singleline(&mut se.text).frame(egui::Frame::NONE).hint_text(hint).font(FontId::proportional(14.0)).desired_width(f32::INFINITY));
                if std::mem::take(&mut se.focus) {
                    te.request_focus();
                }
                if te.changed() {
                    se.selected = 0;
                }
                ui.allocate_rect(ir, Sense::hover());
                ui.add_space(6.0);
                // Results.
                if hits.is_empty() {
                    let msg = if se.text.trim().is_empty() { tr("Type to search") } else { tr("Nothing found") };
                    let (r, _) = ui.allocate_exact_size(vec2(width, 40.0), Sense::hover());
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, msg, FontId::proportional(13.0), p.muted);
                    return;
                }
                egui::ScrollArea::vertical().max_height(420.0).auto_shrink([false, true]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (i, (group, hit)) in hits.iter().enumerate() {
                        if let Some(g) = group {
                            let (r, _) = ui.allocate_exact_size(vec2(width, 24.0), Sense::hover());
                            ui.painter().text(pos2(r.left() + 8.0, r.center().y + 2.0), Align2::LEFT_CENTER, *g, FontId::proportional(11.5), p.muted);
                        }
                        let (r, resp) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::click());
                        if i == se.selected {
                            ui.painter().rect_filled(r, 6.0, p.selection);
                            ui.scroll_to_rect(r, None);
                        } else if resp.hovered() {
                            ui.painter().rect_filled(r, 6.0, p.row_hover);
                        }
                        let (icon, name, secondary, shortcut, highlight): (Ico, String, String, String, Vec<usize>) = match hit {
                            Hit::File { rel, indices, .. } => {
                                let name = rel.rsplit('/').next().unwrap_or(rel).to_string();
                                let off = rel.chars().count() - name.chars().count();
                                let hl = indices.iter().filter_map(|&k| (k as usize).checked_sub(off)).collect();
                                (icons::for_file(&name), name, rel.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default(), String::new(), hl)
                            }
                            Hit::Symbol(s) => (crate::convert::symbol_icon(s.kind), s.name.clone(), s.container.clone(), s.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), Vec::new()),
                            Hit::Action(l, a) => (ico::RUN_ANYTHING, l.to_string(), String::new(), app.keymap.shortcut_text(*a).unwrap_or_default(), Vec::new()),
                        };
                        icons::paint(ui.painter(), Rect::from_center_size(pos2(r.left() + 18.0, r.center().y), vec2(16.0, 16.0)), icon, &p, true);
                        // Name with the matched chars in bold accent.
                        let mut job = egui::text::LayoutJob::default();
                        for (k, ch) in name.chars().enumerate() {
                            let (font, color) = if highlight.contains(&k) { (theme::bold(13.0), p.accent) } else { (FontId::proportional(13.0), p.text) };
                            job.append(&ch.to_string(), 0.0, egui::TextFormat { font_id: font, color, ..Default::default() });
                        }
                        let g = ui.painter().layout_job(job);
                        ui.painter().galley(pos2(r.left() + 34.0, r.center().y - g.size().y / 2.0), g.clone(), p.text);
                        if !secondary.is_empty() {
                            ui.painter().text(pos2(r.left() + 42.0 + g.size().x, r.center().y), Align2::LEFT_CENTER, &secondary, FontId::proportional(12.0), p.muted);
                        }
                        if !shortcut.is_empty() {
                            ui.painter().text(pos2(r.right() - 10.0, r.center().y), Align2::RIGHT_CENTER, &shortcut, FontId::proportional(12.0), p.muted);
                        }
                        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
                        if resp.clicked() {
                            choose = Some(hit.clone());
                        }
                    }
                });
            });
    });
    if ctx.input(|i| i.pointer.any_pressed()) && !area.response.contains_pointer() && choose.is_none() {
        close = true;
    }
    match choose {
        Some(Hit::File { path, line, col, .. }) => {
            match line {
                Some(l) => app.open_at(&path, l.saturating_sub(1) as usize, col.unwrap_or(1).saturating_sub(1) as usize),
                None => app.open_file(&path),
            }
            close = true;
        }
        Some(Hit::Symbol(s)) => {
            app.open_at(&s.path, s.line, s.col);
            close = true;
        }
        Some(Hit::Action(_, a)) => {
            app.pending.push(a);
            close = true;
        }
        None => {}
    }
    if !close {
        app.search_everywhere = Some(se);
    } else {
        app.focus_editor = true;
    }
}

/// The find/replace bar at the top of the editor (⌘F / ⌘R). Returns its height.
pub fn find_bar(app: &mut DotCraftApp, ui: &mut Ui, rect: Rect) -> f32 {
    let Some(mut bar) = app.find_bar.take() else { return 0.0 };
    let Some(doc_i) = app.docs.active else {
        app.find_bar = Some(bar);
        return 0.0;
    };
    let p = rc_kit::pal(ui);
    let rows = if bar.replace.is_some() { 2.0 } else { 1.0 };
    let h = 36.0 * rows;
    let r = Rect::from_min_size(rect.min, vec2(rect.width(), h));
    ui.painter().rect_filled(r, 0.0, p.header_bg);
    ui.painter().hline(r.x_range(), r.bottom() - 0.5, egui::Stroke::new(1.0, p.island_border));
    let mut close = false;
    let mut nav: i32 = 0;
    let mut replace_one = false;
    let mut replace_all = false;
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink2(vec2(10.0, 4.0))).layout(egui::Layout::top_down(egui::Align::Min)));
    child.horizontal(|ui| {
        icons::image(ui, ico::SEARCH, 16.0);
        let te = ui.add(egui::TextEdit::singleline(&mut bar.query).hint_text(tr("Find")).desired_width(260.0));
        if std::mem::take(&mut bar.focus) {
            te.request_focus();
        }
        if te.has_focus() || te.lost_focus() {
            if ui.input(|i| i.key_pressed(Key::Enter)) {
                nav = if ui.input(|i| i.modifiers.shift) { -1 } else { 1 };
                te.request_focus();
            }
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                close = true;
            }
        }
        for (flag, icon, tip) in [(&mut bar.case, ico::MATCH_CASE, tr("Match Case")), (&mut bar.word, ico::WHOLE_WORDS, tr("Words")), (&mut bar.regex, ico::REGEX, tr("Regex"))] {
            if theme::icon_tab(ui, icon, tip, *flag).clicked() {
                *flag = !*flag;
            }
        }
        let doc = &app.docs.list[doc_i];
        let n = doc.editor.search_matches.len();
        let label = match (&bar.error, n, doc.editor.current_match) {
            (Some(e), _, _) => e.clone(),
            (_, 0, _) if !bar.query.is_empty() => tr("0 results").to_string(),
            (_, 0, _) => String::new(),
            (_, n, Some(c)) => format!("{}/{}", c + 1, n),
            (_, n, None) => rc_kit::i18n::trf("{n} results", &[("n", &n)]),
        };
        ui.label(egui::RichText::new(label).size(12.5).color(if bar.error.is_some() || (n == 0 && !bar.query.is_empty()) { p.error } else { p.text_muted }));
        if theme::icon_button(ui, ico::UP, n > 0, tr("Previous Occurrence")).clicked() {
            nav = -1;
        }
        if theme::icon_button(ui, ico::DOWN, n > 0, tr("Next Occurrence")).clicked() {
            nav = 1;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if theme::icon_button(ui, ico::CLOSE_SMALL, true, tr("Close")).clicked() {
                close = true;
            }
        });
    });
    if let Some(rep) = &mut bar.replace {
        child.horizontal(|ui| {
            icons::image(ui, ico::REPLACE, 16.0);
            ui.add(egui::TextEdit::singleline(rep).hint_text(tr("Replace")).desired_width(260.0));
            if ui.button(tr("Replace")).clicked() {
                replace_one = true;
            }
            if ui.button(tr("Replace All")).clicked() {
                replace_all = true;
            }
        });
    }
    ui.allocate_rect(r, Sense::hover());
    // Recompute matches when the query, options or text changed.
    let doc = &mut app.docs.list[doc_i];
    let key = (bar.query.clone(), bar.case, bar.word, bar.regex, doc.editor.buffer.version(), doc.editor.id);
    if bar.computed.as_ref() != Some(&key) {
        bar.computed = Some(key);
        bar.error = None;
        doc.editor.search_matches.clear();
        doc.editor.current_match = None;
        if !bar.query.is_empty() {
            match rc_search::Query::new(bar.options()) {
                Ok(q) => {
                    let text = doc.editor.buffer.rope().to_string();
                    doc.editor.search_matches = q.find_all(&text).into_iter().map(|r| (r.start, r.end)).collect();
                    let caret = doc.editor.caret();
                    doc.editor.current_match = doc.editor.search_matches.iter().position(|&(s, _)| s >= caret).or((!doc.editor.search_matches.is_empty()).then_some(0));
                }
                Err(e) => bar.error = Some(format!("{e:?}")),
            }
        }
    }
    let n = doc.editor.search_matches.len();
    if nav != 0 && n > 0 {
        let cur = doc.editor.current_match.unwrap_or(0);
        let next = if nav > 0 { (cur + 1) % n } else { (cur + n - 1) % n };
        doc.editor.current_match = Some(if doc.editor.selection.primary().is_empty() && nav > 0 { cur } else { next });
        let (s, e) = doc.editor.search_matches[doc.editor.current_match.expect("set")];
        let line = doc.editor.buffer.line_col(s).0;
        doc.editor.unfold_line(line);
        let keep = (doc.editor.search_matches.clone(), doc.editor.current_match);
        doc.editor.select_range(s, e);
        (doc.editor.search_matches, doc.editor.current_match) = keep;
    }
    if (replace_one || replace_all) && n > 0 {
        let rep = bar.replace.clone().unwrap_or_default();
        if let Ok(q) = rc_search::Query::new(bar.options()) {
            let text = doc.editor.buffer.rope().to_string();
            let edits = rc_search::compute_edits(&text, &q, &rep);
            let chosen: Vec<rc_search::TextEdit> = if replace_all {
                edits
            } else {
                let cur = doc.editor.current_match.unwrap_or(0);
                edits.into_iter().nth(cur).into_iter().collect()
            };
            let count = chosen.len();
            doc.editor.apply_text_edits(chosen.into_iter().map(|e| rc_text::Change::new(e.range.start, e.range.end, e.text)).collect());
            if replace_all {
                app.notices.push(crate::app::Notice { text: rc_kit::i18n::trf("{n} occurrences replaced", &[("n", &count)]), error: false, at: 0.0 });
            }
        }
    }
    if close {
        let doc = &mut app.docs.list[doc_i];
        doc.editor.search_matches.clear();
        doc.editor.current_match = None;
        app.focus_editor = true;
    } else {
        app.find_bar = Some(bar);
    }
    h
}
