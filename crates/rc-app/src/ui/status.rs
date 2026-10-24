//! Status bar (New UI): navigation path on the left; caret position, line ending, encoding,
//! indentation and read-only state on the right; transient messages in between.

use crate::app::DotCraftApp;
use egui::{FontId, Ui};
use rc_kit::i18n::{tr, trf};
use rc_kit::{ico, icons, theme};

pub fn show(app: &mut DotCraftApp, ui: &mut Ui) {
    let p = rc_kit::pal(ui);
    let now = ui.input(|i| i.time);
    for n in &mut app.notices {
        if n.at == 0.0 {
            n.at = now;
        }
    }
    // Messages stay 8 s (errors 15 s).
    app.notices.retain(|n| now - n.at < if n.error { 15.0 } else { 8.0 });
    if !app.notices.is_empty() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
    }
    ui.horizontal_centered(|ui| {
        theme::split_row(ui, |ui, side| match side {
            theme::Side::Left => {
                ui.add_space(12.0);
                ui.spacing_mut().item_spacing.x = 2.0;
                breadcrumbs(app, ui);
                if let Some(n) = app.notices.last() {
                    ui.add_space(16.0);
                    icons::image(ui, if n.error { ico::ERROR } else { ico::INFO }, 16.0);
                    ui.label(egui::RichText::new(&n.text).color(if n.error { p.error } else { p.text_muted }).size(12.5));
                }
            }
            theme::Side::Right => {
                ui.add_space(10.0);
                ui.spacing_mut().item_spacing.x = 0.0;
                let Some(doc) = app.docs.active_doc() else { return };
                let e = &doc.editor;
                let widget = |ui: &mut Ui, text: String, tip: &str| {
                    let r = theme::flat_button(ui, None, &text, FontId::proportional(12.5), false, true);
                    r.on_hover_text(tip)
                };
                if e.read_only {
                    icons::image(ui, ico::INFO, 16.0).on_hover_text(tr("The file is read-only"));
                    ui.add_space(4.0);
                }
                let indent = if e.indent.unit == "\t" { tr("Tab").to_string() } else { trf("{n} spaces", &[("n", &e.indent.unit.len())]) };
                widget(ui, indent, tr("Indentation"));
                widget(ui, if doc.bom { "UTF-8 BOM".into() } else { "UTF-8".into() }, tr("File encoding"));
                widget(ui, e.buffer.line_ending().label().into(), tr("Line separator"));
                let (line, col) = e.caret_line_col();
                let sel: usize = e.selection.ranges().iter().map(|r| r.len()).sum();
                let pos = if sel > 0 {
                    trf("{line}:{col} ({n} chars)", &[("line", &(line + 1)), ("col", &(col + 1)), ("n", &sel)])
                } else {
                    format!("{}:{}", line + 1, col + 1)
                };
                if widget(ui, pos, tr("Go to Line…")).clicked() {
                    app.pending.push(rc_editor::Action::GotoLine);
                }
            }
        });
    });
}

/// Workspace › folders › file (each part clickable to locate it in the explorer).
fn breadcrumbs(app: &mut DotCraftApp, ui: &mut Ui) {
    let p = rc_kit::pal(ui);
    let Some(w) = &app.workspace else { return };
    let mut parts: Vec<String> = vec![w.ws.name()];
    if let Some(doc) = app.docs.active_doc()
        && let Ok(rel) = doc.path.strip_prefix(&w.ws.root)
    {
        parts.extend(rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()));
    }
    let n = parts.len();
    for (i, part) in parts.into_iter().enumerate() {
        let color = if i + 1 == n { p.text } else { p.text_muted };
        let resp = ui.add(egui::Label::new(egui::RichText::new(part).color(color).size(12.5)).sense(egui::Sense::click()));
        if resp.clicked() {
            app.locate_active();
        }
        if i + 1 < n {
            icons::image(ui, ico::CHEVRON_RIGHT, 16.0);
        }
    }
}
