//! Live localization of the Shop solution window.

use crate::screens::{HOME_LOCK, fixtures, harness, test_app};
use egui::vec2;
use egui_kittest::kittest::Queryable;
use rc_kit::i18n::{self, Lang};

#[test]
fn language_switch_relabels_open_shop_workspace() {
    let _lock = HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = tempfile::tempdir().unwrap();
    i18n::set_lang(Lang::En);
    let mut app = test_app(home.path(), true);
    app.settings.roslyn_path = Some("/nonexistent".into());
    app.open_workspace(&fixtures().join("Shop/Shop.sln"));
    let mut h = harness(app, vec2(900.0, 600.0));
    h.get_by_label("Explorer");

    for (locale, title) in [("pt-BR", "Explorador"), ("es-ES", "Explorador"), ("en-US", "Explorer")] {
        let mut settings = h.state().settings.clone();
        settings.language = Some(locale.into());
        h.state_mut().apply_settings(&egui::Context::default(), settings);
        h.run_steps(2);
        h.get_by_label(title);
        assert_eq!(h.state().settings.language.as_deref(), Some(locale));
        assert!(h.state().workspace.is_some());
    }
    i18n::set_lang(Lang::En);
}
