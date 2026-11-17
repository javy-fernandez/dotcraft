//! gRPC Client window: endpoint, method (picked from the active `.proto`), TLS options,
//! metadata and JSON request on the left; the response on the right (stacked when narrow).

use crate::app::DotCraftApp;
use crate::toolwindow::ToolWindow;
use crate::ui::kit;
use egui::{Align2, FontId, Rect, Sense, TextEdit, Ui, pos2, vec2};
use rc_kit::i18n::tr;
use rc_kit::{ico, theme};

pub fn show(app: &mut DotCraftApp, ui: &mut Ui) {
    let p = rc_kit::pal(ui);
    let active_proto = app.active_proto().and_then(|file| {
        let doc = app.docs.active_doc()?;
        Some((doc.path.clone(), file.methods(), doc.editor.buffer.is_modified()))
    });
    let busy = app.grpc.busy;
    let status = match &app.grpc.response {
        _ if busy => tr("Sending gRPC request…").to_string(),
        Some(Ok(reply)) => format!("OK · {} ms", reply.elapsed.as_millis()),
        _ => String::new(),
    };
    let actions = [(ico::HIDE, tr("Hide"), true), (ico::RUN, tr("Send gRPC Request"), !busy), (ico::STOP, tr("Cancel Request"), busy)];
    match kit::header(ui, tr("gRPC Client"), &[], &status, &actions).action {
        Some(0) => {
            app.toggle_tool(ToolWindow::GrpcClient);
            return;
        }
        Some(1) => send(app, active_proto.as_ref().map(|(path, _, modified)| (path.clone(), *modified))),
        Some(2) => app.grpc.cancel(),
        _ => {}
    }
    kit::toolbar(ui, |ui| {
        kit::field(ui, tr("Endpoint"), &mut app.grpc.endpoint, "localhost:5000", 200.0, true);
        kit::field(ui, tr("Method"), &mut app.grpc.method, "package.Service/Method", 300.0, true);
        if let Some((_, methods, _)) = &active_proto
            && !methods.is_empty()
        {
            ui.vertical(|ui| {
                ui.add_space(17.0);
                if let Some(i) = kit::dropdown(ui, "grpc_proto_methods", tr("Choose RPC from .proto"), methods, true) {
                    app.grpc.method = methods[i].clone();
                }
            });
        }
    });
    kit::toolbar(ui, |ui| {
        ui.checkbox(&mut app.grpc.plaintext, tr("Plaintext (no TLS)"));
        if !app.grpc.plaintext {
            ui.checkbox(&mut app.grpc.insecure, tr("Skip certificate check"));
        }
        ui.add_enabled_ui(active_proto.is_some(), |ui| ui.checkbox(&mut app.grpc.use_proto, tr("Use active .proto")));
        if app.grpc.use_proto && active_proto.is_some() {
            ui.label(egui::RichText::new(tr("Import path")).size(12.0).color(p.muted));
            ui.add(TextEdit::singleline(&mut app.grpc.import_path).hint_text(tr("Folder of the .proto")).desired_width(200.0).margin(vec2(6.0, 4.0)));
        } else if active_proto.is_none() {
            kit::note(ui, tr("Open a .proto file for a local schema; otherwise server reflection is used."));
        }
    });
    // Request on the left, response on the right; one above the other when narrow.
    let area = ui.available_rect_before_wrap();
    let wide = area.width() >= 760.0;
    let (request_rect, response_rect) = if wide {
        let split = area.left() + (area.width() * 0.45).round();
        (Rect::from_min_max(area.min, pos2(split, area.bottom())), Rect::from_min_max(pos2(split, area.top()), area.max))
    } else {
        let split = area.top() + (area.height() * 0.5).round();
        (Rect::from_min_max(area.min, pos2(area.right(), split)), Rect::from_min_max(pos2(area.left(), split), area.max))
    };
    ui.allocate_rect(area, Sense::hover());
    let mut request_ui = ui.new_child(egui::UiBuilder::new().max_rect(request_rect));
    request_ui.set_clip_rect(request_rect.intersect(ui.clip_rect()));
    kit::content(&mut request_ui, |ui| {
        // Request JSON / Metadata, like the HTTP Client's Body / Headers.
        let headers = app.grpc.metadata.lines().filter(|l| !l.trim().is_empty()).count();
        let metadata_tab = if headers == 0 { tr("Metadata").to_string() } else { format!("{} ({headers})", tr("Metadata")) };
        ui.horizontal(|ui| {
            if theme::tool_button(ui, ico::FILE_TEXT, tr("Request JSON"), !app.grpc.show_metadata).clicked() {
                app.grpc.show_metadata = false;
            }
            if theme::tool_button(ui, ico::MENU, &metadata_tab, app.grpc.show_metadata).clicked() {
                app.grpc.show_metadata = true;
            }
        });
        ui.add_space(4.0);
        let size = [ui.available_width(), ui.available_height().max(48.0)];
        if app.grpc.show_metadata {
            ui.add_sized(size, TextEdit::multiline(&mut app.grpc.metadata).hint_text(tr("Key: Value per line, such as authorization: Bearer …")).code_editor());
        } else {
            ui.add_sized(size, TextEdit::multiline(&mut app.grpc.json).code_editor());
        }
    });
    let line = if wide { egui::Stroke::new(1.0, p.island_border) } else { egui::Stroke::NONE };
    ui.painter().vline(response_rect.left(), response_rect.y_range(), line);
    if !wide {
        ui.painter().hline(response_rect.x_range(), response_rect.top(), egui::Stroke::new(1.0, p.island_border));
    }
    let mut response_ui = ui.new_child(egui::UiBuilder::new().max_rect(response_rect.shrink2(vec2(1.0, 1.0))));
    response_ui.set_clip_rect(response_rect.intersect(ui.clip_rect()));
    response(app, &mut response_ui);
}

fn send(app: &mut DotCraftApp, active_proto: Option<(std::path::PathBuf, bool)>) {
    if app.grpc.use_proto && active_proto.as_ref().is_some_and(|(_, modified)| *modified) {
        app.grpc.response = Some(Err(tr("Save the .proto file before sending to use its latest schema.").to_string()));
        app.grpc.body = None;
        return;
    }
    if let Err(error) = app.grpc.send(active_proto.map(|(path, _)| path)) {
        app.grpc.response = Some(Err(error));
        app.grpc.body = None;
    }
}

fn response(app: &mut DotCraftApp, ui: &mut Ui) {
    let p = rc_kit::pal(ui);
    match &app.grpc.response {
        None if app.grpc.busy => {
            let r = ui.available_rect_before_wrap();
            ui.painter().text(pos2(r.left() + 12.0, r.top() + 16.0), Align2::LEFT_CENTER, tr("Sending gRPC request…"), FontId::proportional(12.5), p.muted);
        }
        None => {
            let (title, hint) = (tr("No gRPC request"), tr("Enter an endpoint and method to call a gRPC service."));
            crate::ui::toolwindows::empty_state(ui, ToolWindow::GrpcClient.icon(), title, hint);
        }
        Some(Err(error)) => {
            let error = error.clone();
            kit::content(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("grpc_error").auto_shrink([false, false]).show(ui, |ui| {
                    ui.label(egui::RichText::new(error).size(12.5).monospace().color(p.error));
                });
            });
        }
        Some(Ok(_)) => {
            if let Some(body) = &mut app.grpc.body {
                let rect = ui.available_rect_before_wrap();
                let config = app.editor_config.clone();
                let mut eu = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                eu.set_clip_rect(rect.intersect(ui.clip_rect()));
                rc_editor::EditorView::new(body, &config, &app.keymap).show(&mut eu);
                ui.allocate_rect(rect, Sense::hover());
            }
        }
    }
}
