//! Docker containers and logs in the Services window.

use crate::app::DotCraftApp;
use crate::toolwindow::ToolWindow;
use crate::ui::kit;
use crate::ui::run_windows::header;
use egui::Ui;
use rc_kit::i18n::tr;
use rc_kit::ico;

pub fn show(app: &mut DotCraftApp, ui: &mut Ui) {
    if !app.docker_services.loaded && !app.docker_services.busy {
        app.docker_services.refresh();
    }
    let busy = app.docker_services.busy;
    let actions = [
        (ico::REFRESH, tr("Refresh containers"), !busy),
        (ico::STOP, tr("Cancel Docker operation"), busy),
    ];
    match header(ui, tr("Services"), &actions) {
        Some(0) => app.docker_services.refresh(),
        Some(1) => app.docker_services.cancel(),
        Some(usize::MAX) => app.toggle_tool(ToolWindow::Services),
        _ => {}
    }
    let containers = app.docker_services.containers.clone();
    let error = app.docker_services.error.clone();
    let empty = containers.is_empty() && app.docker_services.loaded;
    if error.is_some() || empty {
        kit::content(ui, |ui| {
            if let Some(error) = &error {
                kit::message(ui, error, true);
            }
            if empty {
                kit::note(ui, tr("No Docker containers found"));
            }
        });
    }
    {
        egui::ScrollArea::vertical().id_salt("docker_containers").max_height(ui.available_height() * 0.45).show(ui, |ui| {
            for container in &containers {
                let selected = app.docker_services.selected.as_deref() == Some(container.id.as_str());
                let icon = if container.state == "running" { ico::RUN } else { ico::STOP };
                let secondary = format!("{} · {}", container.image, container.status);
                if kit::row(ui, 0, Some(icon), &container.name, &secondary, selected).clicked() && !busy {
                    app.docker_services.select(container.id.clone());
                }
            }
        });
        let Some(container) = app.docker_services.selected.as_ref().and_then(|id| containers.iter().find(|c| &c.id == id)) else {
            return;
        };
        let running = container.state == "running";
        let id = container.id.clone();
        kit::toolbar(ui, |ui| {
            let (icon, label) = if running { (ico::STOP, tr("Stop container")) } else { (ico::RUN, tr("Start container")) };
            if kit::button(ui, Some(icon), label, !busy).clicked() {
                app.docker_services.set_running(id.clone(), !running);
            }
            if kit::button(ui, Some(ico::REFRESH), tr("Refresh logs"), !busy).clicked() {
                app.docker_services.load_logs(id.clone());
            }
        });
        ui.separator();
        egui::ScrollArea::vertical().id_salt("docker_logs").auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
            ui.add(egui::TextEdit::multiline(&mut app.docker_services.logs).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY).interactive(false));
        });
    }
}
