//! gRPC client window: reflection or the active `.proto`, JSON input, metadata and response.

use crate::app::DotCraftApp;
use crate::toolwindow::ToolWindow;
use crate::ui::run_windows::header;
use egui::{ScrollArea, TextEdit, Ui};
use rc_kit::i18n::tr;
use rc_kit::ico;

pub fn show(app: &mut DotCraftApp, ui: &mut Ui) {
    let active_proto = app
        .docs
        .active_doc()
        .filter(|doc| rc_syntax::Lang::from_path(&doc.path) == rc_syntax::Lang::Proto)
        .map(|doc| {
            (
                doc.path.clone(),
                crate::proto::methods(&doc.editor.buffer.rope().to_string()),
                doc.editor.buffer.is_modified(),
            )
        });
    let actions = [
        (ico::RUN, tr("Send gRPC Request"), !app.grpc.busy),
        (ico::STOP, tr("Cancel Request"), app.grpc.busy),
    ];
    match header(ui, tr("gRPC Client"), &actions) {
        Some(0) => {
            if app.grpc.use_proto
                && active_proto
                    .as_ref()
                    .is_some_and(|(_, _, modified)| *modified)
            {
                app.grpc.response = Some(Err(tr(
                    "Save the .proto file before sending to use its latest schema.",
                )
                .to_string()));
            } else {
                let path = active_proto.as_ref().map(|(path, _, _)| path.clone());
                if let Err(error) = app.grpc.send(path) {
                    app.grpc.response = Some(Err(error));
                }
            }
        }
        Some(1) => app.grpc.cancel(),
        Some(usize::MAX) => app.toggle_tool(ToolWindow::GrpcClient),
        _ => {}
    }
    ui.horizontal(|ui| {
        ui.label(tr("Endpoint"));
        ui.text_edit_singleline(&mut app.grpc.endpoint);
    });
    ui.horizontal(|ui| {
        ui.label(tr("Method"));
        ui.text_edit_singleline(&mut app.grpc.method);
    });
    if let Some((_, methods, _)) = &active_proto
        && !methods.is_empty()
    {
        egui::ComboBox::from_id_salt("grpc_proto_methods")
            .selected_text(tr("Choose RPC from .proto"))
            .show_ui(ui, |ui| {
                for method in methods {
                    if ui
                        .selectable_label(app.grpc.method == *method, method)
                        .clicked()
                    {
                        app.grpc.method = method.clone();
                    }
                }
            });
    }
    ui.horizontal(|ui| {
        ui.checkbox(&mut app.grpc.plaintext, tr("Plaintext (no TLS)"));
        ui.add_enabled_ui(active_proto.is_some(), |ui| {
            ui.checkbox(&mut app.grpc.use_proto, tr("Use active .proto"));
        });
    });
    if active_proto.is_none() {
        ui.label(tr(
            "Open a .proto file for a local schema; otherwise server reflection is used.",
        ));
    }
    if app.grpc.use_proto && active_proto.is_some() {
        ui.horizontal(|ui| {
            ui.label(tr("Import path"));
            ui.text_edit_singleline(&mut app.grpc.import_path);
        });
    }
    ui.label(tr("Metadata (Key: Value per line)"));
    ui.add(
        TextEdit::multiline(&mut app.grpc.metadata)
            .desired_rows(3)
            .code_editor(),
    );
    ui.label(tr("Request JSON"));
    ui.add(
        TextEdit::multiline(&mut app.grpc.json)
            .desired_rows(8)
            .code_editor(),
    );
    ui.separator();
    ui.label(tr("Response"));
    if app.grpc.busy {
        ui.label(tr("Sending gRPC request…"));
    }
    match &app.grpc.response {
        Some(Ok(body)) => {
            ScrollArea::vertical().show(ui, |ui| {
                ui.add(egui::Label::new(egui::RichText::new(body).monospace()).wrap());
            });
        }
        Some(Err(error)) => {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        None => {
            ui.label(tr("Send a request to see the gRPC response."));
        }
    }
}
