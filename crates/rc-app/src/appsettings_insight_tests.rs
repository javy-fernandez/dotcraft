//! `appsettings*.json` insight through the whole app (offscreen harness): schema completion
//! while typing, problems, hover and navigation to the bound options class.

use crate::app::DotCraftApp;
use crate::screens::{HOME_LOCK, harness, test_app};
use egui::vec2;
use egui_kittest::Harness;
use std::path::{Path, PathBuf};

pub const SETTINGS: &str = r#"{
  "Logging": {
    "LogLevel": {
      "Default": "Information",
      "Microsoft.AspNetCore": "Warn"
    }
  },
  "AllowedHosts": "*",
  "Smtp": {
    "Host": "smtp.example.com",
    "Port": 587
  }
}
"#;

pub const OPTIONS: &str = r#"namespace Shop.Api.Options;

/// <summary>Outgoing mail settings.</summary>
public sealed class SmtpOptions
{
    public const string SectionName = "Smtp";

    /// <summary>The mail server's host name.</summary>
    public string Host { get; set; } = "";

    /// <summary>The server's port (25, 465 or 587).</summary>
    public int Port { get; set; } = 25;

    /// <summary>Connect with TLS.</summary>
    public bool UseSsl { get; set; }

    public DeliveryMode Mode { get; set; }
}

public enum DeliveryMode { Network, PickupDirectory }
"#;

pub const PROGRAM: &str = r#"using Shop.Api.Options;

var builder = WebApplication.CreateBuilder(args);
builder.Services.Configure<SmtpOptions>(builder.Configuration.GetSection(SmtpOptions.SectionName));
var app = builder.Build();
app.Run();
"#;

/// A folder with an API's settings, an options class and its registration.
pub fn settings_workspace() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::Builder::new().prefix("dotcraft-appsettings").tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap().join("Shop");
    let api = root.join("src/Shop.Api");
    std::fs::create_dir_all(api.join("Options")).unwrap();
    std::fs::write(api.join("appsettings.json"), SETTINGS).unwrap();
    std::fs::write(api.join("Options/SmtpOptions.cs"), OPTIONS).unwrap();
    std::fs::write(api.join("Program.cs"), PROGRAM).unwrap();
    (tmp, root)
}

pub fn app_with(root: &Path, home: &Path, dark: bool) -> Harness<'static, DotCraftApp> {
    let mut app = test_app(home, dark);
    app.settings.roslyn_path = Some("/nonexistent".into());
    app.open_workspace(root);
    app.open_file(&root.join("src/Shop.Api/appsettings.json"));
    let mut h = harness(app, vec2(1280.0, 800.0));
    settle(&mut h, "the options scan", |a| !a.appsettings.index_for_test().bindings.is_empty());
    h
}

/// Runs frames until `done` (and the scan is over), at most 20 s.
pub fn settle(h: &mut Harness<'static, DotCraftApp>, what: &str, mut done: impl FnMut(&DotCraftApp) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        h.run_steps(1);
        if h.state().appsettings.idle_for_test() && done(h.state()) {
            h.run_steps(2);
            return;
        }
        assert!(std::time::Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

fn labels(app: &DotCraftApp) -> Vec<String> {
    let c = app.docs.active_doc().unwrap().editor.popups.completion.as_ref().unwrap();
    c.filtered.iter().map(|&i| c.items[i].label.clone()).collect()
}

fn diagnostics(app: &DotCraftApp) -> Vec<String> {
    app.docs.active_doc().unwrap().editor.diagnostics.iter().map(|d| d.message.clone()).collect()
}

use crate::package_insight_tests::{caret_at, text, type_text};

#[test]
fn keys_complete_from_the_schema_and_the_options_class() {
    let _lock = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    let (_tmp, root) = settings_workspace();
    let mut h = app_with(&root, home.path(), true);
    // A wrong log level is reported.
    settle(&mut h, "problems", |a| !a.docs.active_doc().unwrap().editor.diagnostics.is_empty());
    let d = diagnostics(h.state());
    assert!(d.len() == 1 && d[0].contains("Valid values") && d[0].contains("Information, Warning"), "{d:?}");
    let file = root.join("src/Shop.Api/appsettings.json");
    assert!(h.state().insight.problems.get(&file).is_some_and(|p| p.len() == 1));
    // A new key of the bound class: the quote opens the list, typing filters it.
    caret_at(&mut h, "\"Port\": 587", true);
    type_text(&mut h, ",\n\"");
    settle(&mut h, "completion", |a| a.docs.active_doc().unwrap().editor.popups.completion.is_some());
    assert_eq!(labels(h.state()), ["UseSsl", "Mode"]);
    type_text(&mut h, "Mo");
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert!(text(h.state()).contains("\"Mode\": \"\""), "{}", text(h.state()));
    // The value list opens by itself after the key.
    settle(&mut h, "values", |a| a.docs.active_doc().unwrap().editor.popups.completion.is_some());
    assert_eq!(labels(h.state()), ["Network", "PickupDirectory"]);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert!(text(h.state()).contains("\"Mode\": \"Network\""), "{}", text(h.state()));
    // Root keys from the schema.
    caret_at(&mut h, "\"AllowedHosts\": \"*\",", true);
    type_text(&mut h, "\n\"Kes");
    settle(&mut h, "root keys", |a| a.docs.active_doc().unwrap().editor.popups.completion.is_some());
    assert_eq!(labels(h.state())[0], "Kestrel");
    let c = h.state().docs.active_doc().unwrap().editor.popups.completion.clone().unwrap();
    assert!(c.doc.is_some_and(|d| d.contains("Kestrel")));
}

#[test]
fn keys_lead_to_their_options_class() {
    let _lock = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    let (_tmp, root) = settings_workspace();
    let mut h = app_with(&root, home.path(), true);
    caret_at(&mut h, "\"Port", false);
    let pos = h.state().docs.active_doc().unwrap().editor.caret() + 2;
    {
        let app = h.state_mut();
        let doc = &mut app.docs.list[app.docs.active.unwrap()];
        app.appsettings.on_editor_events(doc, &[rc_editor::EditorEvent::HoverAt { pos, screen: egui::pos2(300.0, 300.0) }]);
    }
    let hover = h.state().docs.active_doc().unwrap().editor.popups.hover.clone().unwrap();
    assert!(hover.text.contains("`SmtpOptions.Port`") && hover.text.contains("The server's port"), "{}", hover.text);
    assert_eq!(hover.actions.len(), 1);
    // ⌘+click opens the property.
    let outputs = {
        let app = h.state_mut();
        let doc = &mut app.docs.list[app.docs.active.unwrap()];
        app.appsettings.on_editor_events(doc, &[rc_editor::EditorEvent::NavigateAt(pos)])
    };
    h.state_mut().handle_appsettings(outputs);
    h.run_steps(2);
    let doc = h.state().docs.active_doc().unwrap();
    assert_eq!(doc.path, root.join("src/Shop.Api/Options/SmtpOptions.cs"));
    assert_eq!(doc.editor.caret_line_col(), (11, 15));
}
