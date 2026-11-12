//! Aspire AppHost controls, resource state, console logs and dashboard navigation.

use crate::app::DotCraftApp;
use crate::runner::ConfigKind;
use crate::toolwindow::ToolWindow;
use crate::ui::run_windows::{header, open_url};
use crate::ui::toolwindows::empty_state;
use egui::{ScrollArea, Ui};
use rc_editor::Action;
use rc_kit::i18n::tr;
use rc_kit::ico;

pub fn show(app: &mut DotCraftApp, ui: &mut Ui) {
    let host = app.aspire.host().map(std::path::Path::to_path_buf);
    let debugging_host = app.debugger.active() && app.runner.configs.iter().any(|config| config.name == app.debugger.name && matches!(&config.kind, ConfigKind::App { project, .. } if Some(project) == host.as_ref()));
    let running = host.as_ref().is_some_and(|host| {
        app.runner.runs.iter().any(|run| {
            run.running()
                && matches!(&run.config.kind, ConfigKind::App { project, .. } if project == host)
        })
    }) || debugging_host;
    let dashboard = host
        .as_ref()
        .and_then(|host| {
            app.runner
                .runs
                .iter()
                .rev()
                .find_map(|run| match &run.config.kind {
                    ConfigKind::App { project, .. } if project == host => run.dashboard_url.clone(),
                    _ => None,
                })
        })
        .or_else(|| {
            if debugging_host {
                app.debugger.dashboard_url.clone()
            } else {
                None
            }
        });
    let actions = [
        (ico::RUN, tr("Run AppHost"), host.is_some()),
        (ico::DEBUG, tr("Debug AppHost"), host.is_some()),
        (
            ico::REFRESH,
            tr("Refresh Resources"),
            host.is_some() && !app.aspire.loading_resources,
        ),
        (
            ico::EXTERNAL_LINK,
            tr("Open Aspire Dashboard"),
            dashboard.is_some(),
        ),
    ];
    match header(ui, tr("Aspire Resources"), &actions) {
        command @ (Some(0) | Some(1)) => {
            if let Some(host) = &host
                && let Some(index) = app.runner.configs.iter().position(|config| matches!(&config.kind, ConfigKind::App { project, .. } if project == host))
            {
                app.runner.selected = index;
                app.pending.push(if command == Some(0) { Action::Run } else { Action::Debug });
            }
        }
        Some(2) => app.aspire.refresh(),
        Some(3) => {
            if let Some(url) = dashboard {
                open_url(&url);
            }
        }
        Some(usize::MAX) => app.toggle_tool(ToolWindow::AspireResources),
        _ => {}
    }
    if app.aspire.hosts.is_empty() {
        empty_state(
            ui,
            ToolWindow::AspireResources.icon(),
            tr("No Aspire AppHost"),
            tr("Open a solution with an Aspire AppHost project."),
        );
        return;
    }
    if app.aspire.hosts.len() > 1 {
        let selected = app.aspire.selected_host;
        let mut chosen = None;
        egui::ComboBox::from_id_salt("aspire_host")
            .selected_text(
                app.aspire
                    .host()
                    .and_then(|h| h.file_stem())
                    .unwrap_or_default()
                    .to_string_lossy(),
            )
            .show_ui(ui, |ui| {
                for (index, host) in app.aspire.hosts.iter().enumerate() {
                    if ui
                        .selectable_label(
                            selected == index,
                            host.file_stem().unwrap_or_default().to_string_lossy(),
                        )
                        .clicked()
                    {
                        chosen = Some(index);
                    }
                }
            });
        if let Some(index) = chosen {
            app.aspire.select_host(index);
        }
    }
    if !running {
        ui.label(tr("Run the AppHost to see live resource state and logs."));
    }
    if let Some(error) = &app.aspire.error {
        ui.colored_label(ui.visuals().error_fg_color, error);
    }
    ui.separator();
    if app.aspire.resources.is_empty() {
        empty_state(
            ui,
            ToolWindow::AspireResources.icon(),
            tr("No Aspire resources"),
            tr("Start the AppHost and refresh to list its resources."),
        );
        return;
    }
    let mut selected = None;
    ScrollArea::vertical()
        .max_height(ui.available_height() * 0.48)
        .show(ui, |ui| {
            for resource in &app.aspire.resources {
                let label = format!(
                    "{}  ·  {}  ·  {}  ·  {}",
                    resource.name, resource.kind, resource.state, resource.health
                );
                if ui
                    .selectable_label(
                        app.aspire.selected_resource.as_deref() == Some(&resource.name),
                        label,
                    )
                    .clicked()
                {
                    selected = Some(resource.name.clone());
                }
                for url in &resource.urls {
                    if ui.link(url).clicked() {
                        open_url(url);
                    }
                }
            }
        });
    if let Some(name) = selected {
        app.aspire.select_resource(&name);
    }
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(tr("Resource Logs"));
        if ui.small_button(tr("Refresh Logs")).clicked() {
            app.aspire.refresh_logs();
        }
    });
    if app.aspire.loading_logs {
        ui.label(tr("Loading logs…"));
    }
    ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
        ui.add(egui::Label::new(egui::RichText::new(&app.aspire.logs).monospace()).wrap());
    });
}
