//! Docker containers and logs in the Services window.

use crate::app::DotCraftApp;
use crate::toolwindow::ToolWindow;
use crate::ui::run_windows::header;
use egui::Ui;
use rc_kit::i18n::tr;
use rc_kit::ico;

pub fn show(app: &mut DotCraftApp, ui: &mut Ui) {
    if !app.docker_services.loaded && !app.docker_services.busy {
        app.docker_services.refresh();
    }
    let actions = [
        (ico::RERUN, tr("Refresh containers"), !app.docker_services.busy),
        (ico::STOP, tr("Cancel Docker operation"), app.docker_services.busy),
    ];
    match header(ui, tr("Services"), &actions) {
        Some(0) => app.docker_services.refresh(),
        Some(1) => app.docker_services.cancel(),
        Some(usize::MAX) => app.toggle_tool(ToolWindow::Services),
        _ => {}
    }
    if let Some(error) = &app.docker_services.error {
        ui.colored_label(ui.visuals().error_fg_color, error);
    }
    let containers = app.docker_services.containers.clone();
    if containers.is_empty() {
        ui.label(tr("No Docker containers found"));
    }
    egui::ScrollArea::vertical().max_height(ui.available_height() * 0.45).show(ui, |ui| {
        for container in containers {
            let selected = app.docker_services.selected.as_deref() == Some(&container.id);
            ui.horizontal(|ui| {
                if ui.add_enabled_ui(!app.docker_services.busy, |ui| ui.selectable_label(selected, format!("{}  ·  {}  ·  {}", container.name, container.image, container.status))).inner.clicked() {
                    app.docker_services.select(container.id.clone());
                }
                let running = container.state == "running";
                let label = if running { tr("Stop container") } else { tr("Start container") };
                if ui.add_enabled(!app.docker_services.busy, egui::Button::new(label)).clicked() {
                    app.docker_services.set_running(container.id.clone(), !running);
                }
            });
        }
    });
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(tr("Container logs"));
        if let Some(id) = app.docker_services.selected.clone()
            && ui.add_enabled(!app.docker_services.busy, egui::Button::new(tr("Refresh logs"))).clicked()
        {
            app.docker_services.load_logs(id);
        }
    });
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.add(egui::TextEdit::multiline(&mut app.docker_services.logs).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY).interactive(false));
    });
}
