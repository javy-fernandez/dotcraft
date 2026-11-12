//! Active keymap shortcuts in the Shop workspace, including a settings change in the same session.

use crate::screens::{HOME_LOCK, fixtures, harness, test_app};
use egui::{Event, vec2};
use rc_core::settings::KeymapChoice;
use rc_editor::Action;

fn press(harness: &mut egui_kittest::Harness<'static, crate::app::DotCraftApp>, action: Action) {
    let chord = harness.state().keymap.chord(action).unwrap();
    harness.event(Event::Key {
        key: chord.key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: chord.mods,
    });
    harness.run_steps(2);
}

#[test]
fn rider_and_visual_studio_shortcuts_follow_settings_in_shop() {
    let _lock = HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = tempfile::tempdir().unwrap();
    let mut app = test_app(home.path(), true);
    app.settings.roslyn_path = Some("/nonexistent".into());
    app.settings.keymap = KeymapChoice::Rider;
    app.open_workspace(&fixtures().join("Shop/Shop.sln"));
    let mut h = harness(app, vec2(900.0, 600.0));
    assert_eq!(h.state().keymap.shortcut_text(Action::GotoFile).as_deref(), Some(if cfg!(target_os = "macos") { "⇧⌘O" } else { "Ctrl+Shift+N" }));
    press(&mut h, Action::GotoFile);
    assert_eq!(h.state().search_everywhere.as_ref().unwrap().tab, crate::search::SeTab::Files);

    h.state_mut().search_everywhere = None;
    let mut settings = h.state().settings.clone();
    settings.keymap = KeymapChoice::VisualStudio;
    h.state_mut().apply_settings(&egui::Context::default(), settings);
    h.run_steps(2);
    assert_eq!(h.state().keymap.shortcut_text(Action::GotoFile).as_deref(), Some("Ctrl+Shift+T"));
    press(&mut h, Action::GotoFile);
    assert_eq!(h.state().search_everywhere.as_ref().unwrap().tab, crate::search::SeTab::Files);
}
