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
                insight_widget(app, ui);
                debugger_widget(app, ui);
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

/// netcoredbg download progress and the state of the debug session (click: Debug window).
fn debugger_widget(app: &mut DotCraftApp, ui: &mut Ui) {
    let (icon, text) = if let Some((done, total)) = app.debugger.downloading {
        let pct = total.map(|t| format!(" {}%", done * 100 / t.max(1))).unwrap_or_default();
        (ico::DEBUG, format!("{}{pct}", tr("Downloading netcoredbg…")))
    } else if app.debugger.active() {
        (if app.debugger.stopped() { ico::PAUSE } else { ico::DEBUG }, app.debugger.state_label())
    } else {
        return;
    };
    if theme::flat_button(ui, Some(icon), &text, FontId::proportional(12.5), false, true).on_hover_text(tr("Debug")).clicked() {
        app.dock.show(crate::toolwindow::ToolWindow::Debug);
    }
    ui.add_space(4.0);
}

/// C# language server state: download offer, progress, failures with a restart button.
fn insight_widget(app: &mut DotCraftApp, ui: &mut Ui) {
    use crate::insight::InsightStatus as S;
    let p = rc_kit::pal(ui);
    match app.insight.status.clone() {
        S::Idle => {}
        S::Ready => {
            let tip = match app.insight.last_completion_ms {
                Some(ms) => trf("C# language server ready (last completion {ms} ms)", &[("ms", &ms)]),
                None => tr("C# language server ready").to_string(),
            };
            theme::flat_button(ui, Some(ico::CHECKMARK), "C#", FontId::proportional(12.5), false, true).on_hover_text(tip);
        }
        S::NotInstalled => {
            if theme::flat_button(ui, Some(ico::DOWNLOAD), tr("Download C# language server"), FontId::proportional(12.5), false, true)
                .on_hover_text(tr("Roslyn is needed for completion, navigation, refactorings and errors (about 60 MB)"))
                .clicked()
            {
                app.insight.download();
            }
        }
        S::Downloading { done, total } => {
            let text = match total {
                Some(t) if t > 0 => trf("Downloading C# language server… {pct}%", &[("pct", &(done * 100 / t))]),
                _ => trf("Downloading C# language server… {mb} MB", &[("mb", &(done / 1_048_576))]),
            };
            ui.add(egui::Spinner::new().size(12.0).color(p.muted));
            ui.add_space(6.0);
            ui.label(egui::RichText::new(text).size(12.5).color(p.text_muted));
            ui.add_space(8.0);
        }
        S::Starting | S::Loading(_) => {
            let text = match &app.insight.status {
                S::Loading(t) => t.clone(),
                _ => tr("Starting C# language server…").to_string(),
            };
            ui.add(egui::Spinner::new().size(12.0).color(p.muted));
            ui.add_space(6.0);
            ui.label(egui::RichText::new(text).size(12.5).color(p.text_muted));
            ui.add_space(8.0);
        }
        S::Failed(why) => {
            if theme::flat_button(ui, Some(ico::ERROR), tr("C# server stopped — Restart"), FontId::proportional(12.5), false, true).on_hover_text(why).clicked() {
                app.insight.restart();
            }
        }
    }
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
