//! Window layout (New UI with Islands): main toolbar on top, tool window stripes on the
//! sides, tool windows and the editor as rounded islands, status bar at the bottom.

pub mod dialogs;
pub mod editor_area;
pub mod explorer;
pub mod header;
pub mod status;
pub mod run_windows;
pub mod search_popup;
pub mod stripes;
pub mod toolwindows;
pub mod welcome;

use crate::app::DotCraftApp;
use crate::toolwindow::Side;
use egui::{Margin, Ui};
use rc_editor::Action;
use rc_kit::theme;

pub const TITLE_H: f32 = 40.0;
pub const STRIPE_W: f32 = 40.0;
pub const STATUS_H: f32 = 30.0;

pub fn frame(app: &mut DotCraftApp, ui: &mut Ui, editor_actions: Vec<Action>) {
    let ctx = ui.ctx().clone();
    let p = rc_kit::pal(ui);
    let bar = egui::Frame::NONE.fill(p.main_bg);
    let title_h = TITLE_H.max(32.0 / ctx.zoom_factor().max(0.1));
    egui::Panel::top("title_bar").exact_size(title_h).resizable(false).show_separator_line(false).frame(bar).show(ui, |ui| header::show(app, ui));
    egui::Panel::bottom("status_bar").exact_size(STATUS_H).resizable(false).show_separator_line(false).frame(bar).show(ui, |ui| status::show(app, ui));
    if app.workspace.is_none() {
        egui::CentralPanel::default().frame(bar.inner_margin(Margin { left: 6, right: 6, top: 0, bottom: 0 })).show(ui, |ui| welcome::show(app, ui));
        dialogs::show(app, &ctx);
        if cfg!(target_os = "windows") {
            rc_kit::window_chrome::resize_edges(&ctx);
        }
        return;
    }
    egui::Panel::left("stripe_left").exact_size(STRIPE_W).resizable(false).show_separator_line(false).frame(bar).show(ui, |ui| stripes::show(app, ui, Side::Left));
    egui::Panel::right("stripe_right").exact_size(STRIPE_W).resizable(false).show_separator_line(false).frame(bar).show(ui, |ui| stripes::show(app, ui, Side::Right));

    let gap = theme::ISLAND_GAP;
    if let Some(tw) = app.dock.left {
        let resp = egui::Panel::left("tw_left")
            .resizable(true)
            .default_size(app.dock.size(tw))
            .size_range(180.0..=800.0)
            .show_separator_line(false)
            .frame(bar.inner_margin(Margin { right: gap, ..Margin::ZERO }))
            .show(ui, |ui| theme::fixed_island(ui, &p, |ui| toolwindows::show(app, ui, tw)));
        remember_size(app, tw, resp.response.rect.width() - gap as f32);
    }
    if let Some(tw) = app.dock.right {
        let resp = egui::Panel::right("tw_right")
            .resizable(true)
            .default_size(app.dock.size(tw))
            .size_range(200.0..=800.0)
            .show_separator_line(false)
            .frame(bar.inner_margin(Margin { left: gap, ..Margin::ZERO }))
            .show(ui, |ui| theme::fixed_island(ui, &p, |ui| toolwindows::show(app, ui, tw)));
        remember_size(app, tw, resp.response.rect.width() - gap as f32);
    }
    if let Some(tw) = app.dock.bottom {
        let left = if app.dock.left.is_some() { gap } else { 0 };
        let right = if app.dock.right.is_some() { gap } else { 0 };
        let _ = (left, right);
        let resp = egui::Panel::bottom("tw_bottom")
            .resizable(true)
            .default_size(app.dock.size(tw))
            .size_range(100.0..=900.0)
            .show_separator_line(false)
            .frame(bar.inner_margin(Margin { top: gap, left: if app.dock.left.is_some() { gap } else { 0 }, right: if app.dock.right.is_some() { gap } else { 0 }, bottom: 0 }))
            .show(ui, |ui| theme::fixed_island(ui, &p, |ui| toolwindows::show(app, ui, tw)));
        remember_size(app, tw, resp.response.rect.height() - gap as f32);
    }
    let margin = Margin {
        left: if app.dock.left.is_some() { gap } else { 0 },
        right: if app.dock.right.is_some() { gap } else { 0 },
        top: 0,
        bottom: if app.dock.bottom.is_some() { gap } else { 0 },
    };
    egui::CentralPanel::default().frame(bar.inner_margin(margin)).show(ui, |ui| {
        theme::fixed_island(ui, &p, |ui| editor_area::show(app, ui, editor_actions));
    });
    dialogs::show(app, &ctx);
    search_popup::show(app, &ctx);
    if cfg!(target_os = "windows") {
        rc_kit::window_chrome::resize_edges(&ctx);
    }
}

fn remember_size(app: &mut DotCraftApp, tw: crate::toolwindow::ToolWindow, size: f32) {
    let size = size.round();
    if (app.dock.size(tw) - size).abs() >= 1.0 {
        app.dock.sizes.insert(tw, size);
        app.session_dirty = true;
    }
}
