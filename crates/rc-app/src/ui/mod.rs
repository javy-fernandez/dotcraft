//! Window layout (New UI with Islands): main toolbar on top, tool window stripes on the
//! sides, tool windows and the editor as rounded islands, status bar at the bottom.

pub mod debug_window;
pub mod database_window;
pub mod dialogs;
pub mod sdk_selection;
pub mod launch_profile_editor;
pub mod editor_area;
pub mod ef_core;
pub mod explorer;
pub mod git;
pub mod main_menu;
pub mod markdown;
pub mod merge;
pub mod monitoring;
pub mod new_project;
pub mod rebase;
pub mod nuget;
pub mod pull_requests;
pub mod header;
pub mod kit;
pub mod status;
pub mod run_windows;
pub mod search_popup;
pub mod stripes;
pub mod toolwindows;
pub mod welcome;
pub mod work_item_links;

use crate::app::{DotCraftApp, FocusMode};
use crate::toolwindow::Side;
use egui::{Margin, Ui};
use rc_editor::Action;
use rc_kit::theme;

pub const TITLE_H: f32 = 40.0;
pub const STRIPE_W: f32 = 40.0;
pub const STATUS_H: f32 = 30.0;

pub fn frame(app: &mut DotCraftApp, ui: &mut Ui, editor_actions: Vec<Action>) {
    let ctx = ui.ctx().clone();
    status::tick_notices(app, &ctx);
    let p = rc_kit::pal(ui);
    let bar = egui::Frame::NONE.fill(p.main_bg);
    let title_h = TITLE_H.max(32.0 / ctx.zoom_factor().max(0.1));
    // The focus modes need an editor: the welcome screen leaves them.
    if app.workspace.is_none() && app.docs.list.is_empty() {
        app.set_focus_mode(&ctx, FocusMode::Normal);
    }
    if !app.focus_mode.hides_chrome() {
        egui::Panel::top("title_bar").exact_size(title_h).resizable(false).show_separator_line(false).frame(bar).show(ui, |ui| header::show(app, ui));
        egui::Panel::bottom("status_bar").exact_size(STATUS_H).resizable(false).show_separator_line(false).frame(bar).show(ui, |ui| status::show(app, ui));
    }
    if app.focus_mode != FocusMode::Normal {
        // Bottom right, over the editor's scroll bar gap: the way out without menus or Esc.
        let bottom = if app.focus_mode.hides_chrome() { 0.0 } else { STATUS_H };
        egui::Area::new(egui::Id::new("exit_focus_mode"))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-28.0, -(bottom + 16.0)))
            .order(egui::Order::Foreground)
            .show(&ctx, |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                if theme::secondary_button(ui, app.focus_mode.label(true)).clicked() {
                    app.set_focus_mode(&ctx, FocusMode::Normal);
                }
            });
    }
    // Without a workspace: the welcome screen, unless files are open on their own.
    if app.workspace.is_none() && app.docs.list.is_empty() {
        egui::CentralPanel::default().frame(bar.inner_margin(Margin { left: 6, right: 6, top: 0, bottom: 0 })).show(ui, |ui| welcome::show(app, ui));
        dialogs::show(app, &ctx);
        if let Some(mut d) = app.new_project.take()
            && new_project::show(app, &ctx, &mut d)
        {
            app.new_project = Some(d);
        }
        if cfg!(target_os = "windows") {
            rc_kit::window_chrome::resize_edges(&ctx);
        }
        return;
    }
    // Files on their own (no workspace): only the editor, like Rider's LightEdit.
    let light = app.workspace.is_none();
    let margin = if app.focus_mode.hides_chrome() {
        Margin::same(6)
    } else if light || app.focus_mode != FocusMode::Normal {
        Margin { left: 6, right: 6, top: 0, bottom: 0 }
    } else {
        tool_windows(app, ui, &p, bar)
    };
    egui::CentralPanel::default().frame(bar.inner_margin(margin)).show(ui, |ui| {
        theme::fixed_island(ui, &p, |ui| editor_area::show(app, ui, editor_actions));
    });
    dialogs::show(app, &ctx);
    nuget::sign_in_dialog(app, &ctx);
    search_popup::show(app, &ctx);
    editor_area::floating_windows(app, &ctx);
    if let Some(mut d) = app.new_project.take()
        && new_project::show(app, &ctx, &mut d)
    {
        app.new_project = Some(d);
    }
    if let Some(mut d) = app.ef.take()
        && ef_core::show(app, &ctx, &mut d)
        && app.ef.is_none()
    {
        app.ef = Some(d);
    }
    if cfg!(target_os = "windows") {
        rc_kit::window_chrome::resize_edges(&ctx);
    }
}

/// Stripes and open tool windows; returns the editor island's margin.
fn tool_windows(app: &mut DotCraftApp, ui: &mut Ui, p: &rc_kit::theme::Palette, bar: egui::Frame) -> Margin {
    let p = *p;
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
    Margin {
        left: if app.dock.left.is_some() { gap } else { 0 },
        right: if app.dock.right.is_some() { gap } else { 0 },
        top: 0,
        bottom: if app.dock.bottom.is_some() { gap } else { 0 },
    }
}

fn remember_size(app: &mut DotCraftApp, tw: crate::toolwindow::ToolWindow, size: f32) {
    let size = size.round();
    if (app.dock.size(tw) - size).abs() >= 1.0 {
        app.dock.sizes.insert(tw, size);
        app.session_dirty = true;
    }
}

#[cfg(test)]
mod focus_mode_tests {
    use super::*;
    use rc_kit::i18n::tr;
    use egui_kittest::kittest::Queryable;

    #[test]
    fn focus_modes_restore_the_existing_dock_layout() {
        let _home = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        let root = crate::screens::fixtures().join("Shop/Shop.sln");
        let mut app = crate::screens::test_app(home.path(), true);
        app.workspace = Some(crate::app::WorkspaceState { ws: rc_core::Workspace::open(&root).unwrap(), solution: Some(rc_dotnet::Solution::load(&root).unwrap()) });
        app.dock.show(crate::toolwindow::ToolWindow::Explorer);
        let mut harness = crate::screens::harness(app, egui::vec2(900.0, 600.0));
        let original = harness.state().dock.clone();

        harness.state_mut().focus_mode = FocusMode::DistractionFree;
        harness.run_steps(3);
        assert_eq!(harness.state().dock, original);
        harness.state_mut().focus_mode = FocusMode::Zen;
        harness.state_mut().notices.push(crate::app::Notice { text: "Expired".into(), error: false, at: 1.0 });
        harness.input_mut().time = Some(20.0);
        harness.run_steps(3);
        assert_eq!(harness.state().dock, original);
        assert!(harness.state().notices.is_empty());
        harness.get_by_label(tr("Exit Zen Mode")).click();
        harness.run_steps(2);
        assert_eq!(harness.state().focus_mode, FocusMode::Normal);
        assert_eq!(harness.state().dock, original);
    }

    #[test]
    fn view_menu_enters_each_focus_mode_and_the_exit_button_leaves_it() {
        let _home = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        let path = crate::screens::fixtures().join("Shop/Shop.sln");
        let mut app = crate::screens::test_app(home.path(), true);
        app.workspace = Some(crate::app::WorkspaceState { ws: rc_core::Workspace::open(&path).unwrap(), solution: Some(rc_dotnet::Solution::load(&path).unwrap()) });
        let mut harness = crate::screens::harness(app, egui::vec2(900.0, 600.0));
        let font = harness.state().editor_config.font.size;
        for mode in FocusMode::ALL {
            harness.get_by_label(tr("Main Menu")).click();
            harness.run_steps(3);
            harness.get_by_label(tr("View")).click();
            harness.run_steps(3);
            harness.get_by_label(tr("Appearance")).click();
            harness.run_steps(3);
            harness.get_by_label(mode.label(false)).click();
            harness.run_steps(3);
            assert_eq!(harness.state().focus_mode, mode);
            // Presentation and Zen hide the title bar (and with it the Main Menu button).
            assert_eq!(harness.query_by_label(tr("Main Menu")).is_none(), mode.hides_chrome(), "{mode:?}");
            assert_eq!(harness.state().editor_config.font.size > font, mode == FocusMode::Presentation, "{mode:?}");
            harness.get_by_label(mode.label(true)).click();
            harness.run_steps(3);
            assert_eq!(harness.state().focus_mode, FocusMode::Normal);
            assert_eq!(harness.state().editor_config.font.size, font);
        }
    }

    #[test]
    fn focus_modes_are_exclusive_actions_and_esc_leaves_them() {
        let _home = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        let path = crate::screens::fixtures().join("Shop/Shop.sln");
        let mut app = crate::screens::test_app(home.path(), true);
        app.workspace = Some(crate::app::WorkspaceState { ws: rc_core::Workspace::open(&path).unwrap(), solution: Some(rc_dotnet::Solution::load(&path).unwrap()) });
        let mut harness = crate::screens::harness(app, egui::vec2(900.0, 600.0));
        let ctx = harness.ctx.clone();
        let font = harness.state().editor_config.font.size;
        // Every mode is a Find Action entry.
        for mode in FocusMode::ALL {
            assert!(crate::search::action_items().iter().any(|(_, a)| Some(*a) == mode.action()), "{mode:?}");
        }
        harness.state_mut().run_ide_action(&ctx, Action::TogglePresentationMode);
        assert_eq!(harness.state().focus_mode, FocusMode::Presentation);
        harness.state_mut().run_ide_action(&ctx, Action::ToggleZenMode);
        assert_eq!(harness.state().focus_mode, FocusMode::Zen);
        assert_eq!(harness.state().editor_config.font.size, font, "leaving Presentation for Zen restores the font");
        harness.state_mut().run_ide_action(&ctx, Action::ToggleDistractionFreeMode);
        assert_eq!(harness.state().focus_mode, FocusMode::DistractionFree);
        harness.state_mut().run_ide_action(&ctx, Action::ToggleDistractionFreeMode);
        assert_eq!(harness.state().focus_mode, FocusMode::Normal);
        harness.state_mut().run_ide_action(&ctx, Action::TogglePresentationMode);
        harness.state_mut().run_ide_action(&ctx, Action::Escape);
        assert_eq!(harness.state().focus_mode, FocusMode::Normal);
        assert_eq!(harness.state().editor_config.font.size, font);
    }
}

pub mod editor_workflows;
