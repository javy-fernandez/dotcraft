//! Tool window stripes: one icon button per tool window, the open one highlighted. Left
//! stripe: left tool windows on top and bottom tool windows below (New UI).

use crate::app::DotCraftApp;
use crate::toolwindow::{Side, ToolWindow};
use egui::{Rect, Sense, Ui, pos2, vec2};
use rc_editor::Action;
use rc_kit::i18n::tr;
use rc_kit::{ico, icons, theme};

const BUTTON: f32 = 30.0;
const GAP: f32 = 6.0;

fn shortcut_for(tw: ToolWindow) -> Option<Action> {
    match tw {
        ToolWindow::Explorer => Some(Action::ToggleProjectWindow),
        ToolWindow::Terminal => Some(Action::ToggleTerminal),
        ToolWindow::Git => Some(Action::ToggleGitWindow),
        ToolWindow::Problems => Some(Action::ToggleProblems),
        _ => None,
    }
}

fn button(app: &mut DotCraftApp, ui: &mut Ui, tw: ToolWindow, at: egui::Pos2) {
    let p = rc_kit::pal(ui);
    let r = Rect::from_min_size(at, vec2(BUTTON, BUTTON));
    let resp = ui.interact(r, ui.id().with(tw.id()), Sense::click());
    let open = app.dock.is_open(tw);
    if open {
        ui.painter().rect_filled(r, 6.0, p.stripe_selected);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, 6.0, p.hover);
    }
    icons::paint(ui.painter(), r, tw.icon(), &p, true);
    let tip = match shortcut_for(tw).and_then(|a| app.keymap.shortcut_text(a)) {
        Some(s) => format!("{} ({s})", tw.title()),
        None => tw.title().to_string(),
    };
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, open, tw.title()));
    let resp = resp.on_hover_text(tip);
    if resp.clicked() {
        app.toggle_tool(tw);
    }
    // Right-click: move the tool window to another side (Rider's "Move To").
    let current = app.dock.side_of(tw);
    let mut moved = None;
    resp.context_menu(|ui| {
        ui.label(egui::RichText::new(tr("Move To")).size(12.0).color(rc_kit::pal(ui).muted));
        for side in Side::ALL {
            if theme::menu_entry(ui, None, side.title(), None, false, side != current).clicked() {
                moved = Some(side);
                ui.close();
            }
        }
        ui.separator();
        if theme::menu_item(ui, Some(if app.dock.is_open(tw) { ico::HIDE } else { ico::SHOW }), if app.dock.is_open(tw) { tr("Hide") } else { tr("Show") }, false).clicked() {
            app.toggle_tool(tw);
            ui.close();
        }
    });
    if let Some(side) = moved {
        app.dock.move_to(tw, side);
        app.session_dirty = true;
    }
}

fn overflow(app: &mut DotCraftApp, ui: &mut Ui, side: Side, windows: &[ToolWindow]) {
    egui::ScrollArea::vertical()
        .id_salt(("tool_window_stripe", side.id()))
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let origin = ui.cursor().min;
            let x = origin.x + (ui.available_width() - BUTTON) / 2.0;
            ui.set_min_height(windows.len() as f32 * (BUTTON + GAP) + 8.0);
            for (index, window) in windows.iter().enumerate() {
                button(app, ui, *window, pos2(x, origin.y + 4.0 + index as f32 * (BUTTON + GAP)));
            }
        });
}

pub fn show(app: &mut DotCraftApp, ui: &mut Ui, side: Side) {
    let rect = ui.max_rect();
    let x = rect.center().x - BUTTON / 2.0;
    match side {
        Side::Left => {
            let mut y = rect.top() + 4.0;
            let list: Vec<ToolWindow> = ToolWindow::ALL.into_iter().filter(|t| app.dock.side_of(*t) == Side::Left).collect();
            let bottoms: Vec<ToolWindow> = ToolWindow::ALL.into_iter().filter(|t| app.dock.side_of(*t) == Side::Bottom).collect();
            if (list.len() + bottoms.len()) as f32 * (BUTTON + GAP) + 8.0 > rect.height() {
                let windows: Vec<_> = list.into_iter().chain(bottoms).collect();
                overflow(app, ui, side, &windows);
                return;
            }
            for tw in list {
                button(app, ui, tw, pos2(x, y));
                y += BUTTON + GAP;
            }
            // Bottom tool windows sit at the lower end of the left stripe.
            let mut y = rect.bottom() - 4.0 - bottoms.len() as f32 * (BUTTON + GAP) + GAP;
            for tw in bottoms {
                button(app, ui, tw, pos2(x, y));
                y += BUTTON + GAP;
            }
        }
        Side::Right => {
            let mut y = rect.top() + 4.0;
            let list: Vec<ToolWindow> = ToolWindow::ALL.into_iter().filter(|t| app.dock.side_of(*t) == Side::Right).collect();
            if list.len() as f32 * (BUTTON + GAP) + 8.0 > rect.height() {
                overflow(app, ui, side, &list);
                return;
            }
            for tw in list {
                button(app, ui, tw, pos2(x, y));
                y += BUTTON + GAP;
            }
        }
        Side::Bottom => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    #[test]
    fn overflowing_stripes_scroll_to_and_toggle_the_last_tool() {
        let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        for side in [Side::Left, Side::Right] {
            let mut app = crate::screens::test_app(home.path(), true);
            if side == Side::Right {
                for window in ToolWindow::ALL {
                    app.dock.move_to(window, side);
                }
            }
            let last_side = if side == Side::Left { Side::Bottom } else { side };
            let last = ToolWindow::ALL.into_iter().filter(|window| app.dock.side_of(*window) == last_side).last().unwrap();
            let mut harness = Harness::builder().with_size(vec2(42.0, 120.0)).build_ui_state(
                move |ui, app: &mut DotCraftApp| show(app, ui, side),
                app,
            );
            harness.run_steps(3);
            harness.input_mut().events.push(egui::Event::PointerMoved(pos2(20.0, 60.0)));
            harness.input_mut().events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, -1000.0),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            });
            harness.run_steps(20);
            let target = harness.get_by_role_and_label(egui::accesskit::Role::Button, last.title());
            assert!(target.rect().top() >= 0.0 && target.rect().bottom() <= 120.0, "last tool must be visible after scrolling: {:?}", target.rect());
            target.click();
            harness.run_steps(2);
            assert!(harness.state().dock.is_open(last), "last tool must be reachable on {side:?}");
            harness.get_by_role_and_label(egui::accesskit::Role::Button, last.title()).click();
            harness.run_steps(2);
            assert!(!harness.state().dock.is_open(last));
        }
    }
}
