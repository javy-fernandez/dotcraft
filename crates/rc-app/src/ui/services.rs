//! Services window: the Aspire app host and its resources (state, endpoints, commands and
//! console, streamed from the app host's resource service), then the Docker containers.

use crate::app::DotCraftApp;
use crate::aspire::Selection;
use crate::toolwindow::ToolWindow;
use crate::ui::kit;
use crate::ui::run_windows::{console, header, open_link, open_url};
use egui::Ui;
use rc_dotnet::aspire::{Health, Resource};
use rc_editor::Action;
use rc_kit::i18n::{tr, trf};
use rc_kit::{Ico, ico};

pub fn show(app: &mut DotCraftApp, ui: &mut Ui) {
    let docker_busy = app.docker_services.busy;
    let actions = [
        (ico::REFRESH, tr("Refresh containers"), !docker_busy),
        (ico::STOP, tr("Cancel Docker operation"), docker_busy),
    ];
    match header(ui, tr("Services"), &actions) {
        Some(0) => app.docker_services.refresh(),
        Some(1) => app.docker_services.cancel(),
        Some(usize::MAX) => app.toggle_tool(ToolWindow::Services),
        _ => {}
    }
    let hosts: Vec<crate::runner::RunConfig> = app.runner.configs.iter().filter(|c| crate::aspire::is_app_host(c)).cloned().collect();
    let aspire = !hosts.is_empty() || app.aspire.host.is_some();
    egui::ScrollArea::vertical().id_salt("services_tree").max_height(ui.available_height() * 0.45).auto_shrink([false, true]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        if aspire {
            let mut open = app.aspire.open;
            let status = host_status(app);
            if kit::section(ui, ico::CONFIG_ASPIRE, tr("Aspire"), &status, &mut open) {
                aspire_tree(app, ui, &hosts);
            }
            app.aspire.open = open;
        }
        let mut open = app.docker_services.open;
        let docker = if aspire { kit::section(ui, ico::TW_SERVICES, tr("Docker"), "", &mut open) } else { true };
        app.docker_services.open = open;
        if docker {
            crate::ui::docker_services::list(app, ui, usize::from(aspire));
        }
    });
    ui.separator();
    if app.docker_services.selected.is_some() {
        crate::ui::docker_services::details(app, ui);
        return;
    }
    match app.aspire.selection.clone() {
        Some(Selection::Host) => host_details(app, ui, &hosts),
        Some(Selection::Resource(name)) => resource_details(app, ui, &name),
        None if !aspire => {}
        None => kit::content(ui, |ui| kit::note(ui, tr("Run or debug the app host to see its resources here."))),
    }
}

fn host_status(app: &DotCraftApp) -> String {
    match &app.aspire.host {
        Some(h) if h.running && !h.connected => tr("Starting…").to_string(),
        Some(h) if h.running && h.debug => tr("Debugging").to_string(),
        Some(h) if h.running => tr("Running").to_string(),
        _ => tr("Not started").to_string(),
    }
}

fn aspire_tree(app: &mut DotCraftApp, ui: &mut Ui, hosts: &[crate::runner::RunConfig]) {
    let name = match &app.aspire.host {
        Some(h) => h.name(),
        None => hosts.first().and_then(crate::aspire::host_project).and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned())).unwrap_or_default(),
    };
    let status = host_status(app);
    let selected = app.aspire.selection == Some(Selection::Host) && app.docker_services.selected.is_none();
    if kit::row(ui, 1, Some(ico::CONFIG_ASPIRE), &name, &status, selected).clicked() {
        app.aspire.selection = Some(Selection::Host);
        app.docker_services.selected = None;
    }
    let Some(host) = &app.aspire.host else { return };
    let resources: Vec<Resource> = host.visible().into_iter().cloned().collect();
    let mut picked = None;
    for r in &resources {
        let selected = app.aspire.selection == Some(Selection::Resource(r.name.clone())) && app.docker_services.selected.is_none();
        let url = r.visible_urls().first().map(|u| u.url.clone());
        let secondary = [Some(type_label(r)), Some(state_label(r)), url].into_iter().flatten().collect::<Vec<_>>().join(" · ");
        if kit::row(ui, 2, Some(state_icon(r)), &r.display_name, &secondary, selected).clicked() {
            picked = Some(r.name.clone());
        }
    }
    if let Some(name) = picked {
        app.docker_services.selected = None;
        app.aspire.select_resource(&name);
    }
}

/// The resource type in the user's language when DotCraft knows it.
fn type_label(r: &Resource) -> String {
    match r.resource_type.as_str() {
        "Project" => tr("Project").to_string(),
        "Container" => tr("Container").to_string(),
        "Executable" => tr("Executable").to_string(),
        "Parameter" => tr("Parameter").to_string(),
        other => other.to_string(),
    }
}

/// The state in the user's language (unknown ones as the app host says them).
pub fn state_label(r: &Resource) -> String {
    let Some(state) = r.state.as_deref() else { return tr("Unknown").to_string() };
    let label = match state {
        "Running" => tr("Running"),
        "Starting" => tr("Starting…"),
        "Waiting" => tr("Waiting"),
        "NotStarted" => tr("Not started"),
        "Building" => tr("Building…"),
        "Stopping" => tr("Stopping…"),
        "Finished" | "Exited" => tr("Finished"),
        "FailedToStart" => tr("Failed to start"),
        "RuntimeUnhealthy" => tr("Container runtime unhealthy"),
        "Hidden" => tr("Hidden"),
        other => return other.to_string(),
    };
    match (state, r.health) {
        ("Running", Some(Health::Unhealthy)) => format!("{label} ({})", tr("Unhealthy")),
        ("Running", Some(Health::Degraded)) => format!("{label} ({})", tr("Degraded")),
        _ => label.to_string(),
    }
}

fn state_icon(r: &Resource) -> Ico {
    match (r.state.as_deref(), r.health, r.state_style.as_deref()) {
        (_, _, Some("error")) | (Some("FailedToStart" | "RuntimeUnhealthy"), ..) => ico::TEST_ERROR,
        (Some("Running"), Some(Health::Unhealthy | Health::Degraded), _) => ico::WARNING,
        (Some("Running"), ..) => ico::TEST_PASSED,
        (Some("Finished" | "Exited"), ..) => ico::TEST_TERMINATED,
        (None, ..) => ico::TEST_UNKNOWN,
        _ => ico::TEST_NOT_RUN,
    }
}

fn host_details(app: &mut DotCraftApp, ui: &mut Ui, hosts: &[crate::runner::RunConfig]) {
    let running = app.aspire.host.as_ref().is_some_and(|h| h.running);
    let dashboard = app.aspire.host.as_ref().filter(|h| h.running).and_then(|h| h.dashboard.clone());
    kit::toolbar(ui, |ui| {
        if running {
            if kit::button(ui, Some(ico::STOP), tr("Stop"), true).clicked() {
                app.pending.push(Action::StopRun);
            }
        } else {
            // The configuration that ran last, else the first of the app host.
            let config = app.aspire.host.as_ref().map(|h| h.config.clone()).or_else(|| hosts.first().cloned());
            let can = config.as_ref().and_then(|c| app.runner.configs.iter().position(|x| x == c));
            if kit::button(ui, Some(ico::RUN), tr("Run"), can.is_some()).clicked()
                && let Some(i) = can
            {
                app.runner.selected = i;
                app.pending.push(Action::Run);
            }
            if kit::button(ui, Some(ico::DEBUG), tr("Debug"), can.is_some()).clicked()
                && let Some(i) = can
            {
                app.runner.selected = i;
                app.pending.push(Action::Debug);
            }
        }
        if kit::button(ui, Some(ico::EXTERNAL_LINK), tr("Open Dashboard"), dashboard.is_some()).clicked()
            && let Some(url) = &dashboard
        {
            open_url(url);
        }
    });
    kit::content(ui, |ui| {
        match &app.aspire.host {
            Some(h) if h.running && !h.connected => kit::note(ui, tr("Waiting for the app host's resource service…")),
            Some(h) if h.running => {
                kit::note(ui, &trf("{count} resources", &[("count", &h.visible().len())]));
                if h.debug {
                    kit::note(ui, tr("Every .NET project the app host starts runs under the debugger in its own Debug tab."));
                }
            }
            _ => kit::note(ui, tr("Run or debug the app host to see its resources here.")),
        }
        if let Some(url) = &dashboard {
            ui.add_space(4.0);
            if kit::button(ui, Some(ico::EXTERNAL_LINK), url, true).on_hover_text(tr("Open Dashboard")).clicked() {
                open_url(url);
            }
        }
    });
}

fn resource_details(app: &mut DotCraftApp, ui: &mut Ui, name: &str) {
    let Some(host) = &app.aspire.host else { return };
    let Some(r) = host.resource(name).cloned() else {
        kit::content(ui, |ui| kit::note(ui, tr("The resource is no longer running.")));
        return;
    };
    let busy = host.busy(name);
    let dashboard = host.dashboard.clone();
    let mut command = None;
    kit::toolbar(ui, |ui| {
        for (id, icon, label) in [("resource-start", ico::RUN, tr("Start")), ("resource-stop", ico::STOP, tr("Stop")), ("resource-restart", ico::RESTART, tr("Restart"))] {
            if let Some(c) = r.command(id)
                && kit::button(ui, Some(icon), label, c.enabled && !busy).clicked()
            {
                command = Some(id.to_string());
            }
        }
        // Commands the app model adds keep the app host's own text.
        for c in r.commands.iter().filter(|c| !c.hidden && !c.name.starts_with("resource-")) {
            if kit::button(ui, None, &c.display_name, c.enabled && !busy).clicked() {
                command = Some(c.name.clone());
            }
        }
        if let Some(url) = &dashboard
            && kit::button(ui, Some(ico::EXTERNAL_LINK), tr("Open Dashboard"), true).clicked()
        {
            open_url(url);
        }
    });
    if let Some(command) = command {
        app.aspire.execute(name, &command);
    }
    let urls: Vec<(String, String)> = r.visible_urls().iter().map(|u| (u.display_name.clone().unwrap_or_else(|| u.url.clone()), u.url.clone())).collect();
    kit::content(ui, |ui| {
        let state = format!("{} · {}", type_label(&r), state_label(&r));
        let project = r.project_path().map(|p| format!(" · {p}")).unwrap_or_default();
        kit::note(ui, &format!("{state}{project}"));
        if !urls.is_empty() {
            ui.horizontal_wrapped(|ui| {
                for (label, url) in &urls {
                    if kit::button(ui, Some(ico::EXTERNAL_LINK), label, true).on_hover_text(url).clicked() {
                        open_url(url);
                    }
                }
            });
        }
    });
    ui.separator();
    let Some(logs) = app.aspire.logs.as_ref().filter(|l| l.resource == name) else { return };
    let cwd = r.project_path().and_then(|p| std::path::Path::new(p).parent().map(|d| d.to_path_buf()));
    for event in console(ui, &logs.console.term) {
        if let rc_term::ViewEvent::LinkClicked(link) = event {
            open_link(app, link, cwd.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::aspire::Selection;
    use egui_kittest::{Harness, kittest::Queryable};
    use rc_kit::i18n::tr;

    #[test]
    fn resources_are_listed_and_their_commands_run() {
        let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(home.path()).unwrap().join("Shop");
        let resources = crate::screens::aspire_sample(&root);
        let mut app = crate::screens::test_app(home.path(), true);
        app.open_workspace(&root);
        let config = app.runner.configs.iter().find(|c| crate::aspire::is_app_host(c)).cloned().expect("app host configuration");
        assert_eq!(config.icon(), rc_kit::ico::CONFIG_ASPIRE);
        app.aspire.demo(&config, resources, "https://localhost:17175/login?t=abc");
        assert_eq!(app.aspire_dashboard().as_deref(), Some("https://localhost:17175/login?t=abc"));
        let mut first = true;
        let mut h = Harness::builder().with_size(egui::vec2(1100.0, 700.0)).build_ui_state(
            move |ui, app: &mut crate::app::DotCraftApp| {
                if std::mem::take(&mut first) {
                    rc_kit::theme::install_fonts(ui.ctx());
                    return;
                }
                super::show(app, ui);
            },
            app,
        );
        h.run_steps(2);
        h.get_by_label("webfrontend").click();
        h.run_steps(2);
        assert_eq!(h.state().aspire.selection, Some(Selection::Resource("webfrontend-c8kd1p".into())));
        assert_eq!(h.state().aspire.logs.as_ref().map(|l| l.resource.as_str()), Some("webfrontend-c8kd1p"));
        h.get_by_label(tr("Restart")).click();
        h.run_steps(2);
        let pending = &h.state().aspire.host.as_ref().unwrap().pending;
        assert_eq!(pending, &vec![("webfrontend-c8kd1p".to_string(), "resource-restart".to_string())]);
        // The app host row shows its actions.
        h.get_by_label("Shop.AppHost").click();
        h.run_steps(2);
        assert_eq!(h.state().aspire.selection, Some(Selection::Host));
        assert!(h.query_by_label(tr("Open Dashboard")).is_some());
    }
}
