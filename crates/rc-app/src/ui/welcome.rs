//! Welcome screen (no workspace open): logo, open buttons and recent projects.

use crate::app::DotCraftApp;
use egui::{Align2, FontId, Rect, Sense, Ui, pos2, vec2};
use rc_kit::i18n::{tr, trf};
use rc_kit::{ico, icons, theme};

pub fn show(app: &mut DotCraftApp, ui: &mut Ui) {
    let p = rc_kit::pal(ui);
    theme::fixed_island(ui, &p, |ui| {
        let rect = ui.max_rect();
        let col_w = 460.0f32.min(rect.width() - 40.0);
        let left = rect.center().x - col_w / 2.0;
        let mut y = rect.top() + (rect.height() * 0.16).max(40.0);
        let logo = Rect::from_min_size(pos2(rect.center().x - 32.0, y), vec2(64.0, 64.0));
        rc_kit::branding::paint(ui, logo);
        y = logo.bottom() + 18.0;
        ui.painter().text(pos2(rect.center().x, y), Align2::CENTER_TOP, tr("Welcome to DotCraft"), theme::bold(20.0), p.text);
        y += 34.0;
        let subtitle = format!("{} · {}", tr("A lightweight IDE for .NET"), trf("Version {version}", &[("version", &env!("CARGO_PKG_VERSION"))]));
        ui.painter().text(pos2(rect.center().x, y), Align2::CENTER_TOP, subtitle, FontId::proportional(13.0), p.text_muted);
        y += 40.0;

        // Primary actions.
        let buttons = [(tr("New Solution…"), ico::ADD, 2), (tr("Open…"), ico::OPEN, 0), (tr("Open Solution or Project…"), ico::SOLUTION, 1)];
        let bw = (col_w - 24.0) / 3.0;
        for (i, (label, icon, kind)) in buttons.into_iter().enumerate() {
            let r = Rect::from_min_size(pos2(left + i as f32 * (bw + 12.0), y), vec2(bw, 64.0));
            let resp = ui.interact(r, ui.id().with(("welcome", i)), Sense::click());
            let fill = if resp.hovered() { theme::blend(p.island_bg, p.hover) } else { p.island_bg };
            ui.painter().rect(r, 10.0, fill, egui::Stroke::new(1.0, if resp.hovered() { p.accent } else { p.control_border }), egui::StrokeKind::Inside);
            icons::paint(ui.painter(), Rect::from_center_size(pos2(r.center().x, r.top() + 22.0), vec2(16.0, 16.0)), icon, &p, true);
            ui.painter().text(pos2(r.center().x, r.bottom() - 18.0), Align2::CENTER_CENTER, label, FontId::proportional(13.0), p.text);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
            if resp.clicked() {
                match kind {
                    0 => app.pick_and_open(),
                    1 => app.pick_solution(),
                    _ => app.new_project_dialog(None),
                }
            }
        }
        y += 64.0 + 32.0;

        // Recent projects.
        let recents: Vec<std::path::PathBuf> = app.settings.recent_workspaces.clone();
        ui.painter().text(pos2(left, y), Align2::LEFT_TOP, tr("Recent Projects"), theme::bold(13.0), p.text);
        y += 26.0;
        if recents.is_empty() {
            ui.painter().text(pos2(left, y + 4.0), Align2::LEFT_TOP, tr("Projects you open will be listed here."), FontId::proportional(13.0), p.muted);
        }
        let mut remove = None;
        for (i, path) in recents.iter().enumerate().take(12) {
            let r = Rect::from_min_size(pos2(left - 8.0, y), vec2(col_w + 16.0, 44.0));
            if r.bottom() > rect.bottom() - 10.0 {
                break;
            }
            let resp = ui.interact(r, ui.id().with(("recent", i)), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(r, 8.0, p.row_hover);
            }
            let name = path.file_stem().map_or_else(|| path.display().to_string(), |s| s.to_string_lossy().into_owned());
            let exists = path.exists();
            let initials = crate::ui::header::initials(&name);
            let badge = Rect::from_min_size(pos2(r.left() + 8.0, r.center().y - 14.0), vec2(28.0, 28.0));
            ui.painter().rect_filled(badge, 6.0, if exists { p.accent.gamma_multiply(0.85) } else { p.disabled });
            ui.painter().text(badge.center(), Align2::CENTER_CENTER, initials, theme::bold(11.0), egui::Color32::WHITE);
            ui.painter().text(pos2(badge.right() + 12.0, r.top() + 7.0), Align2::LEFT_TOP, &name, theme::bold(13.0), if exists { p.text } else { p.disabled });
            let sub = if exists { path.display().to_string() } else { format!("{} — {}", path.display(), tr("not found")) };
            ui.painter().text(pos2(badge.right() + 12.0, r.top() + 25.0), Align2::LEFT_TOP, sub, FontId::proportional(12.0), p.muted);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
            let resp = resp.on_hover_text(path.display().to_string());
            if resp.clicked() && exists {
                app.open_workspace(path);
            }
            resp.context_menu(|ui| {
                if theme::menu_item(ui, Some(ico::REMOVE), tr("Remove from Recent Projects"), false).clicked() {
                    remove = Some(i);
                    ui.close();
                }
            });
            y += 46.0;
        }
        if let Some(i) = remove {
            app.settings.recent_workspaces.remove(i);
            let _ = app.settings.save();
        }
        // Folders or solutions dropped on the window open.
        let dropped: Vec<std::path::PathBuf> = ui.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if let Some(path) = dropped.into_iter().next() {
            app.open_workspace(&path);
        }
        ui.allocate_rect(rect, Sense::hover());
    });
}
