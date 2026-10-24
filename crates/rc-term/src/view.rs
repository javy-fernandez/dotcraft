//! The terminal widget: paints the visible rows of a session's grid (runs of cells with the
//! same style are one section of the row's galley) and turns keyboard, mouse and IME input
//! into bytes for the process.

use crate::colors::{TermColors, dim};
use crate::keys::{self, KeyMode, MouseButton, MouseEncoding};
use crate::links::{self, Link};
use crate::session::{TermSession, TermState};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{Color as AnsiColor, CursorShape, NamedColor};
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, CursorIcon, Event, FontFamily, FontId, Id, Key, PointerButton, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2, pos2, vec2};
use rc_kit::i18n::tr;
use std::ops::Range;

/// Terminal settings shared by every terminal view.
#[derive(Clone, Debug, PartialEq)]
pub struct TermConfig {
    pub font_size: f32,
    /// Multiplier of the font's natural line height.
    pub line_spacing: f32,
    pub cursor_blink: bool,
    /// Option/Alt sends `ESC` + key (off on macOS so Option types `@`, `[`, `{`…).
    pub option_as_meta: bool,
    /// Copy the selection to the clipboard when the mouse button is released.
    pub copy_on_select: bool,
}

impl Default for TermConfig {
    fn default() -> Self {
        TermConfig { font_size: 13.0, line_spacing: 1.0, cursor_blink: true, option_as_meta: !cfg!(target_os = "macos"), copy_on_select: false }
    }
}

/// Something the integrator may act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewEvent {
    /// ⌘/Ctrl+click on a URL or a file path.
    LinkClicked(Link),
    Focused,
}

pub struct TermOutput {
    pub response: Response,
    pub events: Vec<ViewEvent>,
}

const PAD_X: f32 = 6.0;
const PAD_Y: f32 = 4.0;
const SCROLLBAR_W: f32 = 10.0;
const BLINK: f64 = 0.53;
const MULTI_CLICK: f64 = 0.4;

/// Per-widget UI state, kept in egui's memory.
#[derive(Clone, Default)]
struct ViewState {
    blink_epoch: f64,
    scroll_acc: f32,
    last_press: Option<(f64, Pos2)>,
    clicks: u8,
    selecting: bool,
    reported: Option<MouseButton>,
    last_cell: Option<(usize, usize)>,
    preedit: Option<String>,
    suppress_text: bool,
    link_press: Option<Link>,
    bar_top: f32,
}

/// Geometry of the current frame. Cell edges are snapped to physical pixels so backgrounds
/// of adjacent runs touch without gaps and glyphs sit on their cells.
#[derive(Clone, Copy)]
struct Metrics {
    origin: Pos2,
    cell_w: f32,
    row_h: f32,
    text_dy: f32,
    cols: usize,
    rows: usize,
    ppp: f32,
}

impl Metrics {
    fn snap(&self, v: f32) -> f32 {
        (v * self.ppp).round() / self.ppp
    }
    fn x(&self, col: usize) -> f32 {
        self.snap(self.origin.x + col as f32 * self.cell_w)
    }
    fn y(&self, row: usize) -> f32 {
        self.origin.y + row as f32 * self.row_h
    }
    fn cell_rect(&self, row: usize, col: usize, width: usize) -> Rect {
        Rect::from_min_max(pos2(self.x(col), self.y(row)), pos2(self.x(col + width), self.y(row + 1)))
    }
    /// Viewport cell under `p` (clamped) and the half of the cell it is on.
    fn cell_at(&self, p: Pos2) -> (usize, usize, Side) {
        let fx = ((p.x - self.origin.x) / self.cell_w).max(0.0);
        let col = (fx.floor() as usize).min(self.cols - 1);
        let side = if fx - col as f32 >= 0.5 || fx >= self.cols as f32 { Side::Right } else { Side::Left };
        let row = (((p.y - self.origin.y) / self.row_h).floor().max(0.0) as usize).min(self.rows - 1);
        (row, col, side)
    }
}

/// Font variants of a cell.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Face {
    Regular,
    Bold,
    Italic,
}

/// Style of a run of cells (one layout section).
#[derive(Clone, Copy, PartialEq)]
struct Style {
    fg: Color32,
    face: Face,
    underline: bool,
    strike: bool,
}

pub struct TerminalView<'a> {
    session: &'a TermSession,
    config: &'a TermConfig,
    id: Id,
    focus: bool,
}

impl<'a> TerminalView<'a> {
    pub fn new(session: &'a TermSession, config: &'a TermConfig) -> Self {
        TerminalView { session, config, id: Self::id_for(session.id()), focus: false }
    }

    /// Widget id of the view of session `id` (to request focus from outside).
    pub fn id_for(session_id: u64) -> Id {
        Id::new(("dotcraft_terminal", session_id))
    }

    /// Requests keyboard focus this frame.
    pub fn focus(mut self, focus: bool) -> Self {
        self.focus = focus;
        self
    }

    pub fn show(self, ui: &mut Ui) -> TermOutput {
        let TerminalView { session, config, id, focus } = self;
        let dark = ui.visuals().dark_mode;
        session.set_dark(dark);
        let rect = ui.available_rect_before_wrap();
        let response = ui.interact(rect, id, Sense::click_and_drag());
        ui.advance_cursor_after_rect(rect);
        let mut events = Vec::new();
        if focus {
            response.request_focus();
        }
        let mut vs: ViewState = ui.data(|d| d.get_temp(id)).unwrap_or_default();
        let now = ui.input(|i| i.time);

        // ---------------------------------------------------------------- geometry
        let m = metrics(ui, config, rect);
        let cell_px = ((m.cell_w * m.ppp).round() as u16, (m.row_h * m.ppp).round() as u16);
        session.resize(m.cols as u16, m.rows as u16, cell_px.0, cell_px.1);

        let focused = response.has_focus();
        if focused {
            ui.memory_mut(|mem| mem.set_focus_lock_filter(id, egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true }));
        }
        let gained = response.gained_focus();
        let lost = response.lost_focus();
        if gained {
            events.push(ViewEvent::Focused);
            vs.blink_epoch = now;
        }

        let mut st = session.lock();
        if let Some(deadline) = st.flush_sync() {
            ui.ctx().request_repaint_after(deadline.saturating_duration_since(std::time::Instant::now()));
        }
        // The term may not have caught up with a resize from another view of the session.
        if st.term.columns() != m.cols || st.term.screen_lines() != m.rows {
            drop(st);
            ui.ctx().request_repaint();
            return TermOutput { response, events };
        }
        st.term.is_focused = focused;
        let mode = st.mode();
        if (gained || lost) && mode.contains(TermMode::FOCUS_IN_OUT) {
            session.write(if gained { b"\x1b[I".to_vec() } else { b"\x1b[O".to_vec() });
        }

        // ---------------------------------------------------------------- input
        let mac = cfg!(target_os = "macos");
        if focused {
            handle_keyboard(ui, session, &mut st, config, &mut vs, mac);
        }
        let hovered_link = handle_mouse(ui, session, &mut st, config, &m, rect, &response, &mut vs, &mut events, mac);
        handle_wheel(ui, session, &mut st, &m, &response, &mut vs);

        // ---------------------------------------------------------------- paint
        let colors = TermColors::for_theme(dark);
        let painter = ui.painter_at(rect);
        let base_bg = colors.resolve(AnsiColor::Named(NamedColor::Background), st.term.colors());
        painter.rect_filled(rect, 0.0, base_bg);
        let blink_on = !config.cursor_blink || ((now - vs.blink_epoch) % (2.0 * BLINK)) < BLINK;
        paint_grid(ui, &painter, &st, config, &colors, &m, focused, blink_on, hovered_link.as_ref());
        paint_scrollbar(ui, &mut st, &m, rect, id, &mut vs);

        // IME: candidate window at the cursor, preedit text drawn over it.
        let cursor = st.term.grid().cursor.point;
        let cursor_row = cursor.line.0 + st.term.grid().display_offset() as i32;
        drop(st);
        if focused && (0..m.rows as i32).contains(&cursor_row) {
            let cell = m.cell_rect(cursor_row as usize, cursor.column.0.min(m.cols - 1), 1);
            if let Some(pre) = vs.preedit.as_ref().filter(|p| !p.is_empty()) {
                let g = painter.layout_no_wrap(pre.clone(), FontId::monospace(config.font_size), colors.fg);
                let r = Rect::from_min_size(cell.min, vec2(g.size().x, m.row_h));
                painter.rect_filled(r, 0.0, base_bg);
                painter.galley(pos2(r.left(), r.top() + m.text_dy), g, colors.fg);
                painter.hline(r.x_range(), r.bottom() - 1.0, Stroke::new(1.0, colors.fg));
            }
            ui.output_mut(|o| {
                o.ime = Some(egui::output::IMEOutput { purpose: Default::default(), rect, cursor_rect: cell, should_interrupt_composition: false })
            });
        }
        if focused && config.cursor_blink {
            let t = (now - vs.blink_epoch) % BLINK;
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(BLINK - t + 0.005));
        }
        if hovered_link.is_some() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        } else if response.hovered() || response.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::Text);
        }

        context_menu(&response, session);
        ui.data_mut(|d| d.insert_temp(id, vs));
        TermOutput { response, events }
    }
}

fn metrics(ui: &Ui, config: &TermConfig, rect: Rect) -> Metrics {
    let font = FontId::monospace(config.font_size);
    let (cell_w, font_h) = ui.fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
    let ppp = ui.ctx().pixels_per_point();
    let snap = |v: f32| (v * ppp).round() / ppp;
    let row_h = snap((config.font_size * 1.32 * config.line_spacing).max(font_h));
    let text_dy = snap((row_h - font_h) / 2.0);
    let origin = pos2(snap(rect.left() + PAD_X), snap(rect.top() + PAD_Y));
    let w = rect.right() - SCROLLBAR_W - origin.x;
    let h = rect.bottom() - origin.y;
    let cols = ((w / cell_w).floor() as usize).max(2);
    let rows = ((h / row_h).floor() as usize).max(1);
    Metrics { origin, cell_w, row_h, text_dy, cols, rows, ppp }
}

fn fonts(ui: &Ui, size: f32) -> [FontId; 3] {
    let has = |name: &str| ui.fonts(|f| f.definitions().families.contains_key(&FontFamily::Name(name.into())));
    let regular = FontId::monospace(size);
    let named = |name: &str| if has(name) { FontId::new(size, FontFamily::Name(name.into())) } else { regular.clone() };
    [regular.clone(), named(rc_kit::theme::FONT_MONO_BOLD), named(rc_kit::theme::FONT_MONO_ITALIC)]
}

// =====================================================================================
// Painting
// =====================================================================================

#[allow(clippy::too_many_arguments)]
fn paint_grid(ui: &Ui, painter: &egui::Painter, st: &TermState, config: &TermConfig, colors: &TermColors, m: &Metrics, focused: bool, blink_on: bool, link: Option<&(usize, Range<usize>, Link)>) {
    let term = &st.term;
    let grid = term.grid();
    let offset = grid.display_offset();
    let overrides = term.colors();
    let content = term.renderable_content();
    let selection = content.selection;
    let cursor = content.cursor;
    let base_bg = colors.resolve(AnsiColor::Named(NamedColor::Background), overrides);
    let fonts = fonts(ui, config.font_size);
    let font_of = |f: Face| match f {
        Face::Regular => fonts[0].clone(),
        Face::Bold => fonts[1].clone(),
        Face::Italic => fonts[2].clone(),
    };
    let caret = colors.resolve(AnsiColor::Named(NamedColor::Cursor), overrides);

    // Cursor cell in viewport coordinates.
    let cursor_vp = {
        let row = cursor.point.line.0 + offset as i32;
        (cursor.shape != CursorShape::Hidden && (0..m.rows as i32).contains(&row)).then_some((row as usize, cursor.point.column.0))
    };
    let cursor_wide = cursor_vp.is_some_and(|_| grid[cursor.point].flags.contains(Flags::WIDE_CHAR));
    let block_cursor = focused && blink_on && cursor.shape == CursorShape::Block;

    let mut backgrounds: Vec<Shape> = Vec::new();
    let mut texts: Vec<Shape> = Vec::new();
    let mut singles: Vec<(Pos2, f32, String, Style)> = Vec::new();

    for row in 0..m.rows {
        let line = Line(row as i32 - offset as i32);
        let cells = &grid[line];
        let y = m.y(row);
        let mut job = LayoutJob { break_on_newline: false, ..Default::default() };
        job.wrap.max_width = f32::INFINITY;
        let mut run_text = String::new();
        let mut run_style: Option<Style> = None;
        let mut bg_run: Option<(usize, Color32)> = None;
        let flush_bg = |bg_run: &mut Option<(usize, Color32)>, end: usize, backgrounds: &mut Vec<Shape>| {
            if let Some((start, color)) = bg_run.take()
                && color != base_bg
            {
                let r = Rect::from_min_max(pos2(m.x(start), y), pos2(m.x(end), m.y(row + 1)));
                backgrounds.push(Shape::rect_filled(r, 0.0, color));
            }
        };
        let push_run = |job: &mut LayoutJob, text: &mut String, style: Style| {
            if text.is_empty() {
                return;
            }
            let format = TextFormat {
                font_id: font_of(style.face),
                color: style.fg,
                underline: if style.underline { Stroke::new(1.0, style.fg) } else { Stroke::NONE },
                strikethrough: if style.strike { Stroke::new(1.0, style.fg) } else { Stroke::NONE },
                ..Default::default()
            };
            job.append(text, 0.0, format);
            text.clear();
        };

        for col in 0..m.cols {
            let cell = &cells[Column(col)];
            let flags = cell.flags;
            let point = Point::new(line, Column(col));
            let mut fg = colors.resolve(cell.fg, overrides);
            let mut bg = colors.resolve(cell.bg, overrides);
            if flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            if flags.contains(Flags::DIM) {
                fg = dim(fg, bg);
            }
            if flags.contains(Flags::HIDDEN) {
                fg = bg;
            }
            if selection.is_some_and(|s| s.contains(point)) {
                bg = colors.selection;
            }
            let in_link = link.is_some_and(|(r, cols, _)| *r == row && cols.contains(&col));
            if in_link {
                fg = colors.link;
            }
            let on_cursor = cursor_vp.is_some_and(|(r, c)| r == row && (c == col || (cursor_wide && c + 1 == col)));
            if on_cursor && block_cursor {
                fg = base_bg;
                bg = caret;
            }
            let face = if flags.contains(Flags::BOLD) {
                Face::Bold
            } else if flags.contains(Flags::ITALIC) {
                Face::Italic
            } else {
                Face::Regular
            };
            let style = Style { fg, face, underline: in_link || flags.intersects(Flags::ALL_UNDERLINES), strike: flags.contains(Flags::STRIKEOUT) };

            // Background runs.
            match bg_run {
                Some((_, c)) if c == bg => {}
                _ => {
                    flush_bg(&mut bg_run, col, &mut backgrounds);
                    bg_run = Some((col, bg));
                }
            }

            // Text runs: wide chars and glyphs that are not one cell wide (fallback fonts)
            // are painted on their own so they never shift the rest of the row.
            let spacer = flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER);
            let c = if spacer || cell.c.is_control() { ' ' } else { cell.c };
            let wide = flags.contains(Flags::WIDE_CHAR);
            let fits = !wide && (c.is_ascii() || (ui.fonts_mut(|f| (f.glyph_width(&fonts[0], c) - m.cell_w).abs() < 0.01) && cell.zerowidth().is_none()));
            let ch = if fits {
                c
            } else {
                let mut s = String::from(c);
                if let Some(zw) = cell.zerowidth() {
                    s.extend(zw);
                }
                let width = if wide { 2 } else { 1 };
                let span = m.x(col + width) - m.x(col);
                singles.push((pos2(m.x(col), y + m.text_dy), span, s, style));
                ' '
            };
            if run_style != Some(style) {
                if let Some(prev) = run_style {
                    push_run(&mut job, &mut run_text, prev);
                }
                run_style = Some(style);
            }
            run_text.push(ch);
        }
        flush_bg(&mut bg_run, m.cols, &mut backgrounds);
        if let Some(style) = run_style {
            // Trailing blanks without decoration need no glyphs.
            if !style.underline && !style.strike {
                let trimmed = run_text.trim_end_matches(' ').len();
                run_text.truncate(trimmed);
            }
            push_run(&mut job, &mut run_text, style);
        }
        if !job.sections.is_empty() && !job.text.trim().is_empty() || job.sections.iter().any(|s| s.format.underline != Stroke::NONE || s.format.strikethrough != Stroke::NONE) {
            let galley = ui.fonts_mut(|f| f.layout_job(job));
            texts.push(Shape::galley(pos2(m.x(0), y + m.text_dy), galley, colors.fg));
        }
    }

    painter.extend(backgrounds);
    painter.extend(texts);
    for (pos, span, s, style) in singles {
        let g = painter.layout_no_wrap(s, font_of(style.face), style.fg);
        let x = ((pos.x + (span - g.size().x) / 2.0) * m.ppp).round() / m.ppp;
        if style.underline {
            painter.hline(pos.x..=pos.x + span, pos.y + g.size().y - 1.0, Stroke::new(1.0, style.fg));
        }
        painter.galley(pos2(x, pos.y), g, style.fg);
    }

    // Cursor shapes other than the focused block.
    if let Some((row, col)) = cursor_vp {
        let r = m.cell_rect(row, col, if cursor_wide { 2 } else { 1 });
        if !focused || cursor.shape == CursorShape::HollowBlock {
            painter.rect_stroke(r.shrink(0.5), 0.0, Stroke::new(1.0, caret), egui::StrokeKind::Inside);
        } else if blink_on {
            match cursor.shape {
                CursorShape::Beam => {
                    painter.rect_filled(Rect::from_min_size(r.min, vec2(2.0, r.height())), 0.0, caret);
                }
                CursorShape::Underline => {
                    painter.rect_filled(Rect::from_min_max(pos2(r.left(), r.bottom() - 2.0), r.max), 0.0, caret);
                }
                _ => {}
            }
        }
    }
}

fn paint_scrollbar(ui: &Ui, st: &mut TermState, m: &Metrics, rect: Rect, id: Id, vs: &mut ViewState) {
    let history = st.term.grid().history_size();
    if history == 0 {
        return;
    }
    let pal = rc_kit::pal(ui);
    let track = Rect::from_min_max(pos2(rect.right() - SCROLLBAR_W, rect.top()), rect.max);
    let resp = ui.interact(track, id.with("vbar"), Sense::click_and_drag());
    let total = (history + m.rows) as f32;
    let thumb_h = (track.height() * m.rows as f32 / total).max(24.0).min(track.height());
    let range = (track.height() - thumb_h).max(1.0);
    let offset = st.term.grid().display_offset();
    let top = (history - offset) as f32;
    if resp.drag_started() {
        vs.bar_top = top;
    }
    let mut target: Option<f32> = None;
    if resp.dragged() {
        vs.bar_top = (vs.bar_top + resp.drag_delta().y / range * history as f32).clamp(0.0, history as f32);
        target = Some(vs.bar_top);
    } else if resp.clicked()
        && let Some(p) = resp.interact_pointer_pos()
    {
        let t = ((p.y - track.top() - thumb_h / 2.0) / range).clamp(0.0, 1.0);
        target = Some(t * history as f32);
    }
    if let Some(top_line) = target {
        let new_offset = history as i32 - top_line.round() as i32;
        st.term.scroll_display(Scroll::Delta(new_offset - offset as i32));
    }
    let top = (history - st.term.grid().display_offset()) as f32;
    let thumb_y = track.top() + range * (top / history as f32);
    let active = resp.hovered() || resp.dragged();
    let w = if active { 8.0 } else { 6.0 };
    let thumb = Rect::from_min_size(pos2(track.right() - w - 2.0, thumb_y), vec2(w, thumb_h));
    ui.painter_at(rect).rect_filled(thumb, 4.0, pal.text.gamma_multiply(if active { 0.5 } else { 0.28 }));
}

// =====================================================================================
// Input
// =====================================================================================

fn handle_keyboard(ui: &Ui, session: &TermSession, st: &mut TermState, config: &TermConfig, vs: &mut ViewState, mac: bool) {
    let (events, mods_now) = ui.input(|i| (i.events.clone(), i.modifiers));
    let now = ui.input(|i| i.time);
    let mut sent = false;
    let mut send = |bytes: Vec<u8>, st: &mut TermState| {
        if !bytes.is_empty() {
            session.write(bytes);
            st.term.scroll_display(Scroll::Bottom);
            sent = true;
        }
    };
    for ev in events {
        match ev {
            Event::Key { key, pressed: true, modifiers, .. } => {
                let mode = st.mode();
                // Local scrolling.
                if modifiers.shift && !modifiers.ctrl && !modifiers.alt && !mode.contains(TermMode::ALT_SCREEN) {
                    let scroll = match key {
                        Key::PageUp => Some(Scroll::PageUp),
                        Key::PageDown => Some(Scroll::PageDown),
                        Key::Home => Some(Scroll::Top),
                        Key::End => Some(Scroll::Bottom),
                        _ => None,
                    };
                    if let Some(s) = scroll {
                        st.term.scroll_display(s);
                        continue;
                    }
                }
                if mac && modifiers.mac_cmd && key == Key::K {
                    clear_locked(session, st);
                    continue;
                }
                let km = KeyMode { app_cursor: mode.contains(TermMode::APP_CURSOR), alt_meta: config.option_as_meta };
                if let Some(bytes) = keys::key_bytes(key, modifiers, km) {
                    // Alt-as-Meta letters also arrive as text (`∫` for ⌥B): drop it.
                    vs.suppress_text = modifiers.alt && config.option_as_meta && !modifiers.ctrl;
                    send(bytes, st);
                }
            }
            Event::Text(text) => {
                if std::mem::take(&mut vs.suppress_text) || text.is_empty() {
                    continue;
                }
                send(keys::text_bytes(&text), st);
            }
            Event::Paste(text) => {
                if !mac && mods_now.ctrl && !mods_now.shift {
                    send(vec![0x16], st); // Ctrl+V
                } else {
                    send(keys::paste_bytes(&text, st.mode().contains(TermMode::BRACKETED_PASTE)), st);
                }
            }
            Event::Copy => {
                let selected = st.selection_text();
                if mac || mods_now.shift {
                    if let Some(t) = selected {
                        ui.ctx().copy_text(t);
                    }
                } else if let Some(t) = selected {
                    // IntelliJ on Windows/Linux: Ctrl+C copies a selection, else interrupts.
                    ui.ctx().copy_text(t);
                    st.term.selection = None;
                } else {
                    send(vec![0x03], st);
                }
            }
            Event::Cut => {
                if !mac && !mods_now.shift {
                    send(vec![0x18], st); // Ctrl+X
                }
            }
            Event::Ime(ime) => match ime {
                egui::ImeEvent::Preedit { text, .. } => vs.preedit = Some(text),
                egui::ImeEvent::Commit(text) => {
                    vs.preedit = None;
                    send(keys::text_bytes(&text), st);
                }
                _ => {}
            },
            _ => {}
        }
    }
    if sent {
        vs.blink_epoch = now;
    }
}

fn clear_locked(session: &TermSession, st: &mut TermState) {
    st.term.grid_mut().clear_history();
    st.term.scroll_display(Scroll::Bottom);
    st.term.selection = None;
    st.generation += 1;
    if session.is_running() {
        session.write(vec![0x0c]);
    }
}

fn mouse_encoding(mode: TermMode) -> MouseEncoding {
    if mode.contains(TermMode::SGR_MOUSE) {
        MouseEncoding::Sgr
    } else if mode.contains(TermMode::UTF8_MOUSE) {
        MouseEncoding::Utf8
    } else {
        MouseEncoding::X10
    }
}

/// Text of viewport row `row` with the column of every char, for link detection.
fn row_chars(st: &TermState, row: usize) -> (String, Vec<usize>) {
    let grid = st.term.grid();
    let line = Line(row as i32 - grid.display_offset() as i32);
    let mut s = String::new();
    let mut cols = Vec::new();
    for col in 0..grid.columns() {
        let cell = &grid[line][Column(col)];
        if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
            continue;
        }
        s.push(cell.c);
        cols.push(col);
    }
    (s, cols)
}

/// Link under viewport cell (`row`, `col`): an OSC 8 hyperlink or detected text.
fn link_at_cell(st: &TermState, row: usize, col: usize) -> Option<(usize, Range<usize>, Link)> {
    let grid = st.term.grid();
    let line = Line(row as i32 - grid.display_offset() as i32);
    if let Some(h) = grid[line][Column(col)].hyperlink() {
        let same = |c: usize| grid[line][Column(c)].hyperlink().is_some_and(|o| o == h);
        let start = (0..=col).rev().take_while(|&c| same(c)).last().unwrap_or(col);
        let end = (col..grid.columns()).take_while(|&c| same(c)).last().unwrap_or(col) + 1;
        return Some((row, start..end, Link::Url(h.uri().to_string())));
    }
    let (text, cols) = row_chars(st, row);
    let idx = cols.iter().rposition(|&c| c <= col)?;
    let (range, link) = links::link_at(&text, idx)?;
    let start = cols[range.start];
    let end = cols.get(range.end).copied().unwrap_or(cols[range.end - 1] + 1);
    Some((row, start..end, link))
}

#[allow(clippy::too_many_arguments)]
fn handle_mouse(
    ui: &Ui,
    session: &TermSession,
    st: &mut TermState,
    config: &TermConfig,
    m: &Metrics,
    rect: Rect,
    response: &Response,
    vs: &mut ViewState,
    out: &mut Vec<ViewEvent>,
    mac: bool,
) -> Option<(usize, Range<usize>, Link)> {
    let (events, mods, hover, now, primary_down) = ui.input(|i| (i.events.clone(), i.modifiers, i.pointer.hover_pos(), i.time, i.pointer.primary_down()));
    let text_rect = Rect::from_min_max(rect.min, pos2(rect.right() - SCROLLBAR_W, rect.bottom()));
    let mode = st.mode();
    let reporting = mode.intersects(TermMode::MOUSE_MODE) && !mods.shift;
    let link_mod = if mac { mods.mac_cmd } else { mods.ctrl };
    let offset = st.term.grid().display_offset();
    let to_point = |row: usize, col: usize| Point::new(Line(row as i32 - offset as i32), Column(col));
    let over = |p: Pos2| text_rect.contains(p) && response.contains_pointer();

    let hovered_link = match hover {
        Some(p) if link_mod && over(p) && !vs.selecting => {
            let (row, col, _) = m.cell_at(p);
            link_at_cell(st, row, col)
        }
        _ => None,
    };

    for ev in &events {
        match *ev {
            Event::PointerButton { pos, button, pressed: true, modifiers } if over(pos) => {
                response.request_focus();
                let (row, col, side) = m.cell_at(pos);
                if button == PointerButton::Primary
                    && link_mod
                    && let Some((_, _, link)) = link_at_cell(st, row, col)
                {
                    vs.link_press = Some(link);
                    continue;
                }
                if reporting && mode.contains(TermMode::MOUSE_REPORT_CLICK) || reporting && mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION) {
                    let b = match button {
                        PointerButton::Primary => MouseButton::Left,
                        PointerButton::Middle => MouseButton::Middle,
                        PointerButton::Secondary => MouseButton::Right,
                        _ => continue,
                    };
                    if let Some(bytes) = keys::mouse_report(b, true, false, col, row, modifiers, mouse_encoding(mode)) {
                        session.write(bytes);
                    }
                    vs.reported = Some(b);
                    vs.last_cell = Some((row, col));
                    continue;
                }
                if button != PointerButton::Primary {
                    continue;
                }
                let again = vs.last_press.is_some_and(|(t, p)| now - t < MULTI_CLICK && p.distance(pos) < 4.0);
                vs.clicks = if again { vs.clicks % 3 + 1 } else { 1 };
                vs.last_press = Some((now, pos));
                let point = to_point(row, col);
                if modifiers.shift
                    && vs.clicks == 1
                    && let Some(sel) = st.term.selection.as_mut()
                {
                    sel.update(point, side);
                } else {
                    let ty = match vs.clicks {
                        2 => SelectionType::Semantic,
                        3 => SelectionType::Lines,
                        _ if modifiers.alt => SelectionType::Block,
                        _ => SelectionType::Simple,
                    };
                    st.term.selection = Some(Selection::new(ty, point, side));
                }
                vs.selecting = true;
            }
            Event::PointerButton { pos, button, pressed: false, modifiers } => {
                if let Some(link) = vs.link_press.take() {
                    if over(pos) {
                        let (row, col, _) = m.cell_at(pos);
                        if link_at_cell(st, row, col).is_some_and(|(_, _, l)| l == link) {
                            out.push(ViewEvent::LinkClicked(link));
                        }
                    }
                    continue;
                }
                if let Some(b) = vs.reported.take() {
                    let (row, col, _) = m.cell_at(pos);
                    if let Some(bytes) = keys::mouse_report(b, false, false, col, row, modifiers, mouse_encoding(mode)) {
                        session.write(bytes);
                    }
                    continue;
                }
                if button == PointerButton::Primary && vs.selecting {
                    vs.selecting = false;
                    if st.term.selection.as_ref().is_some_and(|s| s.is_empty()) {
                        st.term.selection = None;
                    } else if config.copy_on_select
                        && let Some(t) = st.selection_text()
                    {
                        ui.ctx().copy_text(t);
                    }
                }
            }
            Event::PointerMoved(pos) => {
                let (row, col, side) = m.cell_at(pos);
                if reporting {
                    let motion = match vs.reported {
                        Some(b) if mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION) => Some(b),
                        None if mode.contains(TermMode::MOUSE_MOTION) && over(pos) => Some(MouseButton::None),
                        _ => None,
                    };
                    if let Some(b) = motion
                        && vs.last_cell != Some((row, col))
                    {
                        vs.last_cell = Some((row, col));
                        if let Some(bytes) = keys::mouse_report(b, true, true, col, row, mods, mouse_encoding(mode)) {
                            session.write(bytes);
                        }
                    }
                }
                if vs.selecting
                    && let Some(sel) = st.term.selection.as_mut()
                {
                    sel.update(to_point(row, col), side);
                }
            }
            _ => {}
        }
    }
    if vs.selecting && !primary_down {
        vs.selecting = false;
    }

    // Dragging a selection past the top or bottom scrolls.
    if vs.selecting
        && let Some(p) = hover.or_else(|| ui.input(|i| i.pointer.interact_pos()))
    {
        let delta = if p.y < m.origin.y { 1 } else if p.y > m.y(m.rows) { -1 } else { 0 };
        if delta != 0 {
            st.term.scroll_display(Scroll::Delta(delta));
            let offset = st.term.grid().display_offset();
            let (row, col, side) = m.cell_at(p);
            if let Some(sel) = st.term.selection.as_mut() {
                sel.update(Point::new(Line(row as i32 - offset as i32), Column(col)), side);
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(30));
        }
    }
    hovered_link
}

fn handle_wheel(ui: &Ui, session: &TermSession, st: &mut TermState, m: &Metrics, response: &Response, vs: &mut ViewState) {
    if !response.contains_pointer() {
        return;
    }
    let (delta, mods, hover) = ui.input(|i| (i.smooth_scroll_delta, i.modifiers, i.pointer.hover_pos()));
    if delta.y == 0.0 {
        return;
    }
    ui.ctx().input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
    vs.scroll_acc += delta.y / m.row_h;
    let lines = vs.scroll_acc.trunc() as i32;
    if lines == 0 {
        return;
    }
    vs.scroll_acc -= lines as f32;
    let mode = st.mode();
    if mode.intersects(TermMode::MOUSE_MODE) && !mods.shift {
        let (row, col, _) = hover.map_or((0, 0, Side::Left), |p| m.cell_at(p));
        let b = if lines > 0 { MouseButton::WheelUp } else { MouseButton::WheelDown };
        for _ in 0..lines.unsigned_abs().min(10) {
            if let Some(bytes) = keys::mouse_report(b, true, false, col, row, mods, mouse_encoding(mode)) {
                session.write(bytes);
            }
        }
    } else if mode.contains(TermMode::ALT_SCREEN) && mode.contains(TermMode::ALTERNATE_SCROLL) {
        // Full-screen apps without mouse support (less, man): the wheel sends arrows.
        let app = mode.contains(TermMode::APP_CURSOR);
        let arrow: &[u8] = match (lines > 0, app) {
            (true, true) => b"\x1bOA",
            (true, false) => b"\x1b[A",
            (false, true) => b"\x1bOB",
            (false, false) => b"\x1b[B",
        };
        session.write(arrow.repeat(lines.unsigned_abs() as usize));
    } else {
        st.term.scroll_display(Scroll::Delta(lines));
    }
}

fn context_menu(response: &Response, session: &TermSession) {
    response.context_menu(|ui| {
        let has_selection = session.selection_text().is_some();
        if ui.add_enabled(has_selection, egui::Button::new(tr("Copy"))).clicked() {
            if let Some(t) = session.selection_text() {
                ui.ctx().copy_text(t);
            }
            ui.close();
        }
        if ui.button(tr("Paste")).clicked() {
            response.request_focus();
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::RequestPaste);
            ui.close();
        }
        if ui.button(tr("Select All")).clicked() {
            session.select_all();
            ui.close();
        }
        ui.separator();
        if ui.button(tr("Clear Buffer")).clicked() {
            session.clear();
            ui.close();
        }
    });
}

