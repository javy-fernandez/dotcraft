//! Contents of the tool windows. Each window has the New UI header (title on the left,
//! actions and "hide" on the right) and an empty state that says what will show up there.

use crate::app::DotCraftApp;
use crate::toolwindow::ToolWindow;
use crate::ui::explorer;
use egui::{Align2, FontId, Id, Rect, Sense, Ui, pos2, vec2};
use rc_kit::i18n::tr;
use rc_kit::{Ico, icons, theme};

pub const HEADER_H: f32 = 36.0;

pub fn show(app: &mut DotCraftApp, ui: &mut Ui, tw: ToolWindow) {
    match tw {
        ToolWindow::Explorer => {
            let Some(w) = &app.workspace else { return };
            let (root, name) = (w.ws.root.clone(), w.ws.name());
            let active = app.docs.active_doc().map(|d| d.path.clone());
            let actions = explorer::show(ui, &mut app.explorer, w.solution.as_ref(), &root, &name, active.as_deref());
            if ui.ctx().data_mut(|d| d.remove_temp::<bool>(Id::new("explorer_hide")).unwrap_or(false)) {
                app.toggle_tool(ToolWindow::Explorer);
            }
            if !actions.is_empty() {
                app.session_dirty = true;
            }
            let ctx = ui.ctx().clone();
            app.handle_explorer(&ctx, actions);
        }
        _ => {
            if header(ui, tw.title()) {
                app.toggle_tool(tw);
            }
            let (title, hint) = empty_text(tw);
            empty_state(ui, tw.icon(), title, hint);
        }
    }
}

/// Tool window header; returns true when "Hide" was clicked.
pub fn header(ui: &mut Ui, title: &str) -> bool {
    let p = rc_kit::pal(ui);
    let r = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), HEADER_H));
    ui.allocate_rect(r, Sense::hover());
    ui.painter().text(pos2(r.left() + 12.0, r.center().y), Align2::LEFT_CENTER, title, theme::bold(13.0), p.text);
    let br = Rect::from_min_size(pos2(r.right() - 8.0 - 26.0, r.center().y - 13.0), vec2(26.0, 26.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(br));
    theme::icon_button(&mut child, rc_kit::ico::HIDE, true, tr("Hide")).clicked()
}

fn empty_text(tw: ToolWindow) -> (&'static str, &'static str) {
    match tw {
        ToolWindow::Structure => (tr("No structure"), tr("Open a C# file to see its types and members.")),
        ToolWindow::Commit => (tr("No changes"), tr("Changed files will show up here, ready to commit.")),
        ToolWindow::Run => (tr("Nothing is running"), tr("Run a project to see its output here.")),
        ToolWindow::Debug => (tr("No debug session"), tr("Start debugging to see frames, variables and watches.")),
        ToolWindow::Tests => (tr("No tests run yet"), tr("Run the tests of a project or file to see the results.")),
        ToolWindow::Build => (tr("No build output"), tr("Build the solution to see errors and warnings.")),
        ToolWindow::Terminal => (tr("Terminal"), tr("A shell in the workspace folder.")),
        ToolWindow::Problems => (tr("No problems"), tr("Errors and warnings of the open files show up here.")),
        ToolWindow::Git => (tr("Git"), tr("Branches, history and changes of the repository.")),
        ToolWindow::Find => (tr("No search results"), tr("Find in Files shows its results here.")),
        ToolWindow::Notifications => (tr("No notifications"), tr("Messages from builds, tools and updates show up here.")),
        ToolWindow::Explorer => ("", ""),
    }
}

/// Centered icon, title and hint.
pub fn empty_state(ui: &mut Ui, icon: Ico, title: &str, hint: &str) {
    let p = rc_kit::pal(ui);
    let rect = ui.available_rect_before_wrap();
    let c = rect.center() - vec2(0.0, 20.0);
    icons::paint_sized(ui.painter(), Rect::from_center_size(c - vec2(0.0, 30.0), vec2(16.0, 16.0)), icon, &p, false, 16.0);
    ui.painter().text(c, Align2::CENTER_CENTER, title, theme::bold(13.0), p.text_muted);
    let g = ui.painter().layout(hint.to_string(), FontId::proportional(12.5), p.muted, (rect.width() - 40.0).max(120.0));
    let pos = pos2(c.x - g.size().x / 2.0, c.y + 14.0);
    ui.painter().galley(pos, g, p.muted);
    ui.allocate_rect(rect, Sense::hover());
}
