//! Building blocks for tool windows, so every window looks like the rest of DotCraft: a painted
//! header (title, status, tabs, icon actions on the right), flat toolbar buttons, painted list
//! rows instead of egui's default buttons, combo boxes and collapsing headers.
//! Every interactive piece reports its label to AccessKit, so kittest can find it by text.

use crate::ui::toolwindows::HEADER_H;
use egui::{Align2, FontId, Rect, Response, Sense, Ui, pos2, vec2};
use rc_kit::{Ico, icons, theme};

/// What was clicked in a [`header`]: a tab or an action, by index.
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub struct HeaderClick {
    pub tab: Option<usize>,
    pub action: Option<usize>,
}

/// The window header: bold title, optional tabs (icon, label, selected), a grey status and icon
/// actions (icon, tooltip, enabled) laid out from the right edge, the first one rightmost
/// (by convention Hide).
pub fn header(ui: &mut Ui, title: &str, tabs: &[(Ico, &str, bool)], status: &str, actions: &[(Ico, &str, bool)]) -> HeaderClick {
    let p = rc_kit::pal(ui);
    let r = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), HEADER_H));
    ui.allocate_rect(r, Sense::hover());
    let t = ui.painter().text(pos2(r.left() + 12.0, r.center().y), Align2::LEFT_CENTER, title, theme::bold(13.0), p.text);
    let mut click = HeaderClick::default();
    let mut x = t.right() + 14.0;
    for (i, (icon, label, selected)) in tabs.iter().enumerate() {
        let w = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(12.5), p.text).size().x + 32.0;
        let br = Rect::from_min_size(pos2(x, r.center().y - 12.0), vec2(w, 24.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(br));
        if theme::tool_button(&mut child, *icon, label, *selected).clicked() {
            click.tab = Some(i);
        }
        x += w + 6.0;
    }
    let right = r.right() - 8.0 - 26.0 * actions.len() as f32;
    if !status.is_empty() && right - x > 40.0 {
        let mut job = egui::text::LayoutJob::simple_singleline(status.to_string(), FontId::proportional(12.0), p.muted);
        job.wrap.max_width = right - x - 12.0;
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        job.wrap.overflow_character = Some('…');
        let g = ui.painter().layout_job(job);
        ui.painter().galley(pos2(x + 2.0, r.center().y - g.size().y / 2.0), g, p.muted);
    }
    for (i, (icon, tip, enabled)) in actions.iter().enumerate() {
        let br = Rect::from_min_size(pos2(r.right() - 8.0 - 26.0 * (i + 1) as f32, r.center().y - 13.0), vec2(26.0, 26.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(br));
        if theme::icon_button(&mut child, *icon, *enabled, tip).clicked() && *enabled {
            click.action = Some(i);
        }
    }
    click
}

/// A row of controls under the header, with the windows' side padding; wraps when narrow.
pub fn toolbar<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .inner_margin(egui::Margin { left: 10, right: 10, top: 2, bottom: 4 })
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            ui.horizontal_wrapped(|ui| {
                ui.set_min_height(26.0);
                add(ui)
            })
            .inner
        })
        .inner
}

/// Padded content area (forms, details): the windows' side padding.
pub fn content<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new().inner_margin(egui::Margin { left: 12, right: 12, top: 4, bottom: 8 }).show(ui, add).inner
}

/// Flat text button with an optional icon (the toolbar style).
pub fn button(ui: &mut Ui, icon: Option<Ico>, text: &str, enabled: bool) -> Response {
    let r = theme::flat_button(ui, icon, text, FontId::proportional(12.5), false, enabled);
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, text));
    r
}

/// Grey text for hints, notes and counts.
pub fn note(ui: &mut Ui, text: &str) {
    let p = rc_kit::pal(ui);
    ui.label(egui::RichText::new(text).size(12.0).color(p.muted));
}

/// A message line: red for errors, amber for warnings.
pub fn message(ui: &mut Ui, text: &str, error: bool) {
    let p = rc_kit::pal(ui);
    ui.label(egui::RichText::new(text).size(12.5).color(if error { p.error } else { p.warn }));
}

/// A labelled single-line field: grey caption above, the text field below.
pub fn field(ui: &mut Ui, label: &str, text: &mut String, hint: &str, width: f32, enabled: bool) -> Response {
    let p = rc_kit::pal(ui);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 3.0;
        let caption = ui.label(egui::RichText::new(label).size(12.0).color(p.muted));
        ui.add_enabled(enabled, egui::TextEdit::singleline(text).hint_text(hint).desired_width(width).margin(vec2(6.0, 4.0))).labelled_by(caption.id)
    })
    .inner
}

/// A search / filter box in a toolbar.
pub fn search(ui: &mut Ui, text: &mut String, hint: &str, width: f32) -> Response {
    ui.add(egui::TextEdit::singleline(text).hint_text(hint).desired_width(width).margin(vec2(6.0, 4.0)))
}

/// A flat drop-down (text with a chevron) listing `options`; returns the picked index.
pub fn dropdown(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, selected: &str, options: &[String], enabled: bool) -> Option<usize> {
    let resp = ui.push_id(id, |ui| theme::flat_button(ui, None, selected, FontId::proportional(12.5), true, enabled)).inner;
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, enabled, selected));
    let mut picked = None;
    egui::Popup::menu(&resp).show(|ui| {
        ui.set_min_width(160.0);
        for (i, option) in options.iter().enumerate() {
            if theme::menu_item(ui, None, option, option == selected).clicked() {
                picked = Some(i);
                ui.close();
            }
        }
    });
    picked
}

/// A section title row with a chevron; click toggles `open`. Returns whether it is open.
pub fn section(ui: &mut Ui, icon: Ico, title: &str, secondary: &str, open: &mut bool) -> bool {
    let p = rc_kit::pal(ui);
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(200.0), 26.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r.shrink2(vec2(6.0, 0.0)), 4.0, p.row_hover);
    }
    let chevron = if *open { rc_kit::ico::CHEVRON_DOWN } else { rc_kit::ico::CHEVRON_RIGHT };
    icons::paint_sized(ui.painter(), Rect::from_center_size(pos2(r.left() + 16.0, r.center().y), vec2(14.0, 14.0)), chevron, &p, true, 14.0);
    icons::paint(ui.painter(), Rect::from_center_size(pos2(r.left() + 34.0, r.center().y), vec2(16.0, 16.0)), icon, &p, true);
    let g = ui.painter().layout_no_wrap(title.to_string(), theme::bold(12.5), p.text);
    ui.painter().galley(pos2(r.left() + 48.0, (r.center().y - g.size().y / 2.0).round()), g.clone(), p.text);
    if !secondary.is_empty() {
        ui.painter().text(pos2(r.left() + 56.0 + g.size().x, r.center().y), Align2::LEFT_CENTER, secondary, FontId::proportional(12.0), p.muted);
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::CollapsingHeader, true, title));
    if resp.clicked() {
        *open = !*open;
    }
    *open
}

/// A clickable list row: icon, text, grey secondary text; `depth` indents it.
pub fn row(ui: &mut Ui, depth: usize, icon: Option<Ico>, text: &str, secondary: &str, selected: bool) -> Response {
    let p = rc_kit::pal(ui);
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(240.0), 24.0), Sense::click());
    if selected {
        ui.painter().rect_filled(r.shrink2(vec2(6.0, 0.0)), 4.0, p.selection);
    } else if resp.hovered() {
        ui.painter().rect_filled(r.shrink2(vec2(6.0, 0.0)), 4.0, p.row_hover);
    }
    let mut x = r.left() + 12.0 + depth as f32 * 18.0;
    if let Some(icon) = icon {
        icons::paint(ui.painter(), Rect::from_center_size(pos2(x + 8.0, r.center().y), vec2(16.0, 16.0)), icon, &p, true);
        x += 24.0;
    }
    let g = ui.painter().layout_no_wrap(text.to_string(), FontId::proportional(13.0), p.text);
    ui.painter().galley(pos2(x, (r.center().y - g.size().y / 2.0).round()), g.clone(), p.text);
    if !secondary.is_empty() {
        ui.painter().text(pos2(x + 8.0 + g.size().x, r.center().y), Align2::LEFT_CENTER, secondary, FontId::proportional(12.0), p.muted);
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, text));
    resp
}
