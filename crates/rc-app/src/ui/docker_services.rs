//! Docker containers and logs in the Services window.

use crate::app::DotCraftApp;
use crate::ui::kit;
use egui::Ui;
use rc_kit::i18n::tr;
use rc_kit::ico;

/// The container rows (indented by `depth` under the Docker section).
pub fn list(app: &mut DotCraftApp, ui: &mut Ui, depth: usize) {
    if !app.docker_services.loaded && !app.docker_services.busy {
        app.docker_services.refresh();
    }
    let busy = app.docker_services.busy;
    let containers = app.docker_services.containers.clone();
    let error = app.docker_services.error.clone();
    // Docker missing or stopped is a state, not an error: a short note, the CLI's words on hover.
    if let Some(unavailable) = &app.docker_services.unavailable {
        let text = if unavailable.installed { tr("Docker isn't running") } else { tr("Docker isn't installed") };
        let details = unavailable.details.clone();
        kit::content(ui, |ui| {
            ui.horizontal(|ui| kit::note(ui, text)).response.on_hover_text(details);
        });
        return;
    }
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
    for container in &containers {
        let selected = app.docker_services.selected.as_deref() == Some(container.id.as_str());
        let icon = if container.state == "running" { ico::RUN } else { ico::STOP };
        let secondary = format!("{} · {}", container.image, container.status);
        if kit::row(ui, depth, Some(icon), &container.name, &secondary, selected).clicked() && !busy {
            app.docker_services.select(container.id.clone());
        }
    }
}

/// The selected container's actions and logs.
pub fn details(app: &mut DotCraftApp, ui: &mut Ui) {
    let busy = app.docker_services.busy;
    let Some(container) = app.docker_services.selected.as_ref().and_then(|id| app.docker_services.containers.iter().find(|c| &c.id == id)).cloned() else {
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
