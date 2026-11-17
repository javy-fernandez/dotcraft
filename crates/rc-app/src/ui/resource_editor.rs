//! The `.resx` tab's resource grid (Rider's Resources editor): a key column, one value column per
//! culture file and a comment column. Only the rows on screen are laid out, so files with
//! thousands of keys scroll smoothly; a click edits a cell in place, and a missing translation is
//! a red cell you type into.

use egui::{FontId, Key, Modifiers, Rect, RichText, Sense, Ui, pos2, vec2};
use egui_extras::{Column as TableColumn, TableBuilder};
use rc_kit::i18n::{tr, trf};
use rc_kit::{ico, theme};

use crate::app::DotCraftApp;
use crate::resource_editor::{Column, ResxEditor, ResxView};
use crate::ui::kit;

const ROW_H: f32 = 26.0;
const HEADER_H: f32 = 28.0;

/// What a right-click on a cell asked for.
enum MenuAction {
    RemoveKey(usize),
    RemoveTranslation(usize, usize),
    AddKey,
}

/// The neutral file's column title, or the culture code (`es`, `fr-CA`).
pub fn culture_label(culture: &Option<String>) -> String {
    culture.clone().unwrap_or_else(|| tr("Neutral").to_string())
}

/// The grid of document `idx` in `rect`. Returns whether it was clicked (a split pane becomes the
/// active one).
pub fn show(app: &mut DotCraftApp, ui: &mut Ui, idx: usize, rect: Rect) -> bool {
    let p = rc_kit::pal(ui);
    ui.painter().rect_filled(rect, 0.0, p.editor_bg);
    let doc = &mut app.docs.list[idx];
    let id = doc.editor.id;
    let path = doc.path.clone();
    let Some(resx) = doc.resx.as_deref_mut() else { return false };
    resx.update();
    let clicked = ui.input(|i| i.pointer.primary_pressed() && i.pointer.interact_pos().is_some_and(|pos| rect.contains(pos)));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).id_salt(("resx_grid", id)).layout(egui::Layout::top_down(egui::Align::Min)));
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    let ui = &mut child;
    let mut save_now = false;
    let mut edited = false;
    toolbar(ui, resx);
    if resx.disk.is_some() {
        conflict_banner(ui, resx, &mut save_now);
    }
    if let Some(error) = resx.error.clone() {
        kit::content(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                kit::message(ui, &error, true);
                if kit::button(ui, Some(ico::REFRESH), tr("Refresh"), !resx.loading).clicked() {
                    resx.load(&path, !resx.dirty());
                }
            });
        });
    }
    if resx.group.is_none() {
        if resx.loading {
            kit::content(ui, |ui| kit::note(ui, tr("Loading resources…")));
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
        return clicked;
    }
    if resx.group.as_ref().is_some_and(|group| group.rows.is_empty()) {
        kit::content(ui, |ui| kit::note(ui, tr("No string resources yet. Add a key to start.")));
        return clicked;
    }
    edited |= grid(ui, resx, id);
    if edited {
        app.docs.list[idx].preview = false;
    }
    if save_now
        && let Err(error) = app.docs.save(idx)
    {
        app.notify(error, true);
    }
    clicked
}

fn toolbar(ui: &mut Ui, resx: &mut ResxEditor) {
    let Some(group) = &resx.group else { return };
    let labels: Vec<String> = group.files.iter().map(|file| trf("Comments: {culture}", &[("culture", &culture_label(&file.culture))])).collect();
    let typed = group.files.iter().map(|file| file.typed_entries()).max().unwrap_or(0);
    let keys = group.rows.len();
    let selected = resx.selected.filter(|&row| row < keys);
    let comment_column = resx.comment_column.min(labels.len().saturating_sub(1));
    let missing: usize = resx.visible().1.iter().sum();
    // The Resources / XML switcher floats at the top right: the toolbar wraps before it.
    let area = ui.available_rect_before_wrap();
    let mut bar = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(area.min, pos2(area.right() - 110.0, area.bottom()))).layout(egui::Layout::top_down(egui::Align::Min)));
    kit::toolbar(&mut bar, |ui| {
        if kit::button(ui, Some(ico::ADD), tr("Add Key"), true).clicked() {
            resx.add_key();
        }
        if kit::button(ui, Some(ico::REMOVE), tr("Remove Key"), selected.is_some()).clicked()
            && let Some(row) = selected
        {
            resx.remove_key(row);
        }
        theme::toolbar_separator(ui);
        if let Some(pick) = kit::dropdown(ui, "resx_comment_culture", &labels[comment_column], &labels, true) {
            resx.comment_column = pick;
            resx.edit = None;
        }
        kit::search(ui, &mut resx.filter, tr("Filter keys and values"), 200.0);
        let p = rc_kit::pal(ui);
        let note = |ui: &mut Ui, text: String| ui.add(egui::Label::new(RichText::new(text).size(12.0).color(p.muted)).wrap_mode(egui::TextWrapMode::Extend));
        note(ui, trf("{keys} keys, {missing} missing translations", &[("keys", &keys), ("missing", &missing)]));
        if typed > 0 {
            note(ui, trf("Non-string resources: {count} (XML view)", &[("count", &typed)]));
        }
    });
    let used = bar.min_rect();
    ui.allocate_rect(Rect::from_min_max(area.min, pos2(area.right(), used.bottom())), Sense::hover());
}

fn conflict_banner(ui: &mut Ui, resx: &mut ResxEditor, save_now: &mut bool) {
    let p = rc_kit::pal(ui);
    let (banner, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
    ui.painter().rect_filled(banner, 0.0, theme::blend(p.island_bg, p.warn.gamma_multiply(0.18)));
    let mut b = ui.new_child(egui::UiBuilder::new().max_rect(banner.shrink2(vec2(12.0, 4.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    rc_kit::icons::image(&mut b, ico::WARNING, 16.0);
    b.label(tr("The resource files were changed on disk. Keep your version or load the one on disk?"));
    if kit::button(&mut b, None, tr("Load from Disk"), true).clicked() {
        resx.take_disk();
    }
    if kit::button(&mut b, None, tr("Keep Mine"), true).clicked() && resx.keep_mine() {
        *save_now = true;
    }
}

/// The table. Returns whether a cell changed.
fn grid(ui: &mut Ui, resx: &mut ResxEditor, id: u64) -> bool {
    let p = rc_kit::pal(ui);
    let problems = resx.key_problems().clone();
    let (rows, missing) = {
        let (rows, missing) = resx.visible();
        (rows.to_vec(), missing.to_vec())
    };
    let Some(group) = &resx.group else { return false };
    let cultures: Vec<String> = group.files.iter().map(|file| culture_label(&file.culture)).collect();
    let comment_column = resx.comment_column.min(cultures.len().saturating_sub(1));
    let editing = resx.edit.as_ref().map(|edit| (edit.row, edit.column));
    let selected = resx.selected;
    let scroll_to = resx.scroll_to.take().and_then(|row| rows.iter().position(|&r| r == row));
    let mut clicked_cell = None;
    let mut menu = None;
    let mut edit_rect = None;
    let available = ui.available_rect_before_wrap();
    // Key and comment columns, then the cultures share the rest (scrolling sideways when many).
    let spacing = ui.spacing().item_spacing.x * (cultures.len() + 2) as f32;
    let value_width = ((available.width() - spacing - 2.0 * 200.0) / cultures.len() as f32).clamp(140.0, 420.0);
    let font = FontId::proportional(12.5);
    let comment_title = trf("Comment ({culture})", &[("culture", &cultures[comment_column])]);
    let height = available.height().max(40.0);
    let comment_width = (available.width() - spacing - 200.0 - value_width * cultures.len() as f32 - 2.0).max(160.0);
    egui::ScrollArea::horizontal().id_salt(("resx_horizontal", id)).auto_shrink(false).show(ui, |ui| {
        let mut table = TableBuilder::new(ui)
            .id_salt(("resx_table", id))
            .striped(true)
            .resizable(true)
            .sense(Sense::click())
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(TableColumn::initial(200.0).at_least(80.0).clip(true))
            .columns(TableColumn::initial(value_width).at_least(80.0).clip(true), cultures.len())
            .column(TableColumn::initial(comment_width).at_least(120.0).clip(true))
            .min_scrolled_height(0.0)
            .max_scroll_height(height)
            .auto_shrink(false);
        if let Some(row) = scroll_to {
            table = table.scroll_to_row(row, Some(egui::Align::Center));
        }
        let header_cell = |ui: &mut Ui, title: &str, secondary: Option<String>| {
            ui.painter().rect_filled(ui.max_rect().expand2(vec2(4.0, 0.0)), 0.0, p.header_bg);
            ui.add_space(6.0);
            ui.label(RichText::new(title).font(theme::bold(12.0)).color(p.text));
            if let Some(secondary) = secondary {
                ui.label(RichText::new(secondary).size(11.5).color(p.error));
            }
        };
        table
            .header(HEADER_H, |mut header| {
                header.col(|ui| header_cell(ui, tr("Key"), None));
                for (column, culture) in cultures.iter().enumerate() {
                    let count = missing.get(column).copied().unwrap_or(0);
                    header.col(|ui| header_cell(ui, culture, (count > 0).then(|| trf("{count} missing", &[("count", &count)]))));
                }
                header.col(|ui| header_cell(ui, &comment_title, None));
            })
            .body(|body| {
                body.rows(ROW_H, rows.len(), |mut row| {
                    let index = rows[row.index()];
                    let data = &group.rows[index];
                    row.set_selected(selected == Some(index));
                    let mut cell = |row: &mut egui_extras::TableRow<'_, '_>, column: Column, text: Option<&str>, problem: bool| {
                        let missing = text.is_none() && matches!(column, Column::Culture(_));
                        let (_, resp) = row.col(|ui| {
                            let rect = ui.max_rect();
                            if missing {
                                ui.painter().rect_filled(rect.expand2(vec2(4.0, 0.0)), 0.0, p.error.gamma_multiply(0.12));
                            }
                            ui.add_space(6.0);
                            if let Some(text) = text {
                                let line = text.replace("\r\n", "\n").replace('\n', " ↵ ");
                                let color = if problem { p.error } else if column == Column::Comment { p.muted } else { p.text };
                                ui.add(egui::Label::new(RichText::new(line).font(font.clone()).color(color)).truncate().selectable(false));
                            }
                        });
                        // The table's first (sizing) pass lays cells out without a position.
                        if editing == Some((index, column)) && resp.rect.is_finite() {
                            edit_rect = Some(resp.rect);
                        }
                        if resp.clicked() {
                            clicked_cell = Some((index, column));
                        }
                        let accessible = match column {
                            Column::Key => data.name.clone(),
                            Column::Culture(c) => format!("{} {}", data.name, cultures[c]),
                            Column::Comment => format!("{} {}", data.name, comment_title),
                        };
                        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &accessible));
                        resp.context_menu(|ui| {
                            ui.set_min_width(200.0);
                            if theme::menu_item(ui, Some(ico::ADD), tr("Add Key"), false).clicked() {
                                menu = Some(MenuAction::AddKey);
                                ui.close();
                            }
                            if let Column::Culture(c) = column
                                && text.is_some()
                                && theme::menu_item(ui, Some(ico::DELETE), &trf("Remove {culture} Translation", &[("culture", &cultures[c])]), false).clicked()
                            {
                                menu = Some(MenuAction::RemoveTranslation(index, c));
                                ui.close();
                            }
                            if theme::menu_item(ui, Some(ico::REMOVE), tr("Remove Key"), false).clicked() {
                                menu = Some(MenuAction::RemoveKey(index));
                                ui.close();
                            }
                        });
                    };
                    cell(&mut row, Column::Key, Some(&data.name), problems.contains(&index));
                    for (column, value) in data.cells.iter().enumerate() {
                        cell(&mut row, Column::Culture(column), value.as_ref().map(|value| value.value.as_str()), false);
                    }
                    let comment = data.cells.get(comment_column).and_then(|cell| cell.as_ref()).map(|cell| cell.comment.as_deref().unwrap_or(""));
                    cell(&mut row, Column::Comment, comment, false);
                });
            });
    });
    let mut changed = false;
    match menu {
        Some(MenuAction::AddKey) => {
            resx.add_key();
            changed = true;
        }
        Some(MenuAction::RemoveKey(row)) => {
            resx.remove_key(row);
            changed = true;
        }
        Some(MenuAction::RemoveTranslation(row, column)) => {
            resx.remove_translation(row, column);
            changed = true;
        }
        None => {}
    }
    if let Some((row, column)) = clicked_cell {
        resx.selected = Some(row);
        if editing != Some((row, column)) {
            resx.start_edit(row, column);
        }
    }
    changed | edit_field(ui, resx, edit_rect, id)
}

/// The in-place editor over the edited cell: grows for multi-line text. Enter confirms,
/// Shift+Enter starts a new line, Tab moves to the next column, Escape cancels.
fn edit_field(ui: &mut Ui, resx: &mut ResxEditor, rect: Option<Rect>, id: u64) -> bool {
    let p = rc_kit::pal(ui);
    let Some(edit) = resx.edit.as_mut() else { return false };
    // Scrolled out of view (or a layout pass): the typed text is already in the row, and the
    // field ends the edit when it shows again without the focus.
    let Some(rect) = rect else { return false };
    let text_id = egui::Id::new(("resx_cell_edit", id));
    let focused = ui.memory(|m| m.has_focus(text_id));
    let (enter, escape, tab) = if focused {
        ui.input_mut(|i| (i.consume_key(Modifiers::NONE, Key::Enter), i.consume_key(Modifiers::NONE, Key::Escape), i.consume_key(Modifiers::NONE, Key::Tab)))
    } else {
        (false, false, false)
    };
    if escape {
        resx.cancel_edit();
        return true;
    }
    let lines = edit.text.split('\n').count().max(1);
    let height = (lines as f32 * 17.0 + 8.0).max(rect.height());
    let mut changed = false;
    let mut lost_focus = false;
    let (row, column) = (edit.row, edit.column);
    egui::Area::new(egui::Id::new(("resx_cell_area", id))).order(egui::Order::Foreground).fixed_pos(rect.min).show(ui.ctx(), |ui| {
        egui::Frame::new().fill(p.editor_bg).stroke(egui::Stroke::new(1.0, p.accent)).inner_margin(egui::Margin::symmetric(5, 4)).show(ui, |ui| {
            let mut output = egui::TextEdit::multiline(&mut edit.text)
                .id(text_id)
                .font(FontId::proportional(12.5))
                .frame(egui::Frame::NONE)
                .desired_width((rect.width() - 10.0).max(60.0))
                .desired_rows(lines)
                .min_size(vec2(0.0, height - 8.0))
                .show(ui);
            if edit.focus {
                output.response.request_focus();
                if edit.select_all {
                    let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(edit.text.chars().count()));
                    output.state.cursor.set_char_range(Some(all));
                    output.state.store(ui.ctx(), text_id);
                }
                edit.focus = false;
            } else if !output.response.has_focus() {
                lost_focus = true;
            }
            changed = output.response.changed();
        });
    });
    if changed {
        resx.apply_edit();
    }
    if enter || lost_focus {
        resx.edit = None;
    } else if tab {
        let columns = resx.group.as_ref().map_or(0, |group| group.files.len());
        let next = match column {
            Column::Key => Column::Culture(0),
            Column::Culture(c) if c + 1 < columns => Column::Culture(c + 1),
            Column::Culture(_) => Column::Comment,
            Column::Comment => Column::Key,
        };
        resx.edit = None;
        resx.start_edit(row, next);
    }
    changed
}

/// Resources / XML, floating at the top right of the pane (like the Markdown preview switch).
/// Switching saves the side being left, so the grid and the text never disagree.
pub fn switcher(app: &mut DotCraftApp, ui: &mut Ui, idx: usize, rect: Rect) {
    let Some(current) = app.docs.list[idx].resx.as_ref().map(|resx| resx.view) else { return };
    let p = rc_kit::pal(ui);
    let id = app.docs.list[idx].editor.id;
    let size = vec2(2.0 * 26.0 + 2.0 + 8.0, 34.0);
    let at = pos2(rect.right() - size.x - 22.0, rect.top() + 4.0);
    let mut pick = None;
    egui::Area::new(egui::Id::new(("resx_switch", id, rect.left() as i32))).order(egui::Order::Foreground).fixed_pos(at).show(ui.ctx(), |ui| {
        egui::Frame::new().fill(p.island_bg).stroke(egui::Stroke::new(1.0, p.popup_border)).corner_radius(8.0).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (view, icon, tip) in [(ResxView::Grid, ico::DATA_TABLE, tr("Resources")), (ResxView::Xml, ico::FILE_XML, tr("XML Text"))] {
                    let resp = theme::icon_button(ui, icon, true, tip);
                    if view == current {
                        ui.painter().rect_stroke(resp.rect, 6.0, egui::Stroke::new(1.0, p.accent), egui::StrokeKind::Inside);
                    }
                    if resp.clicked() && view != current {
                        pick = Some(view);
                    }
                }
            });
        });
    });
    if let Some(view) = pick {
        switch_view(app, idx, view);
    }
}

/// Shows the grid or the XML of `.resx` document `idx`, saving the other side's edits first.
pub fn switch_view(app: &mut DotCraftApp, idx: usize, view: ResxView) {
    if app.docs.list[idx].is_modified()
        && let Err(error) = app.docs.save(idx)
    {
        app.notify(error, true);
        return;
    }
    let doc = &mut app.docs.list[idx];
    let path = doc.path.clone();
    if let Some(resx) = doc.resx.as_deref_mut() {
        resx.view = view;
        resx.edit = None;
        if view == ResxView::Grid && resx.group.is_none() && !resx.loading {
            resx.load(&path, true);
        }
    }
    app.focus_editor = view == ResxView::Xml;
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;

    #[test]
    fn neutral_column_has_an_explicit_label() {
        assert_eq!(culture_label(&None), tr("Neutral"));
        assert_eq!(culture_label(&Some("fr-FR".into())), "fr-FR");
    }

    fn pump(harness: &mut egui_kittest::Harness<'_, DotCraftApp>, ready: impl Fn(&DotCraftApp) -> bool) {
        for _ in 0..200 {
            harness.run_steps(2);
            if ready(harness.state()) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the resource grid did not get ready");
    }

    #[test]
    fn opening_resx_shows_the_grid_and_saving_writes_every_culture_and_the_designer() {
        let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let neutral = workspace.path().join("Messages.resx");
        let french = workspace.path().join("Messages.fr.resx");
        std::fs::write(&neutral, "<root>\n  <data name=\"Hello\" xml:space=\"preserve\">\n    <value>Hello</value>\n  </data>\n</root>\n").unwrap();
        std::fs::write(&french, "<root>\n</root>\n").unwrap();
        let designer = workspace.path().join("Messages.Designer.cs");
        std::fs::write(&designer, "// StronglyTypedResourceBuilder\nnamespace App {\n    internal class Messages {\n        \n        /// <summary>\n        ///   Culture.\n        /// </summary>\n        internal static global::System.Globalization.CultureInfo Culture {\n            get {\n                return resourceCulture;\n            }\n        }\n        \n        /// <summary>\n        ///   Looks up a localized string similar to Hello.\n        /// </summary>\n        internal static string Hello {\n            get {\n                return ResourceManager.GetString(\"Hello\", resourceCulture);\n            }\n        }\n    }\n}\n").unwrap();
        let mut app = crate::screens::test_app(home.path(), true);
        app.open_workspace(workspace.path());
        app.open_file(&neutral);
        let mut harness = crate::screens::harness(app, egui::vec2(1200.0, 650.0));
        pump(&mut harness, |app| app.docs.active_doc().and_then(|doc| doc.resx.as_ref()).is_some_and(|resx| resx.group.is_some()));
        harness.run_steps(2);
        // The French cell is missing: a click edits it, typing adds the translation.
        harness.get_by_label("Hello fr").click();
        harness.run_steps(2);
        harness.get_by_role(egui::accesskit::Role::MultilineTextInput).type_text("Bonjour");
        harness.run_steps(2);
        harness.key_press(egui::Key::Enter);
        harness.run_steps(2);
        assert!(harness.state().docs.active_doc().unwrap().is_modified(), "the tab shows unsaved grid edits");
        harness.state_mut().docs.active_doc_mut().unwrap().resx.as_mut().unwrap().add_key();
        harness.run_steps(2);
        harness.state_mut().run_ide_action(&egui::Context::default(), rc_editor::Action::Save);
        assert!(!harness.state().docs.active_doc().unwrap().is_modified());
        assert!(std::fs::read_to_string(&french).unwrap().contains("<value>Bonjour</value>"));
        assert!(std::fs::read_to_string(&neutral).unwrap().contains("name=\"NewKey\""));
        let designer = std::fs::read_to_string(&designer).unwrap();
        assert!(designer.contains("internal static string NewKey {") && designer.contains("GetString(\"NewKey\", resourceCulture)"), "{designer}");
        // The XML view shows the saved text.
        let index = harness.state().docs.active.unwrap();
        switch_view(harness.state_mut(), index, ResxView::Xml);
        assert!(harness.state().docs.list[index].editor.buffer.rope().to_string().contains("NewKey"));
    }
}
