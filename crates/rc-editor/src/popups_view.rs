//! Drawing of the code-insight popups and their keyboard handling.

use crate::popup::Completion;
use crate::state::{EditorState, Severity};
use crate::view::{EditorOutput, Metrics};
use egui::{Align2, Color32, FontId, Id, Key, Modifiers, Order, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};
use rc_kit::{Palette, Scheme, ico, icons};
use rc_text::Selection;

const ROW_H: f32 = 24.0;
const MAX_ROWS: usize = 12;
const LIST_W: f32 = 460.0;

/// Keys for an open completion list; returns whether the key was consumed.
pub(crate) fn handle_key(state: &mut EditorState, key: Key, mods: Modifiers, out: &mut EditorOutput) -> bool {
    if mods.command || mods.alt {
        return false;
    }
    if let Some(c) = &mut state.popups.completion {
        match key {
            Key::ArrowDown => c.move_selection(1),
            Key::ArrowUp => c.move_selection(-1),
            Key::PageDown => c.move_selection(MAX_ROWS as isize - 1),
            Key::PageUp => c.move_selection(-(MAX_ROWS as isize - 1)),
            Key::Enter | Key::Tab => {
                if c.filtered.is_empty() {
                    state.popups.completion = None;
                    return false;
                }
                let index = c.selected;
                let data = c.filtered.get(index).map(|&i| c.items[i].data);
                accept(state, index, key == Key::Tab);
                out.accepted_completion = data;
            }
            Key::Escape => state.popups.completion = None,
            _ => return false,
        }
        return true;
    }
    if key == Key::Escape && (state.popups.signature.is_some() || state.popups.hover.is_some()) {
        state.popups.signature = None;
        state.popups.hover = None;
        return true;
    }
    false
}

/// Inserts the chosen completion: replaces the typed prefix (with Tab, also the rest of the
/// identifier after the caret, like IntelliJ's "replace" completion).
pub fn accept(state: &mut EditorState, index: usize, replace_word: bool) {
    let Some(c) = state.popups.completion.take() else { return };
    let Some(&i) = c.filtered.get(index) else { return };
    let item = &c.items[i];
    let caret = state.caret();
    let (start, mut end) = item.replace.unwrap_or((c.anchor, caret));
    end = end.max(caret);
    if replace_word {
        let rope = state.buffer.rope();
        while end < rope.len_chars() && {
            let ch = rope.char(end);
            ch.is_alphanumeric() || ch == '_'
        } {
            end += 1;
        }
    }
    let (text, caret_offset) = expand_snippet(&item.insert);
    state.replace(start, end, &text);
    let pos = start + caret_offset.unwrap_or(text.chars().count());
    state.selection = Selection::point(pos);
    state.request_scroll_to_caret();
}

/// Strips LSP snippet syntax: `${1:name}` → `name`, `$1`/`$0` → nothing. Returns the text and
/// the char offset of the first tab stop (`$1`, or `$0`) when there is one.
pub fn expand_snippet(s: &str) -> (String, Option<usize>) {
    let mut out = String::new();
    let mut stops: Vec<(u32, usize)> = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            out.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if c == '$' && i + 1 < chars.len() {
            if chars[i + 1].is_ascii_digit() {
                let mut j = i + 1;
                let mut n = 0u32;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    n = n * 10 + chars[j].to_digit(10).unwrap_or(0);
                    j += 1;
                }
                stops.push((n, out.chars().count()));
                i = j;
                continue;
            }
            if chars[i + 1] == '{' {
                let mut j = i + 2;
                let mut n = 0u32;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    n = n * 10 + chars[j].to_digit(10).unwrap_or(0);
                    j += 1;
                }
                let at = out.chars().count();
                if j < chars.len() && chars[j] == ':' {
                    j += 1;
                    let mut depth = 1;
                    while j < chars.len() {
                        match chars[j] {
                            '{' => depth += 1,
                            '}' => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                        out.push(chars[j]);
                        j += 1;
                    }
                }
                stops.push((n, at));
                i = j + 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    let first = stops.iter().filter(|(n, _)| *n > 0).min_by_key(|(n, _)| *n).or_else(|| stops.iter().find(|(n, _)| *n == 0)).map(|(_, p)| *p);
    (out, first)
}

pub(crate) fn show(ui: &Ui, state: &mut EditorState, scheme: &Scheme, pal: &Palette, m: &Metrics, text_rect: Rect, focused: bool) -> Option<usize> {
    let mut accepted = None;
    // A completion whose anchor is no longer before the caret is stale.
    if let Some(c) = &state.popups.completion {
        let caret = state.caret();
        let anchor_line = state.buffer.line_col(c.anchor).0;
        if caret < c.anchor || state.buffer.line_col(caret).0 != anchor_line || state.selection.len() > 1 || !focused {
            state.popups.completion = None;
        }
    }
    if let Some(sig) = state.popups.signature.clone() {
        let anchor = crate::view::caret_anchor(ui, state, scheme, m, text_rect, sig.anchor);
        show_signature(ui, &sig, scheme, pal, anchor, m);
    }
    if state.popups.completion.is_some() {
        let anchor_pos = state.popups.completion.as_ref().map(|c| c.anchor).unwrap_or(0);
        let anchor = crate::view::caret_anchor(ui, state, scheme, m, text_rect, anchor_pos);
        if let Some(i) = show_completion(ui, state, scheme, pal, anchor + vec2(-26.0, m.row_h + 2.0)) {
            accepted = state.popups.completion.as_ref().and_then(|c| c.filtered.get(i).map(|&k| c.items[k].data));
            accept(state, i, false);
        }
    }
    if let Some(h) = state.popups.hover.clone() {
        show_hover(ui, &h, scheme, pal, m);
    }
    accepted
}

fn popup_frame(pal: &Palette) -> egui::Frame {
    egui::Frame::NONE
        .fill(pal.popup_bg)
        .stroke(Stroke::new(1.0, pal.popup_border))
        .corner_radius(8)
        .shadow(egui::Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(if pal.dark { 90 } else { 30 }) })
}

fn show_completion(ui: &Ui, state: &mut EditorState, scheme: &Scheme, pal: &Palette, at: Pos2) -> Option<usize> {
    let c: &mut Completion = state.popups.completion.as_mut()?;
    let mut clicked = None;
    let rows = c.filtered.len().clamp(1, MAX_ROWS);
    let height = rows as f32 * ROW_H + 8.0;
    // Below the caret, or above when it would not fit.
    let screen = ui.ctx().content_rect();
    let pos = if at.y + height > screen.bottom() - 8.0 { pos2(at.x, at.y - height - 24.0) } else { at };
    let pos = pos2(pos.x.min(screen.right() - LIST_W - 8.0).max(screen.left() + 4.0), pos.y);
    egui::Area::new(Id::new(("completion", state.id))).order(Order::Foreground).fixed_pos(pos).show(ui.ctx(), |ui| {
        popup_frame(pal).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
            ui.set_width(LIST_W);
            if c.filtered.is_empty() {
                let text = if c.loading { rc_kit::i18n::tr("Loading…") } else { rc_kit::i18n::tr("No suggestions") };
                let (r, _) = ui.allocate_exact_size(vec2(LIST_W, ROW_H), Sense::hover());
                ui.painter().text(pos2(r.left() + 10.0, r.center().y), Align2::LEFT_CENTER, text, FontId::proportional(13.0), pal.muted);
                return;
            }
            let scroll = egui::ScrollArea::vertical().max_height(MAX_ROWS as f32 * ROW_H).auto_shrink([false, true]);
            scroll.show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (row, &i) in c.filtered.iter().enumerate() {
                    let item = &c.items[i];
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                    let selected = row == c.selected;
                    if selected {
                        ui.painter().rect_filled(r, 4.0, pal.selection);
                        if c.scroll_to_selected {
                            ui.scroll_to_rect(r, None);
                        }
                    } else if resp.hovered() {
                        ui.painter().rect_filled(r, 4.0, pal.row_hover);
                    }
                    icons::paint(ui.painter(), Rect::from_center_size(pos2(r.left() + 14.0, r.center().y), egui::Vec2::splat(16.0)), item.icon, pal, true);
                    // Label with the matched prefix in bold accent (IntelliJ).
                    let mut job = egui::text::LayoutJob::default();
                    let font = FontId::monospace(13.0);
                    let n = c.prefix.chars().count();
                    let prefix_matches = item.label.to_lowercase().starts_with(&c.prefix.to_lowercase());
                    let color = if item.deprecated { pal.muted } else { pal.text };
                    if prefix_matches && n > 0 {
                        let split = item.label.char_indices().nth(n).map_or(item.label.len(), |(b, _)| b);
                        job.append(&item.label[..split], 0.0, egui::TextFormat { font_id: rc_kit::theme::mono_bold(13.0), color: pal.accent, ..Default::default() });
                        job.append(&item.label[split..], 0.0, egui::TextFormat { font_id: font.clone(), color, ..Default::default() });
                    } else {
                        job.append(&item.label, 0.0, egui::TextFormat { font_id: font.clone(), color, strikethrough: if item.deprecated { Stroke::new(1.0, color) } else { Stroke::NONE }, ..Default::default() });
                    }
                    if !item.detail.is_empty() {
                        job.append(&item.detail, 6.0, egui::TextFormat { font_id: FontId::proportional(12.0), color: pal.muted, ..Default::default() });
                    }
                    let type_galley = (!item.type_text.is_empty()).then(|| ui.painter().layout_no_wrap(item.type_text.clone(), FontId::proportional(12.0), pal.muted));
                    let type_w = type_galley.as_ref().map_or(0.0, |g| g.size().x + 10.0);
                    job.wrap.max_width = r.width() - 32.0 - type_w;
                    job.wrap.max_rows = 1;
                    job.wrap.break_anywhere = true;
                    job.wrap.overflow_character = Some('…');
                    let g = ui.painter().layout_job(job);
                    ui.painter().galley(pos2(r.left() + 28.0, r.center().y - g.size().y / 2.0), g, pal.text);
                    if let Some(tg) = type_galley {
                        ui.painter().galley(pos2(r.right() - tg.size().x - 8.0, r.center().y - tg.size().y / 2.0), tg, pal.muted);
                    }
                    if resp.clicked() {
                        clicked = Some(row);
                    }
                    if resp.hovered() && ui.input(|i| i.pointer.is_moving()) {
                        // Hovering does not move the selection (IntelliJ), only clicks.
                    }
                }
            });
            c.scroll_to_selected = false;
        });
    });
    // Documentation of the selected item, to the right of the list.
    if let Some(doc) = c.doc.clone().filter(|d| !d.trim().is_empty()) {
        let doc_pos = pos2(pos.x + LIST_W + 14.0, pos.y);
        egui::Area::new(Id::new(("completion_doc", state.id))).order(Order::Foreground).fixed_pos(doc_pos).show(ui.ctx(), |ui| {
            popup_frame(pal).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                ui.set_max_width(380.0);
                render_doc(ui, &doc, scheme, pal);
            });
        });
    }
    let _ = scheme;
    clicked
}

fn show_signature(ui: &Ui, sig: &crate::popup::SignatureHelp, scheme: &Scheme, pal: &Palette, anchor: Pos2, m: &Metrics) {
    if sig.signatures.is_empty() {
        return;
    }
    let lines = sig.signatures.len().min(6) as f32;
    let pos = pos2(anchor.x - 8.0, anchor.y - lines * 22.0 - 14.0);
    egui::Area::new(Id::new("signature_help")).order(Order::Foreground).fixed_pos(pos).interactable(false).show(ui.ctx(), |ui| {
        popup_frame(pal).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
            for (i, s) in sig.signatures.iter().enumerate().take(6) {
                let active = i == sig.active;
                let mut job = egui::text::LayoutJob::default();
                let base = egui::TextFormat { font_id: FontId::monospace(12.5), color: if active { pal.text } else { pal.muted }, ..Default::default() };
                let bold = egui::TextFormat { font_id: rc_kit::theme::mono_bold(12.5), color: pal.text, ..Default::default() };
                let chars: Vec<char> = s.label.chars().collect();
                let hl = if active { s.active_param.and_then(|p| s.params.get(p)).copied() } else { None };
                match hl {
                    Some((a, b)) if a <= b && b <= chars.len() => {
                        job.append(&chars[..a].iter().collect::<String>(), 0.0, base.clone());
                        job.append(&chars[a..b].iter().collect::<String>(), 0.0, bold);
                        job.append(&chars[b..].iter().collect::<String>(), 0.0, base);
                    }
                    _ => job.append(&s.label, 0.0, base),
                }
                ui.label(job);
            }
        });
    });
    let _ = (scheme, m);
}

fn show_hover(ui: &Ui, h: &crate::popup::Hover, scheme: &Scheme, pal: &Palette, m: &Metrics) {
    let pos = h.screen + vec2(-4.0, m.row_h);
    let resp = egui::Area::new(Id::new("hover_popup")).order(Order::Foreground).fixed_pos(pos).show(ui.ctx(), |ui| {
        popup_frame(pal).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
            ui.set_max_width(560.0);
            if let Some((sev, msg)) = &h.problem {
                ui.horizontal(|ui| {
                    let icon = match sev {
                        Severity::Error => ico::ERROR,
                        Severity::Warning => ico::WARNING,
                        _ => ico::INFO,
                    };
                    icons::image(ui, icon, 16.0);
                    ui.add(egui::Label::new(egui::RichText::new(msg).color(pal.text)).wrap());
                });
                if !h.text.trim().is_empty() {
                    ui.add_space(4.0);
                    ui.separator();
                }
            }
            if !h.text.trim().is_empty() {
                render_doc(ui, &h.text, scheme, pal);
            }
        });
    });
    let hovered = resp.response.hovered() || resp.response.contains_pointer();
    ui.ctx().data_mut(|d| d.insert_temp(Id::new("dotcraft_hover_popup_hovered"), hovered));
}

/// Minimal Markdown for documentation: fenced code blocks in monospace, `inline code`,
/// paragraphs and horizontal rules.
pub fn render_doc(ui: &mut Ui, text: &str, scheme: &Scheme, pal: &Palette) {
    let mut in_code = false;
    let mut code = String::new();
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            if in_code {
                let g = egui::RichText::new(code.trim_end()).font(FontId::monospace(12.5)).color(scheme.text);
                ui.label(g);
                code.clear();
            }
            in_code = !in_code;
            continue;
        }
        if in_code {
            code.push_str(line);
            code.push('\n');
            continue;
        }
        let t = line.trim();
        if t == "---" || t == "***" {
            ui.separator();
            continue;
        }
        if t.is_empty() {
            ui.add_space(4.0);
            continue;
        }
        // `code` spans → monospace runs.
        let mut job = egui::text::LayoutJob::default();
        for (k, part) in t.split('`').enumerate() {
            let part = part.replace("**", "").replace("\\_", "_").replace("\\<", "<").replace("\\>", ">");
            let fmt = if k % 2 == 1 {
                egui::TextFormat { font_id: FontId::monospace(12.5), color: scheme.text, ..Default::default() }
            } else {
                egui::TextFormat { font_id: FontId::proportional(13.0), color: pal.text, ..Default::default() }
            };
            job.append(&part, 0.0, fmt);
        }
        job.wrap.max_width = ui.available_width();
        ui.label(job);
    }
    if in_code && !code.is_empty() {
        ui.label(egui::RichText::new(code.trim_end()).font(FontId::monospace(12.5)).color(scheme.text));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippets_expand_to_text_and_caret() {
        assert_eq!(expand_snippet("WriteLine($1)$0"), ("WriteLine()".into(), Some(10)));
        assert_eq!(expand_snippet("for (${1:int} i)"), ("for (int i)".into(), Some(5)));
        assert_eq!(expand_snippet("plain"), ("plain".into(), None));
    }
}
