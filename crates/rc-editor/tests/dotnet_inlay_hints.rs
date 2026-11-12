use egui::{Key, vec2};
use egui_kittest::Harness;
use rc_editor::{EditorConfig, EditorState, EditorView, InlayHint, Keymap, KeymapKind};
use rc_text::Selection;

const CALCULATOR: &str = include_str!("../../../tests/fixtures/Shop/src/Shop.Core/PriceCalculator.cs");

#[test]
fn shop_csharp_inlay_does_not_change_logical_caret_navigation() {
    let config = EditorConfig::default();
    let keymap = Keymap::new(KeymapKind::Rider);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop/src/Shop.Core/PriceCalculator.cs");
    assert!(path.exists());
    let mut state = EditorState::new(1, Some(path), CALCULATOR);
    let pos = CALCULATOR[..CALCULATOR.find("ThrowIfNull(order)").unwrap() + "ThrowIfNull(".len()].chars().count();
    state.selection = Selection::point(pos);
    state.inlays.push(InlayHint { pos, label: "value:".into(), pad_left: true, pad_right: true });
    let mut first = true;
    let mut harness = Harness::builder().with_size(vec2(800.0, 600.0)).build_ui_state(
        move |ui, state: &mut EditorState| {
            if std::mem::take(&mut first) {
                rc_kit::theme::install_fonts(ui.ctx());
                return;
            }
            let rect = ui.ctx().content_rect();
            let mut root = ui.new_child(egui::UiBuilder::new().max_rect(rect));
            EditorView::new(state, &config, &keymap).focus(true).show(&mut root);
        },
        state,
    );
    harness.run_steps(2);
    harness.key_press(Key::ArrowRight);
    harness.run_steps(1);
    assert_eq!(harness.state().caret(), pos + 1);
    assert_eq!(harness.state().buffer.rope().to_string(), CALCULATOR);
}
