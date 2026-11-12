use egui::{Key, vec2};
use egui_kittest::Harness;
use rc_editor::{EditorConfig, EditorState, EditorView, Keymap, KeymapKind, Signature, SignatureHelp};
use rc_text::{Rope, Selection};

const SHOP: &str = include_str!("../../../tests/fixtures/Shop/src/Shop.Core/PriceCalculator.cs");

fn offset(rope: &Rope, text: &str) -> usize {
    rope.byte_to_char(rope.to_string().find(text).unwrap())
}

fn round_help(anchor: usize) -> SignatureHelp {
    let label = "decimal Math.Round(decimal d, int decimals, MidpointRounding mode)";
    let params = ["decimal d", "int decimals", "MidpointRounding mode"]
        .map(|name| {
            let start = label.find(name).unwrap();
            (start, start + name.len())
        })
        .to_vec();
    SignatureHelp { anchor, active: 0, signatures: vec![Signature { label: label.into(), params, active_param: Some(0), doc: None }] }
}

#[test]
fn shop_round_signature_tracks_caret_through_editor_events() {
    let mut state = EditorState::new(1, Some("PriceCalculator.cs".into()), SHOP);
    let rope = state.buffer.rope();
    let anchor = offset(rope, "Math.Round(") + "Math.Round".chars().count();
    let comma = offset(rope, "TaxRate, 2") + "TaxRate".chars().count();
    state.selection = Selection::point(comma);
    state.popups.signature = Some(round_help(anchor));
    let config = EditorConfig::default();
    let keymap = Keymap::new(KeymapKind::RiderMac);
    let mut first = true;
    let mut h = Harness::builder().with_size(vec2(900.0, 500.0)).build_ui_state(
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
    h.run_steps(2);
    assert_eq!(h.state().popups.signature.as_ref().unwrap().signatures[0].active_param, Some(0));
    h.key_press(Key::ArrowRight);
    h.run_steps(1);
    assert_eq!(h.state().popups.signature.as_ref().unwrap().signatures[0].active_param, Some(1));
    h.key_press(Key::ArrowRight);
    h.run_steps(1);
    assert_eq!(h.state().popups.signature.as_ref().unwrap().signatures[0].active_param, Some(1));
    assert_eq!(h.state().buffer.rope().to_string(), SHOP);
}

#[test]
fn shop_round_signature_ignores_nested_and_quoted_commas() {
    let source = SHOP.replace("taxable * TaxRate, 2", "Math.Max(taxable, 0m) * TaxRate, 2");
    let rope = Rope::from_str(&source);
    let anchor = offset(&rope, "Math.Round(") + "Math.Round".len();
    let help = round_help(anchor);
    let nested_comma = offset(&rope, "taxable, 0m") + "taxable,".len();
    assert_eq!(help.argument_at(&rope, nested_comma), Some(0));
    let outer_comma = offset(&rope, "TaxRate, 2") + "TaxRate,".len();
    assert_eq!(help.argument_at(&rope, outer_comma), Some(1));
    let third = offset(&rope, "2, MidpointRounding") + "2,".len();
    assert_eq!(help.argument_at(&rope, third), Some(2));

    let quoted = Rope::from_str("Log(\"first, second\", Math.Max(1, 2), 3)");
    let quoted_help = round_help(3);
    assert_eq!(quoted_help.argument_at(&quoted, quoted.len_chars() - 1), Some(2));
    assert_eq!(quoted_help.argument_at(&quoted, quoted.len_chars()), None);

    let commented = Rope::from_str("Log(1 /* ignored, comma */ , 2)");
    assert_eq!(quoted_help.argument_at(&commented, commented.len_chars() - 1), Some(1));
}
