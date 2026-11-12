use egui::{Key, Modifiers, vec2};
use egui_kittest::Harness;
use rc_editor::{Diagnostic, EditorConfig, EditorState, EditorView, InlayHint, Keymap, KeymapKind, Severity};
use rc_text::Selection;

const CALCULATOR: &str = include_str!("../../../tests/fixtures/Shop/src/Shop.Core/PriceCalculator.cs");

fn at(needle: &str) -> usize {
    CALCULATOR[..CALCULATOR.find(needle).expect("fixture marker")].chars().count()
}

#[test]
fn shop_fixture_undo_redo_restores_caret_and_decorations_then_invalidates_redo() {
    let config = EditorConfig::default();
    let keymap = Keymap::new(KeymapKind::Rider);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/Shop/src/Shop.Core/PriceCalculator.cs");
    assert!(path.exists());
    let mut state = EditorState::new(1, Some(path), CALCULATOR);
    let insert_at = at("var discount = DiscountFor");
    let anchor = at("DiscountFor(order, subtotal)");
    state.selection = Selection::point(insert_at);
    state.set_diagnostics(vec![Diagnostic {
        start: anchor, end: anchor + "DiscountFor".len(), severity: Severity::Warning,
        message: "Fixture marker".into(), code: None, unnecessary: false, deprecated: false,
    }]);
    state.inlays.push(InlayHint { pos: anchor, label: "order:".into(), pad_left: false, pad_right: true });
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
    harness.event(egui::Event::Text(" ".into()));
    harness.run_steps(1);
    let edited = harness.state().buffer.rope().to_string();
    assert_eq!(harness.state().caret(), insert_at + 1);
    assert_eq!(harness.state().diagnostics[0].start, anchor + 1);
    assert_eq!(harness.state().inlays[0].pos, anchor + 1);
    harness.state_mut().search_matches.push((anchor + 1, anchor + 4));
    harness.state_mut().current_match = Some(0);
    harness.state_mut().highlights.push((anchor + 1, anchor + 4, false));

    let ctrl = Modifiers { ctrl: true, ..Default::default() };
    let ctrl_shift = Modifiers { ctrl: true, shift: true, ..Default::default() };
    harness.key_press_modifiers(ctrl, Key::Z);
    harness.run_steps(1);
    assert_eq!(harness.state().buffer.rope().to_string(), CALCULATOR);
    assert_eq!(harness.state().caret(), insert_at);
    assert_eq!(harness.state().diagnostics[0].start, anchor);
    assert_eq!(harness.state().inlays[0].pos, anchor);
    assert!(harness.state().search_matches.is_empty());
    assert!(harness.state().current_match.is_none());
    assert!(harness.state().highlights.is_empty());

    harness.key_press_modifiers(ctrl_shift, Key::Z);
    harness.run_steps(1);
    assert_eq!(harness.state().buffer.rope().to_string(), edited);
    assert_eq!(harness.state().caret(), insert_at + 1);
    assert_eq!(harness.state().diagnostics[0].start, anchor + 1);
    assert_eq!(harness.state().inlays[0].pos, anchor + 1);

    harness.key_press_modifiers(ctrl, Key::Z);
    harness.run_steps(1);
    harness.state_mut().selection = Selection::point(at("var taxable = subtotal"));
    harness.event(egui::Event::Text(" ".into()));
    harness.run_steps(1);
    assert!(!harness.state().buffer.can_redo());
    let divergent = harness.state().buffer.rope().to_string();
    harness.key_press_modifiers(ctrl_shift, Key::Z);
    harness.run_steps(1);
    assert_eq!(harness.state().buffer.rope().to_string(), divergent);
}
