use std::cell::RefCell;
use std::rc::Rc;

use egui::{Key, Shape, pos2, vec2};
use egui_kittest::Harness;
use rc_editor::{Action, EditorConfig, EditorState, EditorView, Hover, Keymap, KeymapKind};
use rc_text::Selection;

const SHOP: &str = include_str!("../../../tests/fixtures/Shop/src/Shop.Core/PriceCalculator.cs");

#[test]
fn shop_quick_doc_shortcut_popup_and_escape() {
    let mut state = EditorState::new(1, Some("PriceCalculator.cs".into()), SHOP);
    let symbol = state
        .buffer
        .rope()
        .byte_to_char(SHOP.find("PriceBreakdown").unwrap());
    state.selection = Selection::point(symbol);
    let summary = SHOP
        .lines()
        .find(|line| line.contains("The price of an order"))
        .unwrap();
    let documentation = summary
        .trim()
        .trim_start_matches("/// <summary>")
        .trim_end_matches("</summary>")
        .to_string();
    let actions = Rc::new(RefCell::new(Vec::new()));
    let recorded = actions.clone();
    let config = EditorConfig::default();
    let keymap = Keymap::new(KeymapKind::RiderMac);
    let mut first = true;
    let mut h = Harness::builder()
        .with_size(vec2(900.0, 700.0))
        .build_ui_state(
            move |ui, state: &mut EditorState| {
                if std::mem::take(&mut first) {
                    rc_kit::theme::install_fonts(ui.ctx());
                    return;
                }
                let rect = ui.ctx().content_rect();
                let mut root = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                let out = EditorView::new(state, &config, &keymap)
                    .focus(true)
                    .show(&mut root);
                recorded.borrow_mut().extend(out.actions);
            },
            state,
        );
    h.run_steps(2);
    h.key_press(Key::F1);
    h.run_steps(1);
    assert!(actions.borrow().contains(&Action::QuickDoc));
    assert_eq!(h.state().caret(), symbol);

    for screen in [
        pos2(100.0, 100.0),
        pos2(880.0, 100.0),
        pos2(100.0, 680.0),
        pos2(880.0, 680.0),
    ] {
        h.state_mut().popups.hover = Some(Hover {
            pos: symbol,
            text: documentation.clone(),
            problem: None,
            screen,
            actions: Vec::new(),
        });
        h.run_steps(2);
        let painted = h
            .output()
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                Shape::Text(text) if text.galley.job.text.contains("The price of an order") => {
                    Some(text)
                }
                _ => None,
            })
            .expect("Shop summary is painted in the documentation popup");
        assert!(
            painted.pos.x >= 0.0 && painted.pos.y >= 0.0,
            "{screen:?}: {:?}",
            painted.pos
        );
        assert!(
            painted.pos.x + painted.galley.rect.width() <= 900.0,
            "{screen:?}: {:?}",
            painted.pos
        );
        assert!(
            painted.pos.y + painted.galley.rect.height() <= 700.0,
            "{screen:?}: {:?}",
            painted.pos
        );
    }

    h.key_press(Key::Escape);
    h.run_steps(1);
    assert!(h.state().popups.hover.is_none());
    assert_eq!(h.state().buffer.rope().to_string(), SHOP);
}
