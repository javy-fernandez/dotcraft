//! Explicit request selection and a bounded plain-text response view.

use crate::app::DotCraftApp;
use crate::http_client::{HttpError, MAX_SHOWN_CHARS};
use crate::toolwindow::ToolWindow;
use egui::Ui;
use rc_kit::i18n::{tr, trf};

pub fn show(app: &mut DotCraftApp, ui: &mut Ui) {
    if crate::ui::toolwindows::header(ui, tr("HTTP Client")) {
        app.toggle_tool(ToolWindow::Http);
    }
    let active = app.docs.active_doc().filter(|doc| {
        doc.path.extension().and_then(|value| value.to_str()).is_some_and(|extension| extension.eq_ignore_ascii_case("http") || extension.eq_ignore_ascii_case("rest"))
    });
    if let Some(doc) = active {
        app.http.prepare(&doc.path, doc.editor.id, doc.editor.buffer.version(), || {
            if doc.editor.buffer.rope().len_bytes() > crate::http_client::MAX_REQUEST_BYTES {
                Err(HttpError::Invalid("request file exceeds 1 MiB".into()))
            } else {
                Ok(doc.editor.buffer.rope().to_string())
            }
        });
    }
    let running = app.http.running();
    let mut selected = app.http.selected;
    let mut send = None;
    match active.and_then(|_| app.http.requests()) {
        None => { ui.label(tr("Open an .http or .rest file to select a request.")); }
        Some(Err(error)) => { error_label(ui, error); }
        Some(Ok(requests)) if requests.is_empty() => { ui.label(tr("No HTTP requests found.")); }
        Some(Ok(requests)) => {
            selected = selected.min(requests.len() - 1);
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("http_request_selection")
                    .selected_text(requests[selected].title())
                    .width(320.0)
                    .show_ui(ui, |ui| {
                        for (index, request) in requests.iter().enumerate() {
                            ui.selectable_value(&mut selected, index, format!("{}: {}", request.line + 1, request.title()));
                        }
                    });
                if ui.add_enabled(!running, egui::Button::new(tr("Send Request"))).clicked() {
                    send = Some(requests[selected].clone());
                }
            });
        }
    }
    app.http.selected = selected;
    if let Some(request) = send { app.http.start(request); }
    if app.http.running() {
        ui.horizontal(|ui| {
            ui.spinner();
            if ui.button(tr("Cancel Request")).clicked() { app.http.cancel(); }
        });
    }
    ui.separator();
    if !app.http.title.is_empty() { ui.label(&app.http.title); }
    if let Some(error) = &app.http.error { error_label(ui, error); }
    if let Some(response) = &app.http.response {
        ui.label(trf("HTTP {status} in {milliseconds} ms; {bytes} bytes retained", &[
            ("status", &response.status), ("milliseconds", &response.elapsed.as_millis()), ("bytes", &response.body.len()),
        ]));
        if response.truncated { ui.label(tr("Response body truncated at the size limit.")); }
        egui::ScrollArea::vertical().id_salt("http_response").show(ui, |ui| {
            ui.collapsing(tr("Response Headers"), |ui| {
                for (name, value) in &response.headers { ui.monospace(format!("{name}: {value}")); }
            });
            ui.label(tr("Response Body"));
            match std::str::from_utf8(&response.body) {
                Ok(text) => {
                    let mut shown: String = text.chars().take(MAX_SHOWN_CHARS).collect();
                    if text.chars().nth(MAX_SHOWN_CHARS).is_some() { ui.label(tr("Response display truncated.")); }
                    ui.add(egui::TextEdit::multiline(&mut shown).code_editor().desired_width(f32::INFINITY).desired_rows(8).interactive(false));
                }
                Err(_) => { ui.label(tr("Binary response body is not displayed.")); }
            }
        });
    }
}

fn error_label(ui: &mut Ui, error: &HttpError) {
    let text = match error {
        HttpError::Invalid(reason) => trf("Invalid HTTP request: {error}", &[("error", reason)]),
        HttpError::Network(reason) => trf("HTTP request failed: {error}", &[("error", reason)]),
        HttpError::TimedOut => tr("HTTP request timed out.").to_string(),
        HttpError::Cancelled => tr("Cancelled").to_string(),
    };
    ui.colored_label(rc_kit::pal(ui).error, text);
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};
    use std::io::{Read, Write};
    use std::time::{Duration, Instant};

    #[test]
    fn http_window_sends_selected_request_displays_response_and_cancels() {
        let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (ready, connected) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            for second in [false, true] {
                let (mut socket, _) = listener.accept().unwrap();
                socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut received = Vec::new();
                let mut bytes = [0; 1024];
                while !received.ends_with(b"\r\n\r\n") {
                    let size = socket.read(&mut bytes).unwrap();
                    assert!(size > 0);
                    received.extend_from_slice(&bytes[..size]);
                }
                assert!(received.starts_with(b"GET /two HTTP/1.1"));
                if second {
                    socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nx").unwrap();
                    ready.send(()).unwrap();
                    assert_eq!(socket.read(&mut bytes).unwrap(), 0);
                } else {
                    socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello").unwrap();
                }
            }
        });
        let path = home.path().join("sample.http");
        std::fs::write(&path, format!("GET http://{address}/one\n### second\nGET http://{address}/two")).unwrap();
        let mut app = crate::screens::test_app(home.path(), true);
        app.open_file(&path);
        app.http.selected = 1;
        let mut first = true;
        let mut harness = Harness::builder().with_size(egui::vec2(1000.0, 650.0)).build_ui_state(
            move |ui, app: &mut DotCraftApp| {
                if std::mem::take(&mut first) {
                    rc_kit::theme::install_fonts(ui.ctx());
                    return;
                }
                app.http.update();
                show(app, ui);
            }, app,
        );
        harness.run_steps(2);
        assert!(!harness.state().http.running());
        assert!(harness.state().http.response.is_none());
        harness.get_by_label(tr("Send Request")).click();
        let start = Instant::now();
        while harness.state().http.response.is_none() && start.elapsed() < Duration::from_secs(5) {
            harness.run_steps(2);
            std::thread::sleep(Duration::from_millis(10));
        }
        let response = harness.state().http.response.as_ref().expect("HTTP response should arrive");
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"hello");
        harness.get_by_label(tr("Response Body"));
        harness.get_by_label(tr("Send Request")).click();
        harness.run_steps(2);
        connected.recv_timeout(Duration::from_secs(5)).unwrap();
        harness.get_by_label(tr("Cancel Request")).click();
        let start = Instant::now();
        while harness.state().http.error.is_none() && start.elapsed() < Duration::from_secs(5) {
            harness.run_steps(2);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(harness.state().http.error, Some(HttpError::Cancelled));
        harness.state_mut().close_workspace();
        assert!(harness.state().http.error.is_none());
        assert!(harness.state().http.requests().is_none());
        server.join().unwrap();
    }
}
