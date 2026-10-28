//! Markdown preview (Rider, VS Code): a `.md` file shows its editor, the rendered page or both
//! side by side, switched from the buttons at the top right; the preview follows the editor's
//! scroll and updates as you type.

use crate::app::DotCraftApp;
use crate::docs::MdView;
use egui::{Rect, Ui, vec2};
use rc_kit::i18n::tr;
use rc_kit::{ico, theme};

/// The editor's and the preview's parts of `rect` for `view`.
pub fn layout(view: MdView, rect: Rect) -> (Option<Rect>, Option<Rect>) {
    match view {
        MdView::Editor => (Some(rect), None),
        MdView::Preview => (None, Some(rect)),
        MdView::Split => {
            let mid = rect.center().x.round();
            (Some(Rect::from_min_max(rect.min, egui::pos2(mid, rect.max.y))), Some(Rect::from_min_max(egui::pos2(mid + 1.0, rect.min.y), rect.max)))
        }
    }
}

/// The rendered page of document `idx` in `rect`. Returns whether it was clicked.
pub fn preview(app: &mut DotCraftApp, ui: &mut Ui, idx: usize, rect: Rect) -> bool {
    let p = rc_kit::pal(ui);
    let DotCraftApp { docs, md_cache, .. } = app;
    let doc = &docs.list[idx];
    let id = doc.editor.id;
    ui.painter().rect_filled(rect, 0.0, p.editor_bg);
    if doc.md_view == Some(MdView::Split) {
        ui.painter().vline(rect.left() - 0.5, rect.y_range(), egui::Stroke::new(1.0, p.island_border));
    }
    let text = doc.editor.buffer.rope().to_string();
    // Relative images (`![](docs/shot.png)`) load from the file's folder.
    let base = doc.path.parent().map(|d| format!("file://{}/", d.display())).unwrap_or_default();
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).id_salt(("md_preview", id)));
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    // Page sizes rather than the IDE's compact ones: headings scale between Body and Heading.
    {
        use egui::{FontId, TextStyle};
        let styles = &mut child.style_mut().text_styles;
        styles.insert(TextStyle::Heading, theme::bold(26.0));
        styles.insert(TextStyle::Body, FontId::proportional(14.0));
        styles.insert(TextStyle::Monospace, FontId::monospace(13.0));
        child.style_mut().spacing.item_spacing.y = 6.0;
    }
    let mut area = egui::ScrollArea::vertical().id_salt(("md_scroll", id)).auto_shrink(false);
    // Side by side, the page follows the editor when the editor scrolls.
    let frac = doc.editor.scroll_fraction();
    let key = egui::Id::new(("md_sync", id));
    let (last, max): (Option<f32>, f32) = ui.data(|d| d.get_temp(key)).unwrap_or((None, 0.0));
    if doc.md_view == Some(MdView::Split) && last.is_some_and(|l| (l - frac).abs() > f32::EPSILON) {
        area = area.vertical_scroll_offset(frac * max);
    }
    let width = rect.width() - 56.0;
    let out = area.show(&mut child, |ui| {
        egui::Frame::new().inner_margin(egui::Margin::symmetric(28, 20)).show(ui, |ui| {
            ui.set_width(width.max(100.0));
            egui_commonmark::CommonMarkViewer::new().default_implicit_uri_scheme(base).max_image_width(Some(width.max(100.0) as usize)).show(ui, md_cache, &text);
        });
    });
    let max = (out.content_size.y - out.inner_rect.height()).max(0.0);
    ui.data_mut(|d| d.insert_temp(key, (Some(frac), max)));
    ui.input(|i| i.pointer.primary_pressed() && i.pointer.interact_pos().is_some_and(|p| rect.contains(p)))
}

/// Editor / Editor and Preview / Preview, floating at the top right of the pane.
pub fn switcher(app: &mut DotCraftApp, ui: &mut Ui, idx: usize, rect: Rect) {
    let Some(current) = app.docs.list[idx].md_view else { return };
    let p = rc_kit::pal(ui);
    let id = app.docs.list[idx].editor.id;
    let size = vec2(3.0 * 26.0 + 2.0 * 2.0 + 8.0, 34.0);
    let at = egui::pos2(rect.right() - size.x - 22.0, rect.top() + 6.0);
    egui::Area::new(egui::Id::new(("md_switch", id, rect.left() as i32))).order(egui::Order::Foreground).fixed_pos(at).show(ui.ctx(), |ui| {
        egui::Frame::new().fill(p.island_bg).stroke(egui::Stroke::new(1.0, p.popup_border)).corner_radius(8.0).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (view, icon, tip) in [(MdView::Editor, ico::EDIT, tr("Editor Only")), (MdView::Split, ico::SPLIT_RIGHT, tr("Editor and Preview")), (MdView::Preview, ico::PREVIEW, tr("Preview Only"))] {
                    let resp = theme::icon_button(ui, icon, true, tip);
                    if view == current {
                        ui.painter().rect_stroke(resp.rect, 6.0, egui::Stroke::new(1.0, p.accent), egui::StrokeKind::Inside);
                    }
                    if resp.clicked() {
                        app.docs.list[idx].md_view = Some(view);
                        app.focus_editor = view != MdView::Preview;
                    }
                }
            });
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_files_open_beside_their_preview() {
        let dir = tempfile::tempdir().unwrap();
        let (md, cs) = (dir.path().join("README.md"), dir.path().join("Program.cs"));
        std::fs::write(&md, "# Hi\n").unwrap();
        std::fs::write(&cs, "class A {}\n").unwrap();
        let mut docs = crate::docs::Docs::new(rc_syntax::ParserPool::new(std::sync::Arc::new(|| {})));
        let i = docs.open(&md).unwrap();
        assert_eq!(docs.list[i].md_view, Some(MdView::Split));
        let j = docs.open(&cs).unwrap();
        assert_eq!(docs.list[j].md_view, None);
        let r = Rect::from_min_size(egui::pos2(0.0, 0.0), vec2(800.0, 600.0));
        let (ed, preview) = layout(MdView::Split, r);
        assert_eq!((ed.unwrap().width(), preview.unwrap().left()), (400.0, 401.0));
        assert_eq!(layout(MdView::Preview, r), (None, Some(r)));
    }
}
