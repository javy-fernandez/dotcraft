//! The editor widget: paints only the visible lines (gutter, text, selections, carets,
//! decorations, popups) and turns keyboard and mouse input into edits.

use crate::keymap::{Action, Keymap};
use crate::layout::{EditorFont, LineInput, LineLayout, Virtual, VirtualKind, fold_placeholder, layout_line, mono, severity_color};
use crate::state::{BreakpointKind, EditorEvent, EditorState, GutterIcon, LineChange, Severity};
use egui::{Color32, CornerRadius, CursorIcon, Event, Id, Key, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2, pos2, vec2};
use rc_kit::{Scheme, ico, icons};
use rc_text::commands::{self, Motion};
use rc_text::{EditKind, Range, Selection};

/// Editor settings shared by every editor view.
#[derive(Clone, Debug)]
pub struct EditorConfig {
    pub font: EditorFont,
    pub auto_pair: bool,
    pub show_indent_guides: bool,
    pub right_margin: Option<usize>,
    pub show_inlays: bool,
    pub caret_blink: bool,
}

impl Default for EditorConfig {
    fn default() -> Self {
        EditorConfig { font: EditorFont::default(), auto_pair: true, show_indent_guides: true, right_margin: Some(120), show_inlays: true, caret_blink: true }
    }
}

/// What a frame of the editor produced.
pub struct EditorOutput {
    pub response: egui::Response,
    /// IDE actions triggered by shortcuts while the editor had focus.
    pub actions: Vec<Action>,
    pub events: Vec<EditorEvent>,
    /// `data` of the completion item accepted this frame (the caller's index of the item).
    pub accepted_completion: Option<usize>,
    /// Screen rect of the primary caret when it is visible (popups open below it).
    pub caret_rect: Option<Rect>,
}

/// Geometry of the current frame.
#[derive(Clone, Copy)]
pub(crate) struct Metrics {
    pub row_h: f32,
    pub char_w: f32,
    /// Y offset of the galley inside a row (centres the glyphs).
    pub text_dy: f32,
    pub gutter_w: f32,
    pub num_right: f32,
    pub icons_x: f32,
    pub vcs_x: f32,
    pub fold_x: f32,
}

const TEXT_PAD: f32 = 6.0;
const SCROLLBAR_W: f32 = 12.0;
const HOVER_DELAY: f64 = 0.55;

pub struct EditorView<'a> {
    state: &'a mut EditorState,
    config: &'a EditorConfig,
    keymap: &'a Keymap,
    id: Id,
    focus_on_show: bool,
}

impl<'a> EditorView<'a> {
    pub fn new(state: &'a mut EditorState, config: &'a EditorConfig, keymap: &'a Keymap) -> Self {
        let id = Id::new(("dotcraft_editor", state.id));
        EditorView { state, config, keymap, id, focus_on_show: false }
    }

    /// Requests keyboard focus for this editor.
    pub fn focus(mut self, focus: bool) -> Self {
        self.focus_on_show = focus;
        self
    }

    pub fn id_for(doc: u64) -> Id {
        Id::new(("dotcraft_editor", doc))
    }

    pub fn show(self, ui: &mut Ui) -> EditorOutput {
        let EditorView { state, config, keymap, id, focus_on_show } = self;
        let scheme = rc_kit::theme::scheme(ui.visuals().dark_mode);
        let pal = rc_kit::pal(ui);
        let rect = ui.available_rect_before_wrap();
        let response = ui.interact(rect, id, Sense::click_and_drag());
        ui.advance_cursor_after_rect(rect);
        let mut out = EditorOutput { response: response.clone(), actions: Vec::new(), events: Vec::new(), accepted_completion: None, caret_rect: None };
        if focus_on_show {
            response.request_focus();
        }
        let focused = response.has_focus();
        if focused {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(id, egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true })
            });
        }
        if response.gained_focus() {
            out.events.push(EditorEvent::Focused);
            state.blink_epoch = ui.input(|i| i.time);
        }

        let m = metrics(ui, state, config, rect);
        state.row_h = m.row_h;
        let text_rect = Rect::from_min_max(pos2(rect.left() + m.gutter_w, rect.top()), pos2(rect.right() - SCROLLBAR_W, rect.bottom()));

        // ---------------------------------------------------------------- input
        let before_sel = state.selection.clone();
        let before_version = state.buffer.version();
        if focused {
            handle_keyboard(ui, state, config, keymap, &mut out);
        }
        handle_mouse(ui, state, &m, rect, text_rect, &response, &mut out);
        handle_scroll_wheel(ui, state, &m, rect, &response);

        if state.buffer.version() != before_version {
            out.events.push(EditorEvent::Changed);
        }
        if state.selection != before_sel {
            out.events.push(EditorEvent::CaretMoved);
            state.blink_epoch = ui.input(|i| i.time);
            // Moving the caret away closes the hover.
            state.popups.hover = None;
        }

        // ---------------------------------------------------------------- scrolling
        let rows = display_row_count(state);
        let content_h = rows as f32 * m.row_h + (text_rect.height() - 3.0 * m.row_h).max(0.0);
        let content_w = state.max_cols as f32 * m.char_w + TEXT_PAD * 2.0 + m.char_w * 4.0;
        if state.scroll_to_caret {
            state.scroll_to_caret = false;
            scroll_caret_into_view(state, &m, text_rect);
        }
        let max_y = (content_h - text_rect.height()).max(0.0);
        let max_x = (content_w - text_rect.width()).max(0.0);
        state.scroll.y = state.scroll.y.clamp(0.0, max_y);
        state.scroll.x = state.scroll.x.clamp(0.0, max_x);

        // ---------------------------------------------------------------- paint
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::ZERO, pal.editor_bg);
        paint_text_area(ui, state, config, &scheme, &m, text_rect, focused);
        paint_gutter(ui, state, &scheme, &pal, &m, rect, text_rect);
        let (vbar, hbar) = paint_scrollbars(ui, state, &scheme, &pal, rect, text_rect, content_h, content_w, &m);
        if let Some(r) = vbar.or(hbar) {
            let _ = r;
        }

        // IME position.
        out.caret_rect = caret_screen_rect(ui, state, config, &scheme, &m, text_rect);
        if focused {
            if let Some(cursor_rect) = out.caret_rect {
                ui.output_mut(|o| {
                    o.ime = Some(egui::output::IMEOutput {
                        purpose: Default::default(),
                        rect: text_rect,
                        cursor_rect,
                        should_interrupt_composition: false,
                    })
                });
            }
            if config.caret_blink {
                let t = ui.input(|i| i.time) - state.blink_epoch;
                let next = 0.5 - (t % 0.5);
                ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(next));
            }
        }

        // ---------------------------------------------------------------- popups
        if let Some(i) = crate::popups_view::show(ui, state, &scheme, &pal, &m, text_rect, focused) {
            out.accepted_completion = Some(i);
        }

        if (response.hovered() || response.dragged()) && state.link.is_none() {
            ui.ctx().set_cursor_icon(CursorIcon::Text);
        }
        out
    }
}

pub(crate) fn metrics(ui: &Ui, state: &EditorState, config: &EditorConfig, rect: Rect) -> Metrics {
    let font = mono(config.font.size);
    let (char_w, font_h) = ui.fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
    let ppp = ui.ctx().pixels_per_point();
    let snap = |v: f32| (v * ppp).round() / ppp;
    let row_h = snap((config.font.size * 1.32 * config.font.line_spacing).max(font_h));
    let text_dy = snap((row_h - font_h) / 2.0);
    let digits = state.buffer.len_lines().max(1).to_string().len().max(2) as f32;
    let num_w = digits * char_w;
    // Blame annotations take a column left of the line numbers.
    let ann_w = state.annotations.as_ref().map_or(0.0, |a| {
        let longest = a.iter().map(|s| s.chars().count()).max().unwrap_or(0).min(40) as f32;
        (longest * char_w * 0.92 + 16.0).round()
    });
    let left = rect.left() + ann_w;
    let num_right = snap(left + 10.0 + num_w);
    let icons_x = num_right + 8.0;
    let vcs_x = icons_x + 18.0;
    let fold_x = vcs_x + 5.0;
    let gutter_w = snap(fold_x + 14.0 - rect.left() + 2.0);
    Metrics { row_h, char_w, text_dy, gutter_w, num_right, icons_x, vcs_x, fold_x }
}

fn display_row_count(state: &mut EditorState) -> usize {
    let n = state.buffer.len_lines();
    state.display_rows().map_or(n, |r| r.len())
}

/// Display row of buffer line `line` (the fold's first line when it is hidden).
pub(crate) fn row_of_line(state: &mut EditorState, line: usize) -> usize {
    match state.display_rows() {
        None => line,
        Some(rows) => match rows.binary_search(&line) {
            Ok(r) => r,
            Err(r) => r.saturating_sub(1),
        },
    }
}

pub(crate) fn line_of_row(state: &mut EditorState, row: usize) -> usize {
    let n = state.buffer.len_lines();
    match state.display_rows() {
        None => row.min(n.saturating_sub(1)),
        Some(rows) => rows.get(row).copied().unwrap_or_else(|| *rows.last().unwrap_or(&0)),
    }
}

fn scroll_caret_into_view(state: &mut EditorState, m: &Metrics, text_rect: Rect) {
    let (line, _) = state.caret_line_col();
    let row = row_of_line(state, line);
    let y = row as f32 * m.row_h;
    let h = text_rect.height();
    let margin = (m.row_h * 3.0).min(h / 3.0);
    if y < state.scroll.y + margin || y + m.row_h > state.scroll.y + h - margin {
        let far = y + m.row_h < state.scroll.y - h || y > state.scroll.y + 2.0 * h;
        state.scroll.y = if far {
            // A jump: centre the caret like IntelliJ's "scroll to center".
            (y - h / 2.0 + m.row_h / 2.0).max(0.0)
        } else if y < state.scroll.y + margin {
            (y - margin).max(0.0)
        } else {
            y + m.row_h + margin - h
        };
    }
    // Horizontal: tabs and wide chars make this approximate; galleys refine it when painted.
    let col = commands::visual_col(state.buffer.rope(), state.caret(), state.indent.tab_width);
    let x = col as f32 * m.char_w + TEXT_PAD;
    let w = text_rect.width();
    let hmargin = m.char_w * 4.0;
    if x < state.scroll.x + hmargin {
        state.scroll.x = (x - hmargin * 2.0).max(0.0);
    } else if x > state.scroll.x + w - hmargin {
        state.scroll.x = x - w + hmargin * 2.0;
    }
}

// =====================================================================================
// Painting
// =====================================================================================

/// Lays out buffer line `line` with its decorations.
pub(crate) fn layout_buffer_line(ui: &Ui, state: &mut EditorState, config: &EditorConfig, scheme: &Scheme, line: usize, spans: &[rc_syntax::Span]) -> LineLayout {
    let rope = state.buffer.rope();
    let text = state.buffer.line_text(line);
    let line_start = rope.line_to_char(line);
    let line_end = line_start + text.chars().count();
    let mut virtuals: Vec<Virtual> = Vec::new();
    if config.show_inlays {
        for h in state.inlays.iter().filter(|h| h.pos >= line_start && h.pos <= line_end) {
            let mut label = h.label.clone();
            if h.pad_left {
                label.insert(0, ' ');
            }
            if h.pad_right {
                label.push(' ');
            }
            virtuals.push(Virtual { col: h.pos - line_start, text: label.trim().to_string(), kind: VirtualKind::InlayHint });
        }
    }
    let mut suffix: Option<(String, Vec<rc_syntax::Span>)> = None;
    if let Some(f) = state.collapsed_at(line) {
        virtuals.push(Virtual { col: f.start_col.min(text.chars().count()), text: fold_placeholder(f.kind, &text, f.start_col), kind: VirtualKind::FoldPlaceholder });
        let end_text = state.buffer.line_text(f.end_line.min(state.buffer.len_lines() - 1));
        let rest: String = end_text.chars().skip(f.end_col).collect();
        if f.kind == rc_syntax::FoldKind::Block && !rest.trim().is_empty() {
            suffix = Some((rest, Vec::new()));
        }
    }
    virtuals.sort_by_key(|v| (v.col, v.kind == VirtualKind::FoldPlaceholder));
    let dimmed: Vec<(usize, usize)> = state
        .diagnostics
        .iter()
        .filter(|d| d.unnecessary && d.start < line_end.max(line_start + 1) && d.end > line_start)
        .map(|d| (d.start.saturating_sub(line_start), (d.end - line_start).min(text.chars().count())))
        .collect();
    let input = LineInput {
        text: &text,
        spans,
        dimmed: &dimmed,
        virtuals: &virtuals,
        suffix: suffix.as_ref().map(|(s, sp)| (s.as_str(), sp.as_slice())),
        tab_width: state.indent.tab_width,
    };
    layout_line(&input, scheme, &config.font, |job| ui.painter().layout_job(job))
}

fn paint_text_area(ui: &Ui, state: &mut EditorState, config: &EditorConfig, scheme: &Scheme, m: &Metrics, text_rect: Rect, focused: bool) {
    let pal = rc_kit::pal(ui);
    let painter = ui.painter_at(text_rect);
    let total_rows = display_row_count(state);
    let first_row = (state.scroll.y / m.row_h).floor() as usize;
    let visible = (text_rect.height() / m.row_h).ceil() as usize + 1;
    let last_row = (first_row + visible).min(total_rows);
    if first_row >= last_row {
        return;
    }
    let x0 = text_rect.left() + TEXT_PAD - state.scroll.x;
    let scroll_y = state.scroll.y;
    let y_of_row = |row: usize| text_rect.top() + row as f32 * m.row_h - scroll_y;

    // Buffer lines of the visible rows.
    let lines: Vec<usize> = (first_row..last_row).map(|r| line_of_row(state, r)).collect();
    let first_line = lines[0];
    let last_line = *lines.last().expect("non-empty");
    let span_lines: Vec<Vec<rc_syntax::Span>> = state.line_spans(first_line, last_line - first_line + 1).to_vec();
    let layouts: Vec<LineLayout> = lines
        .iter()
        .map(|&l| {
            let spans = span_lines.get(l - first_line).map(Vec::as_slice).unwrap_or(&[]);
            layout_buffer_line(ui, state, config, scheme, l, spans)
        })
        .collect();

    let rope = state.buffer.rope().clone();
    let caret_line = state.caret_line_col().0;
    let has_selection = state.selection.ranges().iter().any(|r| !r.is_empty());
    let line_start = |l: usize| rope.line_to_char(l);

    // Caret row (only without a selection, like IntelliJ).
    for (i, &l) in lines.iter().enumerate() {
        let y = y_of_row(first_row + i);
        let row_rect = Rect::from_min_size(pos2(text_rect.left(), y), vec2(text_rect.width(), m.row_h));
        if let Some(bg) = state.line_backgrounds.get(&l) {
            painter.rect_filled(row_rect, 0.0, *bg);
        } else if Some(l) == state.execution_line {
            painter.rect_filled(row_rect, 0.0, scheme.execution_line);
        } else if state.breakpoints.contains_key(&l) && state.breakpoints[&l] != BreakpointKind::Disabled {
            painter.rect_filled(row_rect, 0.0, scheme.breakpoint_line);
        } else if l == caret_line && !has_selection && state.selection.len() == 1 {
            painter.rect_filled(row_rect, 0.0, pal.caret_row);
        }
    }

    let col_x = |lay: &LineLayout, col: usize| x0 + lay.x_of_col(col);
    let range_rects = |a: usize, b: usize, color: Color32, round: f32| {
        for (i, &l) in lines.iter().enumerate() {
            let ls = line_start(l);
            let lay = &layouts[i];
            let le = ls + lay.len;
            if b < ls || a > le || (a == b) {
                continue;
            }
            let ca = a.max(ls) - ls;
            let cb = b.min(le) - ls;
            let y = y_of_row(first_row + i);
            let x_a = col_x(lay, ca);
            let mut x_b = col_x(lay, cb);
            if b > le {
                x_b += m.char_w * 0.6; // the line break is selected too
            }
            if x_b > x_a {
                painter.rect_filled(Rect::from_min_max(pos2(x_a, y), pos2(x_b, y + m.row_h)), round, color);
            }
        }
    };

    // Search results, highlights, matched braces.
    let vis_start = line_start(first_line);
    let vis_end = if last_line + 1 < rope.len_lines() { line_start(last_line + 1) } else { rope.len_chars() };
    let from = state.search_matches.partition_point(|&(_, e)| e < vis_start);
    for (k, &(a, b)) in state.search_matches[from..].iter().enumerate().take_while(|(_, (a, _))| *a <= vis_end) {
        let current = state.current_match == Some(from + k);
        range_rects(a, b, if current { scheme.search_result_current } else { scheme.search_result }, 2.0);
    }
    for &(a, b, _write) in &state.highlights {
        if b >= vis_start && a <= vis_end {
            range_rects(a, b, scheme.identifier_under_caret, 2.0);
        }
    }
    if let Some(tree) = state.syntax.tree()
        && !has_selection
        && let Some((p, q)) = rc_syntax::matching_bracket(tree, &rope, state.caret())
    {
        range_rects(p, p + 1, scheme.matched_brace, 2.0);
        range_rects(q, q + 1, scheme.matched_brace, 2.0);
    }

    // Selections.
    for r in state.selection.ranges() {
        if !r.is_empty() {
            range_rects(r.start(), r.end(), pal.editor_selection, 0.0);
        }
    }

    // Indent guides: one at the indent of every enclosing block (IntelliJ draws them from
    // the line of the `{` down to the line before the `}`).
    if config.show_indent_guides {
        let unit = state.indent.unit.chars().count().max(1);
        let tab = state.indent.tab_width;
        let indent_of = |line: usize| -> Option<usize> {
            let t = rope.line(line);
            let mut col = 0;
            for ch in t.chars() {
                match ch {
                    ' ' => col += 1,
                    '\t' => col = (col / tab + 1) * tab,
                    '\n' => return None,
                    _ => return Some(col),
                }
            }
            None
        };
        for (i, &l) in lines.iter().enumerate() {
            // Blank lines continue the deeper of their neighbours' guides.
            let indent = indent_of(l).unwrap_or_else(|| {
                let up = (0..l).rev().take(100).find_map(indent_of).unwrap_or(0);
                let down = (l + 1..rope.len_lines()).take(100).find_map(indent_of).unwrap_or(0);
                up.max(down)
            });
            let y = y_of_row(first_row + i);
            let mut col = 0;
            while col + unit <= indent {
                // Guides start under the first char of the block line, slightly inset.
                let x = (x0 + col as f32 * m.char_w).round() + 0.5;
                if x > text_rect.left() {
                    painter.vline(x, y..=y + m.row_h, Stroke::new(1.0, scheme.indent_guide));
                }
                col += unit;
            }
        }
    }

    // Right margin.
    if let Some(col) = config.right_margin {
        let x = (x0 + col as f32 * m.char_w).round() + 0.5;
        if x > text_rect.left() {
            painter.vline(x, text_rect.y_range(), Stroke::new(1.0, scheme.right_margin));
        }
    }

    // Text and virtual-text boxes.
    for (i, lay) in layouts.iter().enumerate() {
        let y = y_of_row(first_row + i);
        let origin = pos2(x0, y + m.text_dy);
        for (r, kind) in &lay.virtuals {
            let xa = lay.galley.pos_from_cursor(egui::text::CCursor::new(r.start)).min.x;
            let xb = lay.galley.pos_from_cursor(egui::text::CCursor::new(r.end)).min.x;
            let bx = Rect::from_min_max(pos2(x0 + xa, y + 2.0), pos2(x0 + xb, y + m.row_h - 2.0));
            match kind {
                VirtualKind::FoldPlaceholder => {
                    painter.rect(bx.shrink2(vec2(0.0, 0.5)), 3.0, scheme.folded_bg, Stroke::new(1.0, scheme.folded_border), egui::StrokeKind::Inside);
                }
                VirtualKind::InlayHint => {
                    painter.rect_filled(bx.shrink2(vec2(1.0, 1.0)), 3.0, scheme.hint_bg);
                }
            }
        }
        // Hints carry their own background in the galley; strip it by painting with the
        // galley's colours (backgrounds are drawn above as rounded boxes).
        painter.galley(origin, lay.galley.clone(), scheme.text);
    }

    // ⌘/Ctrl+hover link: underlined like Rider's.
    if let Some((a, b)) = state.link {
        for (i, &l) in lines.iter().enumerate() {
            let (ls, lay) = (line_start(l), &layouts[i]);
            if a >= ls && b <= ls + lay.len {
                let y = y_of_row(first_row + i) + m.row_h - 3.0;
                painter.hline(col_x(lay, a - ls)..=col_x(lay, b - ls), y, Stroke::new(1.0, scheme.hyperlink));
            }
        }
    }

    // Diagnostics: wavy underlines.
    for d in &state.diagnostics {
        if d.unnecessary || d.severity == Severity::Hint || d.end < vis_start || d.start > vis_end {
            continue;
        }
        let color = severity_color(d.severity, scheme);
        for (i, &l) in lines.iter().enumerate() {
            let ls = line_start(l);
            let lay = &layouts[i];
            let le = ls + lay.len;
            if d.end < ls || d.start > le {
                continue;
            }
            let (mut a, mut b) = (d.start.max(ls) - ls, d.end.min(le) - ls);
            if a == b {
                // Zero-width diagnostics underline the char before (or after) the spot.
                if b < lay.len { b += 1 } else { a = a.saturating_sub(1) }
            }
            let (xa, xb) = (col_x(lay, a), col_x(lay, b).max(col_x(lay, a) + m.char_w));
            let y = y_of_row(first_row + i) + m.row_h - 3.0;
            if d.severity == Severity::Info {
                painter.hline(xa..=xb, y + 1.0, Stroke::new(1.0, color));
            } else {
                painter.add(wavy(xa, xb, y, color));
            }
        }
    }

    // Carets.
    let t = ui.input(|i| i.time) - state.blink_epoch;
    let visible_phase = !config.caret_blink || (t % 1.0) < 0.5;
    if focused && visible_phase {
        for r in state.selection.ranges() {
            let (cl, cc) = state.buffer.line_col(r.head);
            if let Some(i) = lines.iter().position(|&l| l == cl) {
                let lay = &layouts[i];
                let x = (col_x(lay, cc.min(lay.len)) * ui.ctx().pixels_per_point()).round() / ui.ctx().pixels_per_point();
                let y = y_of_row(first_row + i);
                painter.rect_filled(Rect::from_min_size(pos2(x, y + 1.0), vec2(2.0, m.row_h - 2.0)), 0.0, scheme.caret);
            }
        }
    }
    // IME preedit text drawn at the caret.
    if let Some(pre) = &state.ime_preedit
        && !pre.is_empty()
    {
        let (cl, cc) = state.caret_line_col();
        if let Some(i) = lines.iter().position(|&l| l == cl) {
            let lay = &layouts[i];
            let x = col_x(lay, cc);
            let y = y_of_row(first_row + i);
            let g = painter.layout_no_wrap(pre.clone(), mono(config.font.size), scheme.text);
            let r = Rect::from_min_size(pos2(x, y), vec2(g.size().x, m.row_h));
            painter.rect_filled(r, 0.0, pal.editor_bg);
            painter.galley(pos2(x, y + m.text_dy), g, scheme.text);
            painter.hline(r.x_range(), r.bottom() - 2.0, Stroke::new(1.0, scheme.text));
        }
    }
}

fn wavy(x0: f32, x1: f32, y: f32, color: Color32) -> Shape {
    let mut pts = Vec::new();
    let step = 2.0;
    let mut x = x0;
    let mut up = true;
    while x <= x1 {
        pts.push(pos2(x, if up { y } else { y + 2.0 }));
        x += step;
        up = !up;
    }
    Shape::line(pts, Stroke::new(1.0, color))
}

fn caret_screen_rect(ui: &Ui, state: &mut EditorState, config: &EditorConfig, scheme: &Scheme, m: &Metrics, text_rect: Rect) -> Option<Rect> {
    let (line, col) = state.caret_line_col();
    let row = row_of_line(state, line);
    let y = text_rect.top() + row as f32 * m.row_h - state.scroll.y;
    if y < text_rect.top() - m.row_h || y > text_rect.bottom() {
        return None;
    }
    let spans = state.line_spans(line, 1).first().cloned().unwrap_or_default();
    let lay = layout_buffer_line(ui, state, config, scheme, line, &spans);
    let x = text_rect.left() + TEXT_PAD - state.scroll.x + lay.x_of_col(col);
    Some(Rect::from_min_size(pos2(x, y), vec2(2.0, m.row_h)))
}

/// Screen position under the caret's line (where popups open).
pub(crate) fn caret_anchor(ui: &Ui, state: &mut EditorState, scheme: &Scheme, m: &Metrics, text_rect: Rect, pos: usize) -> Pos2 {
    let (line, col) = state.buffer.line_col(pos);
    let row = row_of_line(state, line);
    let y = text_rect.top() + row as f32 * m.row_h - state.scroll.y;
    let spans = state.line_spans(line, 1).first().cloned().unwrap_or_default();
    let config = EditorConfig::default();
    let lay = layout_buffer_line(ui, state, &config, scheme, line, &spans);
    pos2(text_rect.left() + TEXT_PAD - state.scroll.x + lay.x_of_col(col), y)
}

fn paint_gutter(ui: &Ui, state: &mut EditorState, scheme: &Scheme, pal: &rc_kit::Palette, m: &Metrics, rect: Rect, text_rect: Rect) {
    let gutter = Rect::from_min_max(rect.min, pos2(rect.left() + m.gutter_w, rect.bottom()));
    let painter = ui.painter_at(gutter);
    painter.rect_filled(gutter, 0.0, pal.editor_bg);
    let total_rows = display_row_count(state);
    let first_row = (state.scroll.y / m.row_h).floor() as usize;
    let visible = (text_rect.height() / m.row_h).ceil() as usize + 1;
    let last_row = (first_row + visible).min(total_rows);
    let caret_line = state.caret_line_col().0;
    let font = mono(12.0);
    let hover_pos = ui.input(|i| i.pointer.hover_pos()).filter(|p| gutter.contains(*p));
    let fold_hover = hover_pos.is_some();
    let folds: Vec<rc_syntax::FoldRange> = state.syntax.folds().to_vec();
    let fold_start_lines: std::collections::HashMap<usize, rc_syntax::FoldRange> = folds.iter().map(|f| (f.start_line, *f)).collect();
    let fold_end_lines: std::collections::HashMap<usize, usize> = folds.iter().filter(|f| !state.is_folded(f.start_line)).map(|f| (f.end_line, f.start_line)).collect();
    // The fold around the hovered or caret line is the one whose markers show (New UI).
    let hover_line = hover_pos.map(|p| {
        let row = ((p.y - text_rect.top() + state.scroll.y) / m.row_h).floor().max(0.0) as usize;
        line_of_row(state, row)
    });
    for row in first_row..last_row {
        let l = line_of_row(state, row);
        let y = text_rect.top() + row as f32 * m.row_h - state.scroll.y;
        let row_rect = Rect::from_min_max(pos2(gutter.left(), y), pos2(gutter.right(), y + m.row_h));
        let center_y = y + m.row_h / 2.0;
        if let Some(text) = state.annotations.as_ref().and_then(|a| a.get(l)) {
            if let Some(bg) = state.line_backgrounds.get(&l) {
                painter.rect_filled(Rect::from_min_max(pos2(gutter.left(), y), pos2(gutter.left() + (m.num_right - 10.0 - gutter.left()) - 6.0, y + m.row_h)), 0.0, *bg);
            }
            let g = painter.layout_no_wrap(text.clone(), egui::FontId::proportional(11.5), pal.text_muted);
            painter.galley(pos2(gutter.left() + 8.0, (center_y - g.size().y / 2.0).round()), g, pal.text_muted);
        }
        // Breakpoint icon replaces the line number.
        if let Some(kind) = state.breakpoints.get(&l) {
            let icon = match kind {
                BreakpointKind::Normal => ico::BREAKPOINT,
                BreakpointKind::Conditional => ico::BREAKPOINT,
                BreakpointKind::Disabled => ico::BREAKPOINT_DISABLED,
                BreakpointKind::Verified => ico::BREAKPOINT_VALID,
                BreakpointKind::Invalid => ico::BREAKPOINT_INVALID,
            };
            let r = Rect::from_center_size(pos2(m.num_right - 7.0, center_y), Vec2::splat(16.0));
            icons::paint(&painter, r, icon, pal, true);
            if *kind == BreakpointKind::Conditional {
                // The "?" badge sits on the top-right corner, as in IntelliJ.
                let badge = Rect::from_min_size(pos2(r.right() - 7.0, r.top() - 1.0), vec2(7.0, 10.0));
                icons::paint_sized(&painter, badge, ico::QUESTION_BADGE, pal, true, 16.0);
            }
        } else {
            let color = if l == caret_line { pal.gutter_fg_caret } else { pal.gutter_fg };
            let number = match &state.display_numbers {
                Some(map) => map.get(l).copied().flatten().map(|n| (n + 1).to_string()).unwrap_or_default(),
                None => (l + 1).to_string(),
            };
            let g = painter.layout_no_wrap(number, font.clone(), color);
            let gy = (center_y - g.size().y / 2.0).round();
            painter.galley(pos2(m.num_right - g.size().x, gy), g, color);
        }
        // Execution point arrow.
        if Some(l) == state.execution_line {
            let r = Rect::from_center_size(pos2(m.icons_x + 8.0, center_y), Vec2::splat(16.0));
            icons::paint(&painter, r, ico::EXECUTION_POINT, pal, true);
        } else if let Some(icon) = state.gutter_icons.get(&l) {
            let ic = match icon {
                GutterIcon::RunTest => ico::GUTTER_RUN,
                GutterIcon::TestPassed => ico::GUTTER_RUN_SUCCESS,
                GutterIcon::TestFailed => ico::GUTTER_RUN_FAILED,
                GutterIcon::RunApp => ico::GUTTER_RUN,
                GutterIcon::Implemented => ico::GUTTER_IMPLEMENTED,
                GutterIcon::Implementing => ico::GUTTER_IMPLEMENTING,
                GutterIcon::Overridden => ico::GUTTER_OVERRIDDEN,
                GutterIcon::Overriding => ico::GUTTER_OVERRIDING,
                GutterIcon::Bookmark => ico::GUTTER_BOOKMARK,
            };
            let r = Rect::from_center_size(pos2(m.icons_x + 8.0, center_y), Vec2::splat(16.0));
            icons::paint(&painter, r, ic, pal, true);
        }
        // VCS change bar.
        if let Some(change) = state.line_changes.get(&l) {
            let (color, r) = match change {
                LineChange::Added => (scheme.added_lines, Rect::from_min_size(pos2(m.vcs_x, y), vec2(3.0, m.row_h))),
                LineChange::Modified => (scheme.modified_lines, Rect::from_min_size(pos2(m.vcs_x, y), vec2(3.0, m.row_h))),
                LineChange::Deleted => (scheme.deleted_lines, Rect::from_min_size(pos2(m.vcs_x - 1.0, y + m.row_h - 2.0), vec2(5.0, 4.0))),
            };
            painter.rect_filled(r, 1.0, color);
        }
        // Fold markers: always for collapsed folds, on gutter hover for open ones.
        let fold_rect = Rect::from_center_size(pos2(m.fold_x + 7.0, center_y), Vec2::splat(16.0));
        if state.is_folded(l) {
            icons::paint(&painter, fold_rect, ico::GUTTER_UNFOLD, pal, true);
        } else if fold_hover && fold_start_lines.contains_key(&l) {
            let near = hover_line.is_some_and(|h| {
                let f = fold_start_lines[&l];
                h >= f.start_line && h <= f.end_line
            });
            if near || hover_line == Some(l) {
                icons::paint(&painter, fold_rect, ico::GUTTER_FOLD, pal, true);
            }
        } else if fold_hover
            && let Some(&start) = fold_end_lines.get(&l)
            && hover_line.is_some_and(|h| h >= start && h <= l)
        {
            icons::paint(&painter, fold_rect, ico::GUTTER_FOLD_BOTTOM, pal, true);
        }
        let _ = row_rect;
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_scrollbars(
    ui: &Ui,
    state: &mut EditorState,
    scheme: &Scheme,
    pal: &rc_kit::Palette,
    rect: Rect,
    text_rect: Rect,
    content_h: f32,
    content_w: f32,
    m: &Metrics,
) -> (Option<egui::Response>, Option<egui::Response>) {
    let track = Rect::from_min_max(pos2(rect.right() - SCROLLBAR_W, rect.top()), rect.max);
    let painter = ui.painter_at(rect);
    // Error stripe: marks for problems and search results along the whole document.
    let rows = display_row_count(state).max(1) as f32;
    let lines_total = state.buffer.len_lines();
    let mark_y = |state: &mut EditorState, pos: usize| {
        let line = state.buffer.line_col(pos).0;
        let row = row_of_line(state, line) as f32;
        track.top() + (row / rows.max(track.height() / m.row_h)) * track.height()
    };
    let diags: Vec<(usize, Severity)> = state.diagnostics.iter().filter(|d| d.severity <= Severity::Warning).map(|d| (d.start, d.severity)).collect();
    for (pos, sev) in diags {
        let y = mark_y(state, pos);
        painter.rect_filled(Rect::from_min_size(pos2(track.left() + 3.0, y), vec2(SCROLLBAR_W - 5.0, 2.0)), 0.0, severity_color(sev, scheme));
    }
    let matches: Vec<usize> = state.search_matches.iter().take(2000).map(|m| m.0).collect();
    for pos in matches {
        let y = mark_y(state, pos);
        painter.rect_filled(Rect::from_min_size(pos2(track.left() + 3.0, y), vec2(SCROLLBAR_W - 5.0, 2.0)), 0.0, scheme.search_result_current);
    }
    let _ = lines_total;

    let mut vresp = None;
    if content_h > text_rect.height() + 0.5 {
        let id = Id::new(("editor_vbar", state.id));
        let resp = ui.interact(track, id, Sense::click_and_drag());
        let ratio = text_rect.height() / content_h;
        let thumb_h = (track.height() * ratio).max(24.0);
        let max_scroll = content_h - text_rect.height();
        let thumb_y = track.top() + (track.height() - thumb_h) * (state.scroll.y / max_scroll);
        let thumb = Rect::from_min_size(pos2(track.right() - 8.0, thumb_y), vec2(6.0, thumb_h));
        if resp.dragged() {
            let dy = resp.drag_delta().y;
            state.scroll.y = (state.scroll.y + dy / (track.height() - thumb_h) * max_scroll).clamp(0.0, max_scroll);
        } else if resp.clicked()
            && let Some(p) = resp.interact_pointer_pos()
            && !thumb.contains(p)
        {
            let t = ((p.y - track.top() - thumb_h / 2.0) / (track.height() - thumb_h)).clamp(0.0, 1.0);
            state.scroll.y = t * max_scroll;
        }
        let active = resp.hovered() || resp.dragged();
        let alpha = if active { 0.5 } else { 0.28 };
        let thumb = if active { thumb.expand2(vec2(1.5, 0.0)).translate(vec2(-1.5, 0.0)) } else { thumb };
        painter.rect_filled(thumb, 4.0, pal.text.gamma_multiply(alpha));
        vresp = Some(resp);
    }
    let mut hresp = None;
    if content_w > text_rect.width() + 0.5 {
        let htrack = Rect::from_min_max(pos2(text_rect.left(), rect.bottom() - 10.0), pos2(text_rect.right(), rect.bottom()));
        let id = Id::new(("editor_hbar", state.id));
        let resp = ui.interact(htrack, id, Sense::drag());
        let ratio = text_rect.width() / content_w;
        let thumb_w = (htrack.width() * ratio).max(24.0);
        let max_scroll = content_w - text_rect.width();
        let thumb_x = htrack.left() + (htrack.width() - thumb_w) * (state.scroll.x / max_scroll);
        if resp.dragged() {
            state.scroll.x = (state.scroll.x + resp.drag_delta().x / (htrack.width() - thumb_w) * max_scroll).clamp(0.0, max_scroll);
        }
        let active = resp.hovered() || resp.dragged();
        let thumb = Rect::from_min_size(pos2(thumb_x, htrack.bottom() - 8.0), vec2(thumb_w, 6.0));
        painter.rect_filled(thumb, 4.0, pal.text.gamma_multiply(if active { 0.5 } else { 0.28 }));
        hresp = Some(resp);
    }
    (vresp, hresp)
}

// =====================================================================================
// Input
// =====================================================================================

fn handle_scroll_wheel(ui: &Ui, state: &mut EditorState, m: &Metrics, rect: Rect, response: &egui::Response) {
    let hovered = ui.rect_contains_pointer(rect) && response.hovered() || ui.ui_contains_pointer() && ui.input(|i| i.pointer.hover_pos()).is_some_and(|p| rect.contains(p));
    if !hovered {
        return;
    }
    let delta = ui.input(|i| i.smooth_scroll_delta);
    if delta != Vec2::ZERO {
        state.scroll -= delta;
        ui.ctx().input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
        let _ = m;
    }
}

/// Char offset under a screen position (clamped to the text).
pub(crate) fn pos_at(ui: &Ui, state: &mut EditorState, config: &EditorConfig, m: &Metrics, text_rect: Rect, p: Pos2) -> (usize, bool) {
    let scheme = rc_kit::theme::scheme(ui.visuals().dark_mode);
    let rows = display_row_count(state);
    let row = ((p.y - text_rect.top() + state.scroll.y) / m.row_h).floor();
    if row < 0.0 {
        return (0, false);
    }
    let row = row as usize;
    if row >= rows {
        return (state.buffer.len_chars(), false);
    }
    let line = line_of_row(state, row);
    let spans = state.line_spans(line, 1).first().cloned().unwrap_or_default();
    let lay = layout_buffer_line(ui, state, config, &scheme, line, &spans);
    let x = p.x - (text_rect.left() + TEXT_PAD - state.scroll.x);
    let on_placeholder = lay.placeholder_at_x(x);
    let col = lay.col_at_x(x);
    (state.buffer.rope().line_to_char(line) + col, on_placeholder)
}

fn handle_mouse(ui: &Ui, state: &mut EditorState, m: &Metrics, rect: Rect, text_rect: Rect, response: &egui::Response, out: &mut crate::view::EditorOutput) {
    let config = EditorConfig::default();
    let (pointer, now, mods) = ui.input(|i| (i.pointer.clone(), i.time, i.modifiers));
    let gutter = Rect::from_min_max(rect.min, pos2(text_rect.left(), rect.bottom()));

    // Right-click on the line numbers: breakpoint properties.
    if response.secondary_clicked()
        && let Some(p) = response.interact_pointer_pos()
        && gutter.contains(p)
        && p.x < m.icons_x - 2.0
    {
        let row = ((p.y - text_rect.top() + state.scroll.y) / m.row_h).floor().max(0.0) as usize;
        if row < display_row_count(state) {
            out.events.push(EditorEvent::GutterSecondaryClicked { line: line_of_row(state, row), screen: p });
        }
        return;
    }
    // Right-click in the text: the caret goes there unless it hits the selection (the context
    // menu then acts on that place).
    if response.secondary_clicked()
        && let Some(p) = response.interact_pointer_pos()
        && text_rect.contains(p)
    {
        let (pos, _) = pos_at(ui, state, &config, m, text_rect, p);
        if !state.selection.ranges().iter().any(|r| r.start() <= pos && pos <= r.end() && !r.is_empty()) {
            state.set_selection(Selection::point(pos));
        }
        out.events.push(EditorEvent::TextSecondaryClicked);
        return;
    }
    // Gutter clicks.
    if response.clicked()
        && let Some(p) = response.interact_pointer_pos()
        && gutter.contains(p)
    {
        let row = ((p.y - text_rect.top() + state.scroll.y) / m.row_h).floor().max(0.0) as usize;
        if row < display_row_count(state) {
            let line = line_of_row(state, row);
            if p.x >= m.fold_x - 2.0 {
                if state.is_folded(line) || state.syntax.folds().iter().any(|f| f.start_line == line) {
                    state.toggle_fold(line);
                } else if let Some(f) = state.syntax.folds().iter().find(|f| f.end_line == line).copied() {
                    state.toggle_fold(f.start_line);
                }
            } else if p.x >= m.icons_x - 2.0 && p.x < m.vcs_x {
                if let Some(icon) = state.gutter_icons.get(&line) {
                    out.events.push(EditorEvent::GutterIconClicked { line, icon: *icon });
                } else {
                    out.events.push(EditorEvent::GutterClicked { line });
                }
            } else {
                out.events.push(EditorEvent::GutterClicked { line });
            }
        }
        return;
    }
    if gutter.contains(pointer.hover_pos().unwrap_or(Pos2::ZERO)) && pointer.hover_pos().is_some() {
        ui.ctx().set_cursor_icon(CursorIcon::Default);
    }

    // Press in the text area.
    let pressed_here = pointer.primary_pressed() && pointer.interact_pos().is_some_and(|p| text_rect.contains(p)) && response.hovered();
    if pressed_here {
        response.request_focus();
        let p = pointer.interact_pos().expect("checked");
        let (pos, on_placeholder) = pos_at(ui, state, &config, m, text_rect, p);
        if on_placeholder {
            let line = state.buffer.line_col(pos).0;
            state.toggle_fold(line);
            return;
        }
        // Click counting.
        let count = match state.last_click {
            Some((t, at, n)) if now - t < 0.4 && at.abs_diff(pos) <= 1 => (n % 3) + 1,
            _ => 1,
        };
        state.last_click = Some((now, pos, count));
        state.popups.completion = None;
        let rope = state.buffer.rope().clone();
        let command = mods.command;
        let add_caret = mods.alt && mods.shift;
        if command && !mods.shift && count == 1 {
            // ⌘/Ctrl+click: navigate (the caret also moves there).
            state.set_selection(Selection::point(pos));
            out.events.push(EditorEvent::NavigateAt(pos));
            state.drag_anchor = None;
            return;
        }
        if mods.alt && !mods.shift {
            // Column (block) selection by dragging with Alt.
            let line = rope.char_to_line(pos);
            let col = commands::visual_col(&rope, pos, state.indent.tab_width);
            state.column_drag = Some((line, col));
            state.selection = Selection::point(pos);
            state.drag_anchor = None;
            return;
        }
        state.column_drag = None;
        let range = match count {
            2 => {
                let (s, e) = commands::word_at(&rope, pos);
                if s == e { Range::new(pos, (pos + 1).min(rope.len_chars())) } else { Range::new(s, e) }
            }
            3 => {
                let line = rope.char_to_line(pos);
                let s = rope.line_to_char(line);
                let e = if line + 1 < rope.len_lines() { rope.line_to_char(line + 1) } else { rope.len_chars() };
                Range::new(s, e)
            }
            _ => Range::point(pos),
        };
        if mods.shift && !add_caret {
            let anchor = state.selection.primary().anchor;
            state.selection = Selection::single(Range::new(anchor, pos));
        } else if add_caret {
            state.selection = state.selection.push(range);
        } else {
            state.selection = Selection::single(range);
        }
        state.drag_anchor = Some(range);
        state.buffer.seal_undo();
    }

    // Dragging.
    if pointer.primary_down()
        && response.is_pointer_button_down_on()
        && let Some(p) = pointer.interact_pos()
    {
        if let Some((line0, col0)) = state.column_drag {
            let (pos, _) = pos_at(ui, state, &config, m, text_rect, p);
            let rope = state.buffer.rope().clone();
            let line1 = rope.char_to_line(pos);
            let col1 = commands::visual_col(&rope, pos, state.indent.tab_width).max(((p.x - text_rect.left() - TEXT_PAD + state.scroll.x) / m.char_w).round().max(0.0) as usize);
            let (a, b) = (line0.min(line1), line0.max(line1));
            let ranges: Vec<Range> = (a..=b)
                .map(|l| {
                    let s = commands::pos_at_visual_col(&rope, l, col0, state.indent.tab_width);
                    let e = commands::pos_at_visual_col(&rope, l, col1, state.indent.tab_width);
                    Range::new(s, e)
                })
                .collect();
            let primary = if line1 >= line0 { ranges.len() - 1 } else { 0 };
            state.selection = Selection::new(ranges, primary);
        } else if let Some(anchor) = state.drag_anchor
            && response.dragged()
        {
            let (pos, _) = pos_at(ui, state, &config, m, text_rect, p);
            let rope = state.buffer.rope().clone();
            let count = state.last_click.map_or(1, |c| c.2);
            let new = if count == 2 {
                let (s, e) = commands::word_at(&rope, pos);
                if pos < anchor.start() { Range::new(anchor.end(), s) } else { Range::new(anchor.start(), e.max(pos)) }
            } else if count == 3 {
                let line = rope.char_to_line(pos);
                if pos < anchor.start() {
                    Range::new(anchor.end(), rope.line_to_char(line))
                } else {
                    let e = if line + 1 < rope.len_lines() { rope.line_to_char(line + 1) } else { rope.len_chars() };
                    Range::new(anchor.start(), e)
                }
            } else {
                Range::new(anchor.anchor, pos)
            };
            let primary = state.selection.primary();
            state.selection = state.selection.transform(|r| if r == primary { new } else { r });
            // Auto-scroll while dragging past the edges.
            if p.y < text_rect.top() {
                state.scroll.y -= m.row_h;
            } else if p.y > text_rect.bottom() {
                state.scroll.y += m.row_h;
            }
            ui.ctx().request_repaint();
        }
    }
    if pointer.primary_released() {
        state.drag_anchor = None;
        state.column_drag = None;
    }

    // ⌘/Ctrl held over an identifier: it becomes a link (⌘+click goes to its declaration).
    let link = pointer.hover_pos().filter(|p| mods.command && !pointer.any_down() && text_rect.contains(*p) && response.hovered()).and_then(|p| {
        let (pos, _) = pos_at(ui, state, &config, m, text_rect, p);
        let (ws, we) = commands::word_at(state.buffer.rope(), pos);
        let first = state.buffer.rope().get_char(ws)?;
        (ws < we && (first.is_alphabetic() || first == '_')).then_some((ws, we))
    });
    if link.is_some() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    if link != state.link {
        state.link = link;
        ui.ctx().request_repaint();
    }

    // Hover (documentation / problems) after a delay.
    if let Some(p) = pointer.hover_pos().filter(|p| text_rect.contains(*p) && response.hovered()) {
        if pointer.any_down() || pointer.is_moving() && pointer.velocity().length() > 400.0 {
            return;
        }
        let (pos, _) = pos_at(ui, state, &config, m, text_rect, p);
        let (ws, we) = commands::word_at(state.buffer.rope(), pos);
        let key = if ws < we { ws } else { pos };
        match state.hover {
            Some((k, since, fired)) if k == key => {
                if !fired && now - since > HOVER_DELAY {
                    state.hover = Some((k, since, true));
                    out.events.push(EditorEvent::HoverAt { pos, screen: p });
                } else if !fired {
                    ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(HOVER_DELAY - (now - since) + 0.01));
                }
            }
            Some((_, _, fired)) => {
                if fired && !hover_popup_hovered(ui) {
                    out.events.push(EditorEvent::HoverEnd);
                    state.popups.hover = None;
                }
                state.hover = Some((key, now, false));
                ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(HOVER_DELAY + 0.01));
            }
            None => {
                state.hover = Some((key, now, false));
                ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(HOVER_DELAY + 0.01));
            }
        }
    } else if state.hover.is_some() && !hover_popup_hovered(ui) {
        if state.hover.is_some_and(|h| h.2) {
            out.events.push(EditorEvent::HoverEnd);
            state.popups.hover = None;
        }
        state.hover = None;
    }
}

fn hover_popup_hovered(ui: &Ui) -> bool {
    ui.ctx().data(|d| d.get_temp::<bool>(Id::new("dotcraft_hover_popup_hovered"))).unwrap_or(false)
}

fn handle_keyboard(ui: &Ui, state: &mut EditorState, config: &EditorConfig, keymap: &Keymap, out: &mut EditorOutput) {
    let events = ui.input(|i| i.events.clone());
    let now = ui.input(|i| i.time);
    // "Alt Alt + ↑/↓" clones carets: track a double tap of Alt that is still held.
    let alt = ui.input(|i| i.modifiers.alt);
    let (last_tap, taps, was_down) = state.alt_taps;
    if alt && !was_down {
        let taps = if now - last_tap < 0.35 { taps + 1 } else { 1 };
        state.alt_taps = (now, taps, true);
    } else if !alt && was_down {
        state.alt_taps = (now, taps, false);
    }
    let double_alt_held = alt && state.alt_taps.1 >= 2;

    for ev in events {
        match ev {
            Event::Key { key, pressed: true, modifiers, .. } => {
                if crate::popups_view::handle_key(state, key, modifiers, out) {
                    continue;
                }
                if double_alt_held && (key == Key::ArrowUp || key == Key::ArrowDown) {
                    let up = key == Key::ArrowUp;
                    state.selection = commands::add_caret_vertical(state.buffer.rope(), &state.selection, up, state.indent.tab_width);
                    state.scroll_to_caret = true;
                    continue;
                }
                let Some(action) = keymap.lookup(key, modifiers) else { continue };
                if action.is_editing() {
                    exec(ui, state, config, action, out);
                } else if !action.is_ide() {
                    // IDE-wide shortcuts are handled by the application for every focus.
                    out.actions.push(action);
                }
            }
            Event::Text(text) => {
                if state.read_only || text.is_empty() {
                    continue;
                }
                type_text(state, config, &text, out);
            }
            Event::Paste(text) => {
                paste(state, &text);
                out.events.push(EditorEvent::Pasted);
            }
            Event::Copy => copy(ui, state, false),
            Event::Cut => copy(ui, state, true),
            Event::Ime(ime) => match ime {
                egui::ImeEvent::Preedit { text, .. } => state.ime_preedit = Some(text),
                egui::ImeEvent::Commit(text) => {
                    state.ime_preedit = None;
                    if !text.is_empty() {
                        type_text(state, config, &text, out);
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
}

fn type_text(state: &mut EditorState, config: &EditorConfig, text: &str, out: &mut EditorOutput) {
    let rope = state.buffer.rope().clone();
    let mut chars = text.chars();
    if let (Some(ch), None) = (chars.next(), chars.next()) {
        // Typing `}` on a blank line re-indents it to its block (electric brace).
        let edited = if ch == '}' && state.selection.len() == 1 && state.selection.primary().is_empty() {
            electric_close_brace(&rope, state).unwrap_or_else(|| commands::type_char(&rope, &state.selection, ch, config.auto_pair))
        } else {
            commands::type_char(&rope, &state.selection, ch, config.auto_pair)
        };
        state.apply(edited, EditKind::Insert);
        markup_assist(state, ch);
        out.events.push(EditorEvent::Typed(ch));
        // Typing filters an open completion list.
        if let Some(c) = &mut state.popups.completion {
            let caret = state.selection.primary().head;
            if caret < c.anchor || !(ch.is_alphanumeric() || ch == '_') {
                state.popups.completion = None;
            } else {
                let prefix = state.buffer.rope().slice(c.anchor..caret).to_string();
                c.filter(&prefix);
            }
        }
    } else {
        let edited = commands::replace_selections(&state.selection, text, None);
        state.apply(edited, EditKind::Other);
    }
}

/// Markup typing assists (Razor/HTML/XML), with one caret: `>` closes the tag, `</` completes
/// the open element's name, `=` in a tag adds the attribute's quotes.
fn markup_assist(state: &mut EditorState, ch: char) {
    use rc_syntax::Lang;
    use rc_text::markup;
    let lang = state.syntax.lang;
    if !matches!(lang, Lang::Razor | Lang::Xml) || state.selection.len() != 1 || !state.selection.primary().is_empty() {
        return;
    }
    let html = lang == Lang::Razor;
    let caret = state.caret();
    let rope = state.buffer.rope().clone();
    let mut remove = 0;
    let (text, caret_after) = match ch {
        '>' => match markup::close_tag(&rope, caret, html) {
            Some(t) => (t, caret),
            None => return,
        },
        '/' => match markup::close_open_element(&rope, caret, html) {
            Some(t) => {
                // The closing tag inserted with `>` is right after: typing it replaces that one.
                let dup = format!("</{t}");
                let after: String = rope.slice(caret..(caret + dup.chars().count()).min(rope.len_chars())).chars().collect();
                if after == dup {
                    remove = dup.chars().count();
                }
                let n = t.chars().count();
                (t, caret + n)
            }
            None => return,
        },
        '=' if markup::quote_attribute(&rope, caret) => ("\"\"".to_string(), caret + 1),
        _ => return,
    };
    let cs = rc_text::ChangeSet::single(rc_text::Change::new(caret, caret + remove, text));
    state.apply(commands::Edited { changes: cs, selection: Selection::point(caret_after) }, EditKind::Insert);
}

fn electric_close_brace(rope: &rc_text::Rope, state: &EditorState) -> Option<commands::Edited> {
    let pos = state.caret();
    let line = rope.char_to_line(pos);
    let ls = rope.line_to_char(line);
    let before = rope.slice(ls..pos).to_string();
    if !before.chars().all(|c| c == ' ' || c == '\t') || before.is_empty() {
        return None;
    }
    let unit = state.indent.unit.chars().count();
    let new_indent: String = if state.indent.unit == "\t" {
        before.chars().skip(1).collect()
    } else {
        " ".repeat(before.chars().count().saturating_sub(unit))
    };
    let text = format!("{new_indent}}}");
    let cs = rc_text::ChangeSet::single(rc_text::Change::new(ls, pos, text.clone()));
    Some(commands::Edited { changes: cs, selection: Selection::point(ls + text.chars().count()) })
}

fn copy(ui: &Ui, state: &mut EditorState, cut: bool) {
    let rope = state.buffer.rope().clone();
    let sel = state.selection.clone();
    let any_selected = sel.ranges().iter().any(|r| !r.is_empty());
    let text = if any_selected {
        state.line_clipboard = None;
        sel.ranges().iter().map(|r| rope.slice(r.start()..r.end()).to_string()).collect::<Vec<_>>().join("\n")
    } else {
        // No selection: the whole line (IntelliJ).
        let blocks = commands::selected_line_blocks(&rope, &sel);
        let mut t = String::new();
        for (a, b) in blocks {
            let s = rope.line_to_char(a);
            let e = if b + 1 < rope.len_lines() { rope.line_to_char(b + 1) } else { rope.len_chars() };
            t.push_str(&rope.slice(s..e).to_string());
            if !t.ends_with('\n') {
                t.push('\n');
            }
        }
        state.line_clipboard = Some(t.clone());
        t
    };
    ui.ctx().copy_text(text);
    if cut && !state.read_only {
        let edited = if any_selected { commands::delete_forward(&rope, &sel) } else { commands::delete_lines(&rope, &sel) };
        state.apply(edited, EditKind::Other);
    }
}

fn paste(state: &mut EditorState, text: &str) {
    if state.read_only {
        return;
    }
    let text = text.replace("\r\n", "\n");
    let rope = state.buffer.rope().clone();
    let sel = state.selection.clone();
    // A whole line copied without selection goes above the caret's line.
    if state.line_clipboard.as_deref() == Some(text.as_str()) && sel.ranges().iter().all(|r| r.is_empty()) {
        let changes: Vec<rc_text::Change> = sel
            .ranges()
            .iter()
            .map(|r| rc_text::Change::insert(rope.line_to_char(rope.char_to_line(r.head)), text.clone()))
            .collect();
        let cs = rc_text::ChangeSet::new(changes);
        let n = text.chars().count();
        let after = sel.transform(|r| Range::point(r.head + n));
        state.apply(commands::Edited { changes: cs, selection: after }, EditKind::Other);
        return;
    }
    // As many lines as carets: one line per caret.
    let lines: Vec<String> = text.split('\n').map(str::to_string).collect();
    let per = if sel.len() > 1 && lines.len() == sel.len() { Some(lines) } else { None };
    let edited = commands::replace_selections(&sel, &text, per.as_deref());
    state.apply(edited, EditKind::Other);
}

fn exec(ui: &Ui, state: &mut EditorState, config: &EditorConfig, action: Action, out: &mut EditorOutput) {
    use Action::*;
    let rope = state.buffer.rope().clone();
    let tab = state.indent.tab_width;
    let indent = state.indent.clone();
    let sel = state.selection.clone();
    let page = (ui.available_height() / 20.0).max(5.0) as usize;
    let mv = |state: &mut EditorState, motion: Motion, extend: bool| {
        state.selection = commands::move_selection(&rope, &sel, motion, extend, tab);
        state.buffer.seal_undo();
        state.scroll_to_caret = true;
    };
    let ro = state.read_only;
    match action {
        Left(e) => mv(state, Motion::Left, e),
        Right(e) => mv(state, Motion::Right, e),
        Up(e) => mv(state, Motion::Up, e),
        Down(e) => mv(state, Motion::Down, e),
        WordLeft(e) => mv(state, Motion::WordLeft, e),
        WordRight(e) => mv(state, Motion::WordRight, e),
        LineStart(e) => mv(state, Motion::LineStart, e),
        LineEnd(e) => mv(state, Motion::LineEnd, e),
        DocStart(e) => mv(state, Motion::DocStart, e),
        DocEnd(e) => mv(state, Motion::DocEnd, e),
        PageUp(e) => mv(state, Motion::PageUp(page), e),
        PageDown(e) => mv(state, Motion::PageDown(page), e),
        ScrollUp => state.scroll.y -= 20.0,
        ScrollDown => state.scroll.y += 20.0,
        Enter if !ro => {
            let e = commands::newline(&rope, &sel, &indent);
            state.apply(e, EditKind::Other);
            // The language server may continue a `///` comment on the new line.
            out.events.push(EditorEvent::Typed('\n'));
        }
        StartNewLine if !ro => {
            let end = commands::move_selection(&rope, &sel, Motion::LineEnd, false, tab);
            let e = commands::newline(&rope, &end, &indent);
            state.apply(e, EditKind::Other);
        }
        StartNewLineBefore if !ro => {
            let ranges: Vec<rc_text::Change> = sel
                .ranges()
                .iter()
                .map(|r| {
                    let l = rope.char_to_line(r.head);
                    rc_text::Change::insert(rope.line_to_char(l), format!("{}\n", commands::line_indent(&rope, l)))
                })
                .collect();
            let cs = rc_text::ChangeSet::new(ranges);
            let after = sel.transform(|r| {
                let l = rope.char_to_line(r.head);
                let ind = commands::line_indent(&rope, l).chars().count();
                Range::point(rope.line_to_char(l) + ind)
            });
            // Carets of later lines shift by the inserted lines before them.
            let after = Selection::new(
                after.ranges().iter().enumerate().map(|(i, r)| {
                    let shift: usize = sel.ranges()[..i].iter().map(|o| commands::line_indent(&rope, rope.char_to_line(o.head)).chars().count() + 1).sum();
                    Range::point(r.head + shift)
                }),
                sel.primary_index(),
            );
            state.apply(commands::Edited { changes: cs, selection: after }, EditKind::Other);
        }
        Backspace if !ro => {
            let e = commands::backspace(&rope, &sel, &indent);
            state.apply(e, EditKind::Delete);
            refilter_completion(state);
        }
        Delete if !ro => {
            let e = commands::delete_forward(&rope, &sel);
            state.apply(e, EditKind::Delete);
        }
        DeleteWordLeft if !ro => {
            let e = commands::delete_word(&rope, &sel, true);
            state.apply(e, EditKind::Other);
        }
        DeleteWordRight if !ro => {
            let e = commands::delete_word(&rope, &sel, false);
            state.apply(e, EditKind::Other);
        }
        DeleteLine if !ro => {
            let e = commands::delete_lines(&rope, &sel);
            state.apply(e, EditKind::Other);
        }
        DuplicateLine if !ro => {
            let e = commands::duplicate(&rope, &sel);
            state.apply(e, EditKind::Other);
        }
        MoveLineUp if !ro => {
            let e = commands::move_lines(&rope, &sel, true);
            state.apply(e, EditKind::Other);
        }
        MoveLineDown if !ro => {
            let e = commands::move_lines(&rope, &sel, false);
            state.apply(e, EditKind::Other);
        }
        CommentLine if !ro => {
            if let Some(prefix) = state.lang().line_comment() {
                let e = commands::toggle_line_comment(&rope, &sel, prefix);
                state.apply(e, EditKind::Other);
            } else if let Some((open, close)) = state.lang().block_comment() {
                let e = toggle_block_comment(&rope, &sel, open, close, true);
                state.apply(e, EditKind::Other);
            }
        }
        CommentBlock if !ro => {
            if let Some((open, close)) = state.lang().block_comment() {
                let e = toggle_block_comment(&rope, &sel, open, close, false);
                state.apply(e, EditKind::Other);
            }
        }
        Indent if !ro => {
            let multi_line = sel.ranges().iter().any(|r| rope.char_to_line(r.start()) != rope.char_to_line(r.end()));
            let e = if multi_line { commands::indent_lines(&rope, &sel, &indent, false) } else { commands::insert_tab(&rope, &sel, &indent) };
            state.apply(e, EditKind::Other);
        }
        Unindent if !ro => {
            let e = commands::indent_lines(&rope, &sel, &indent, true);
            state.apply(e, EditKind::Other);
        }
        JoinLines if !ro => {
            let changes: Vec<rc_text::Change> = commands::selected_line_blocks(&rope, &sel)
                .into_iter()
                .filter_map(|(a, b)| {
                    let last = if a == b { a + 1 } else { b };
                    if last >= rope.len_lines() {
                        return None;
                    }
                    let start = rope.line_to_char(a) + rc_text::buffer::line_len(&rope, a);
                    let next = rope.line_to_char(last);
                    let ws = commands::line_indent(&rope, last).chars().count();
                    let _ = start;
                    Some(rc_text::Change::new(next - 1, next + ws, " "))
                })
                .collect();
            let cs = rc_text::ChangeSet::new(changes);
            let after = sel.map(&cs);
            state.apply(commands::Edited { changes: cs, selection: after }, EditKind::Other);
        }
        ToggleCase if !ro => {
            let changes: Vec<rc_text::Change> = sel
                .ranges()
                .iter()
                .filter(|r| !r.is_empty())
                .map(|r| {
                    let t = rope.slice(r.start()..r.end()).to_string();
                    let upper = t.to_uppercase();
                    rc_text::Change::new(r.start(), r.end(), if t == upper { t.to_lowercase() } else { upper })
                })
                .collect();
            let cs = rc_text::ChangeSet::new(changes);
            state.apply(commands::Edited { changes: cs, selection: sel.clone() }, EditKind::Other);
        }
        Undo if !ro => state.undo(),
        Redo if !ro => state.redo(),
        // Copy/cut/paste come as egui events (from the keyboard or the native menu).
        Copy | Cut | Paste => {}
        SelectAll => {
            state.selection = Selection::single(Range::new(0, rope.len_chars()));
        }
        ExtendSelection => {
            state.selection = crate::select::extend(state);
        }
        ShrinkSelection => {
            state.selection = crate::select::shrink(state);
        }
        SelectNextOccurrence => {
            state.selection = commands::select_next_occurrence(&rope, &sel);
            state.scroll_to_caret = true;
        }
        UnselectOccurrence => {
            if sel.len() > 1 {
                let p = sel.primary();
                let rest: Vec<Range> = sel.ranges().iter().copied().filter(|r| *r != p).collect();
                let n = rest.len() - 1;
                state.selection = Selection::new(rest, n);
                state.scroll_to_caret = true;
            }
        }
        SelectAllOccurrences => {
            state.selection = commands::select_all_occurrences(&rope, &sel);
        }
        CloneCaretAbove => state.selection = commands::add_caret_vertical(&rope, &sel, true, tab),
        CloneCaretBelow => state.selection = commands::add_caret_vertical(&rope, &sel, false, tab),
        Escape => {
            if state.popups.any() {
                state.popups.close_all();
            } else if sel.len() > 1 {
                state.selection = sel.clone().into_primary();
            } else if !sel.primary().is_empty() {
                state.selection = Selection::point(sel.primary().head);
            } else {
                out.actions.push(Action::Escape);
            }
        }
        Fold => {
            let line = state.caret_line_col().0;
            state.fold_at(line);
        }
        Unfold => {
            let line = state.caret_line_col().0;
            state.unfold_line(line);
        }
        FoldAll => state.fold_all(),
        UnfoldAll => state.unfold_all(),
        _ => {}
    }
    let _ = config;
}

/// Runs an editing action on `state` from outside the widget (native menu, actions popup).
/// Returns the code-insight actions it could not handle itself.
pub fn run_action(ui: &Ui, state: &mut EditorState, config: &EditorConfig, action: Action) -> Vec<Action> {
    let response = ui.interact(Rect::NOTHING, Id::new("dotcraft_editor_action"), Sense::hover());
    let mut out = EditorOutput { response, actions: Vec::new(), events: Vec::new(), accepted_completion: None, caret_rect: None };
    if action.is_editing() {
        exec(ui, state, config, action, &mut out);
    } else {
        out.actions.push(action);
    }
    out.actions
}

fn refilter_completion(state: &mut EditorState) {
    if let Some(c) = &mut state.popups.completion {
        let caret = state.selection.primary().head;
        if caret < c.anchor {
            state.popups.completion = None;
        } else {
            let prefix = state.buffer.rope().slice(c.anchor..caret).to_string();
            c.filter(&prefix);
        }
    }
}

/// Wraps the selection in `open … close`, or unwraps it; with `whole_lines` and no selection,
/// comments the caret's line (XML has no line comments).
fn toggle_block_comment(rope: &rc_text::Rope, sel: &Selection, open: &str, close: &str, whole_lines: bool) -> commands::Edited {
    let mut changes = Vec::new();
    for r in sel.ranges() {
        let (s, e) = if r.is_empty() && whole_lines {
            let l = rope.char_to_line(r.head);
            let ls = rope.line_to_char(l) + commands::line_indent(rope, l).chars().count();
            (ls, rope.line_to_char(l) + rc_text::buffer::line_len(rope, l))
        } else {
            (r.start(), r.end())
        };
        let text = rope.slice(s..e).to_string();
        let trimmed = text.trim();
        if trimmed.starts_with(open) && trimmed.ends_with(close) && trimmed.len() >= open.len() + close.len() {
            let lead = text.len() - text.trim_start().len();
            let a = s + text[..lead].chars().count();
            let inner_start = a + open.chars().count();
            let b = a + trimmed.chars().count();
            let inner_end = b - close.chars().count();
            let strip_space = |t: &str| t.to_string();
            let inner: String = rope.slice(inner_start..inner_end).to_string();
            let inner = strip_space(inner.strip_prefix(' ').unwrap_or(&inner));
            let inner = inner.strip_suffix(' ').unwrap_or(&inner).to_string();
            changes.push(rc_text::Change::new(a, b, inner));
        } else {
            changes.push(rc_text::Change::insert(s, format!("{open} ")));
            changes.push(rc_text::Change::insert(e, format!(" {close}")));
        }
    }
    let cs = rc_text::ChangeSet::new(changes);
    let selection = sel.map(&cs);
    commands::Edited { changes: cs, selection }
}
