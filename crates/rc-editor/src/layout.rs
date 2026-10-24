//! Laying out one line for painting: tabs expanded, inlay hints and fold placeholders
//! inserted, colours applied. Keeps the map between buffer columns and the characters of the
//! galley so carets, selections and hit-testing line up exactly.

use egui::text::{CCursor, LayoutJob, LayoutSection, TextFormat};
use egui::{Color32, FontFamily, FontId, Galley};
use rc_kit::Scheme;
use rc_syntax::{Hl, Span};
use std::sync::Arc;

/// Font settings of the editor.
#[derive(Clone, Debug, PartialEq)]
pub struct EditorFont {
    pub size: f32,
    /// Multiplier of the font's natural height (IntelliJ's "line height", 1.2 by default).
    pub line_spacing: f32,
}

impl Default for EditorFont {
    fn default() -> Self {
        EditorFont { size: 13.0, line_spacing: 1.2 }
    }
}

/// Something shown inside a line that is not buffer text.
#[derive(Clone, Debug)]
pub struct Virtual {
    /// Buffer column it is shown before.
    pub col: usize,
    pub text: String,
    pub kind: VirtualKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VirtualKind {
    InlayHint,
    /// `{...}` of a collapsed fold: replaces columns `col..` of the line.
    FoldPlaceholder,
}

pub struct LineLayout {
    pub galley: Arc<Galley>,
    /// Galley char index of each buffer column (`len + 1` entries).
    col_to_char: Vec<usize>,
    /// Buffer columns of the line (without the fold suffix).
    pub len: usize,
    /// Galley char ranges of virtual text (hints, placeholders), for painting boxes.
    pub virtuals: Vec<(std::ops::Range<usize>, VirtualKind)>,
    /// When the line ends in a collapsed fold: buffer columns of the end line shown after the
    /// placeholder start at this galley char.
    pub suffix_start: Option<usize>,
}

impl LineLayout {
    pub fn char_of_col(&self, col: usize) -> usize {
        self.col_to_char.get(col).copied().unwrap_or_else(|| *self.col_to_char.last().unwrap_or(&0))
    }

    /// X offset of buffer column `col` from the line's left edge.
    pub fn x_of_col(&self, col: usize) -> f32 {
        self.galley.pos_from_cursor(CCursor::new(self.char_of_col(col))).min.x
    }

    /// Buffer column nearest to x (virtual text snaps to its edges).
    pub fn col_at_x(&self, x: f32) -> usize {
        let c = self.galley.cursor_from_pos(egui::vec2(x, self.galley.rect.center().y)).index.0;
        match self.col_to_char.binary_search(&c) {
            Ok(col) => col,
            Err(col) => {
                // Inside virtual text: pick the nearer edge.
                let col = col.min(self.len);
                if col == 0 {
                    return 0;
                }
                let (a, b) = (self.col_to_char[col - 1], self.col_to_char[col]);
                if c - a <= b - c { col - 1 } else { col }
            }
        }
    }

    /// Whether x falls on the fold placeholder.
    pub fn placeholder_at_x(&self, x: f32) -> bool {
        self.virtuals.iter().any(|(r, k)| {
            *k == VirtualKind::FoldPlaceholder && {
                let x0 = self.galley.pos_from_cursor(CCursor::new(r.start)).min.x;
                let x1 = self.galley.pos_from_cursor(CCursor::new(r.end)).min.x;
                x >= x0 && x < x1
            }
        })
    }

    pub fn width(&self) -> f32 {
        self.galley.rect.width()
    }
}

fn br(a: usize, b: usize) -> std::ops::Range<egui::text::ByteIndex> {
    egui::text::ByteIndex(a)..egui::text::ByteIndex(b)
}

pub fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

fn mono_italic(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(rc_kit::theme::FONT_MONO_ITALIC.into()))
}

fn mono_bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(rc_kit::theme::FONT_MONO_BOLD.into()))
}

/// Text format of a colour role in the Rider schemes.
pub fn format_for(hl: Hl, s: &Scheme, size: f32) -> TextFormat {
    let (color, font) = match hl {
        Hl::Plain | Hl::Local | Hl::Parameter => (s.identifier, mono(size)),
        Hl::Keyword | Hl::Preproc => (s.keyword, mono(size)),
        Hl::String => (s.string, mono(size)),
        Hl::Escape => (s.escape, mono(size)),
        Hl::Number => (s.number, mono(size)),
        Hl::Comment | Hl::DocComment => (s.comment, mono_italic(size)),
        Hl::Class | Hl::Interface | Hl::TypeParam | Hl::Namespace | Hl::Attribute => (s.class, mono(size)),
        Hl::Struct | Hl::Enum | Hl::Delegate => (s.struct_, mono(size)),
        Hl::Method => (s.method, mono(size)),
        Hl::Field | Hl::Property | Hl::EnumMember => (s.field, mono(size)),
        Hl::Constant => (s.field, mono_bold(size)),
        Hl::Event => (s.event, mono(size)),
        Hl::Operator | Hl::Punct => (s.punct, mono(size)),
        Hl::Tag => (s.class, mono(size)),
        Hl::XmlAttr => (s.xml_attr, mono(size)),
        Hl::Entity => (s.entity, mono(size)),
        Hl::Heading => (s.keyword, mono_bold(size)),
        Hl::Code => (s.string, mono(size)),
        Hl::Link => (s.hyperlink, mono(size)),
        Hl::Dimmed => (s.dimmed, mono(size)),
    };
    TextFormat { font_id: font, color, ..Default::default() }
}

pub struct LineInput<'a> {
    /// The line's text without the line break.
    pub text: &'a str,
    pub spans: &'a [Span],
    /// Column ranges drawn dimmed (unused code).
    pub dimmed: &'a [(usize, usize)],
    pub virtuals: &'a [Virtual],
    /// Text shown after a fold placeholder (the rest of the fold's end line), with its spans.
    pub suffix: Option<(&'a str, &'a [Span])>,
    pub tab_width: usize,
}

/// Builds the layout job of a line and lays it out with `layout`.
pub fn layout_line(input: &LineInput, scheme: &Scheme, font: &EditorFont, layout: impl FnOnce(LayoutJob) -> Arc<Galley>) -> LineLayout {
    let size = font.size;
    let mut job = LayoutJob { break_on_newline: false, ..Default::default() };
    job.wrap.max_width = f32::INFINITY;
    let mut col_to_char = Vec::with_capacity(input.text.len() + 1);
    let mut virtuals = Vec::new();
    let mut nchars = 0usize;
    let mut visual = 0usize;
    let hint_fmt = TextFormat {
        font_id: mono(size - 1.0),
        color: scheme.hint_fg,
        background: scheme.hint_bg,
        valign: egui::Align::Center,
        ..Default::default()
    };
    let fold_fmt = TextFormat { font_id: mono(size), color: scheme.folded_fg, ..Default::default() };

    // Colour role of each column: spans first, dimmed ranges override.
    let role_at = |spans: &[Span], col: usize| -> Hl {
        let i = spans.partition_point(|s| s.end <= col);
        spans.get(i).filter(|s| s.start <= col).map_or(Hl::Plain, |s| s.hl)
    };
    let mut current: Option<(Hl, usize)> = None; // (role, byte start of section)
    let push_section = |job: &mut LayoutJob, hl: Hl, start: usize| {
        let end = job.text.len();
        if end > start {
            job.sections.push(LayoutSection { leading_space: 0.0, byte_range: br(start, end), format: format_for(hl, scheme, size) });
        }
    };
    let mut vi = input.virtuals.iter().peekable();
    let mut cut_at: Option<usize> = None;
    for (col, ch) in input.text.chars().enumerate() {
        // Virtual text before this column.
        while let Some(v) = vi.peek().filter(|v| v.col == col) {
            if let Some((hl, start)) = current.take() {
                push_section(&mut job, hl, start);
            }
            let start_char = nchars;
            let start_byte = job.text.len();
            let (text, fmt) = match v.kind {
                VirtualKind::InlayHint => (format!("\u{2009}{}\u{2009}", v.text), hint_fmt.clone()),
                VirtualKind::FoldPlaceholder => (v.text.clone(), fold_fmt.clone()),
            };
            job.text.push_str(&text);
            nchars += text.chars().count();
            visual += text.chars().count();
            job.sections.push(LayoutSection { leading_space: 0.0, byte_range: br(start_byte, job.text.len()), format: fmt });
            virtuals.push((start_char..nchars, v.kind));
            if v.kind == VirtualKind::FoldPlaceholder {
                cut_at = Some(col);
            }
            vi.next();
        }
        if cut_at.is_some() {
            break;
        }
        col_to_char.push(nchars);
        let dim = input.dimmed.iter().any(|&(a, b)| a <= col && col < b);
        let hl = if dim { Hl::Dimmed } else { role_at(input.spans, col) };
        match current {
            Some((h, _)) if h == hl => {}
            _ => {
                if let Some((h, start)) = current.take() {
                    push_section(&mut job, h, start);
                }
                current = Some((hl, job.text.len()));
            }
        }
        if ch == '\t' {
            let n = input.tab_width - visual % input.tab_width.max(1);
            for _ in 0..n {
                job.text.push(' ');
            }
            nchars += n;
            visual += n;
        } else {
            job.text.push(ch);
            nchars += 1;
            visual += 1;
        }
    }
    if let Some((hl, start)) = current.take() {
        push_section(&mut job, hl, start);
    }
    let len = cut_at.unwrap_or_else(|| input.text.chars().count());
    // Virtual text at the end of the line (hints after the last char, a placeholder at EOL).
    for v in vi {
        if cut_at.is_some() {
            break;
        }
        if v.col < len {
            continue;
        }
        let start_char = nchars;
        let start_byte = job.text.len();
        let (text, fmt) = match v.kind {
            VirtualKind::InlayHint => (format!("\u{2009}{}\u{2009}", v.text), hint_fmt.clone()),
            VirtualKind::FoldPlaceholder => (v.text.clone(), fold_fmt.clone()),
        };
        if v.kind == VirtualKind::FoldPlaceholder {
            cut_at = Some(len);
        }
        col_to_char.truncate(len);
        job.text.push_str(&text);
        nchars += text.chars().count();
        job.sections.push(LayoutSection { leading_space: 0.0, byte_range: br(start_byte, job.text.len()), format: fmt });
        virtuals.push((start_char..nchars, v.kind));
    }
    // The end-of-line column maps to just before trailing virtual text… unless the line was
    // cut by a placeholder, where the caret goes before the placeholder.
    let eol_char = match cut_at {
        Some(_) => virtuals.iter().find(|(_, k)| *k == VirtualKind::FoldPlaceholder).map_or(nchars, |(r, _)| r.start),
        None => virtuals.iter().filter(|(r, _)| r.start >= *col_to_char.last().unwrap_or(&0)).map(|(r, _)| r.start).next().unwrap_or(nchars),
    };
    col_to_char.truncate(len);
    col_to_char.push(eol_char.max(*col_to_char.last().unwrap_or(&0)));

    let mut suffix_start = None;
    if let (Some(_), Some((suffix, spans))) = (cut_at, input.suffix) {
        suffix_start = Some(nchars);
        let mut current: Option<(Hl, usize)> = None;
        for (col, ch) in suffix.chars().enumerate() {
            let hl = role_at(spans, col);
            match current {
                Some((h, _)) if h == hl => {}
                _ => {
                    if let Some((h, start)) = current.take() {
                        push_section(&mut job, h, start);
                    }
                    current = Some((hl, job.text.len()));
                }
            }
            job.text.push(if ch == '\t' { ' ' } else { ch });
        }
        if let Some((hl, start)) = current.take() {
            push_section(&mut job, hl, start);
        }
    }
    if job.text.is_empty() {
        // An empty galley still needs the font's row height for the caret.
        job.sections.push(LayoutSection { leading_space: 0.0, byte_range: br(0, 0), format: format_for(Hl::Plain, scheme, size) });
    }
    let galley = layout(job);
    LineLayout { galley, col_to_char, len, virtuals, suffix_start }
}

/// Placeholder of a collapsed fold.
pub fn fold_placeholder(kind: rc_syntax::FoldKind, line_text: &str, start_col: usize) -> String {
    use rc_syntax::FoldKind::*;
    match kind {
        Block => {
            let first: String = line_text.chars().skip(start_col).take(1).collect();
            match first.as_str() {
                "{" => "{...}".into(),
                "[" => "[...]".into(),
                "<" => "<.../>".into(),
                _ => "...".into(),
            }
        }
        Comment => {
            let rest: String = line_text.chars().skip(start_col).collect();
            if rest.starts_with("///") {
                // Doc comments show their summary, like Rider.
                let summary = rest.trim_start_matches('/').trim().trim_start_matches("<summary>").trim();
                if summary.is_empty() { "/// ...".into() } else { format!("/// {summary} ...") }
            } else if rest.starts_with("/*") {
                "/*...*/".into()
            } else {
                "// ...".into()
            }
        }
        Imports => "...".into(),
        Region => line_text.chars().skip(start_col).collect::<String>().trim_start_matches("#region").trim().to_string(),
    }
}

/// Colour of a severity's underline.
pub fn severity_color(sev: crate::state::Severity, s: &Scheme) -> Color32 {
    use crate::state::Severity::*;
    match sev {
        Error => s.error,
        Warning => s.warning,
        Info | Hint => s.info,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lay(input: &LineInput) -> LineLayout {
        let ctx = egui::Context::default();
        rc_kit::theme::install_fonts(&ctx);
        let mut out = None;
        ctx.run_ui(Default::default(), |ui| {
            out = Some(layout_line(input, &rc_kit::theme::SCHEME_DARK, &EditorFont::default(), |job| ui.painter().layout_job(job)));
        })
        .textures_delta
        .clear();
        out.unwrap()
    }

    #[test]
    fn tabs_and_hints_keep_columns_aligned() {
        let virtuals = [Virtual { col: 4, text: "name:".into(), kind: VirtualKind::InlayHint }];
        let l = lay(&LineInput { text: "\tf(x);", spans: &[], dimmed: &[], virtuals: &virtuals, suffix: None, tab_width: 4 });
        assert_eq!(l.char_of_col(0), 0);
        assert_eq!(l.char_of_col(1), 4); // after the tab
        assert_eq!(l.char_of_col(3), 6);
        // The hint sits before column 4 ("x"), so "x" moves right by the hint's chars.
        assert_eq!(l.char_of_col(4), 7 + "\u{2009}name:\u{2009}".chars().count());
        assert_eq!(l.char_of_col(6), l.galley.text().chars().count());
        // Hit-testing inside the hint snaps to one of its edges.
        let mid = (l.x_of_col(3) + l.x_of_col(4)) / 2.0;
        assert!(matches!(l.col_at_x(mid), 3 | 4));
    }

    #[test]
    fn fold_placeholder_cuts_the_line_and_shows_the_suffix() {
        let virtuals = [Virtual { col: 9, text: "{...}".into(), kind: VirtualKind::FoldPlaceholder }];
        let l = lay(&LineInput { text: "void M() {", spans: &[], dimmed: &[], virtuals: &virtuals, suffix: Some((");", &[])), tab_width: 4 });
        assert_eq!(l.galley.text(), "void M() {...});");
        assert_eq!(l.len, 9);
        assert!(l.placeholder_at_x(l.x_of_col(9) + 3.0));
    }
}
