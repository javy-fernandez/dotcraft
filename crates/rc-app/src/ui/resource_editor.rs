//! Grid editor for sibling `.resx` culture files.

use egui::{Align2, Rect, Sense, Ui, pos2, vec2};
use rc_dotnet::resx::ResourceCell;
use rc_kit::i18n::tr;
use rc_kit::{ico, theme};

use crate::app::DotCraftApp;
use crate::toolwindow::ToolWindow;

const HEADER_H: f32 = 36.0;

pub fn show(app: &mut DotCraftApp, ui: &mut Ui) {
    app.resource_editor.update();
    let palette = rc_kit::pal(ui);
    let dirty = app.resource_editor.dirty();
    let busy = app.resource_editor.loading || app.resource_editor.saving;
    let header = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), HEADER_H));
    ui.allocate_rect(header, Sense::hover());
    let title = match &app.resource_editor.path {
        Some(path) => format!(
            "{} · {}{}",
            tr("Resources"),
            path.file_name()
                .map(|name| name.to_string_lossy())
                .unwrap_or_default(),
            if dirty { " *" } else { "" }
        ),
        None => tr("Resources").to_string(),
    };
    ui.painter()
        .with_clip_rect(Rect::from_min_max(
            header.min,
            pos2(header.right() - 154.0, header.bottom()),
        ))
        .text(
            pos2(header.left() + 12.0, header.center().y),
            Align2::LEFT_CENTER,
            title,
            theme::bold(13.0),
            palette.text,
        );
    let actions = [
        (
            ico::FILE_XML,
            tr("Open XML"),
            app.resource_editor.path.is_some() && !dirty && !busy,
        ),
        (ico::UNDO, tr("Discard"), dirty && !busy),
        (
            ico::REFRESH,
            tr("Refresh"),
            app.resource_editor.path.is_some() && !busy,
        ),
        (ico::SAVE, tr("Save"), dirty && !busy),
        (ico::HIDE, tr("Hide"), true),
    ];
    let mut action = None;
    let action_count = actions.len();
    for (index, (icon, label, enabled)) in actions.into_iter().enumerate() {
        let rect = Rect::from_min_size(
            pos2(
                header.right() - 8.0 - 28.0 * (action_count - index) as f32,
                header.center().y - 13.0,
            ),
            vec2(26.0, 26.0),
        );
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        if theme::icon_button(&mut child, icon, enabled, label).clicked() {
            action = Some(index);
        }
    }
    match action {
        Some(0) => {
            if let Some(path) = app.resource_editor.path.clone() {
                app.open_text_file(&path);
                app.dock.toggle(ToolWindow::Resources);
                app.session_dirty = true;
            }
            return;
        }
        Some(1) => app.resource_editor.refresh(true),
        Some(2) => app.resource_editor.refresh(false),
        Some(3) => app.resource_editor.save(),
        Some(4) => {
            app.toggle_tool(ToolWindow::Resources);
            return;
        }
        _ => {}
    }
    if let Some(error) = &app.resource_editor.error {
        ui.colored_label(palette.error, error);
    }
    if busy {
        ui.label(if app.resource_editor.saving {
            tr("Saving…")
        } else {
            tr("Loading…")
        });
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
        return;
    }
    let Some(group) = app.resource_editor.group.as_mut() else {
        ui.label(tr("Open a .resx file to edit resources"));
        return;
    };
    ui.horizontal(|ui| {
        if ui.button(tr("Add")).clicked() {
            group.add_row();
        }
        ui.label(tr("Comments"));
        let selected = app
            .resource_editor
            .comment_column
            .min(group.files.len().saturating_sub(1));
        let label = culture_label(&group.files[selected].culture);
        egui::ComboBox::from_id_salt("resx_comment_culture")
            .selected_text(label)
            .show_ui(ui, |ui| {
                for (index, file) in group.files.iter().enumerate() {
                    ui.selectable_value(
                        &mut app.resource_editor.comment_column,
                        index,
                        culture_label(&file.culture),
                    );
                }
            });
    });
    let mut remove = None;
    let comment_column = app
        .resource_editor
        .comment_column
        .min(group.files.len().saturating_sub(1));
    egui::ScrollArea::both()
        .id_salt("resource_grid_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("resource_grid")
                .striped(true)
                .spacing(vec2(8.0, 8.0))
                .show(ui, |ui| {
                    ui.label(tr("Key"));
                    for file in &group.files {
                        ui.label(culture_label(&file.culture));
                    }
                    ui.label(tr("Comments"));
                    ui.label("");
                    ui.end_row();
                    for (index, row) in group.rows.iter_mut().enumerate() {
                        ui.add(egui::TextEdit::singleline(&mut row.name).desired_width(170.0));
                        for slot in &mut row.cells {
                            match slot {
                                Some(cell) => {
                                    let mut remove_translation = false;
                                    ui.horizontal(|ui| {
                                        ui.add(
                                            egui::TextEdit::multiline(&mut cell.value)
                                                .desired_width(190.0)
                                                .desired_rows(2),
                                        );
                                        if ui
                                            .small_button("×")
                                            .on_hover_text(tr("Remove translation"))
                                            .clicked()
                                        {
                                            remove_translation = true;
                                        }
                                    });
                                    if remove_translation {
                                        *slot = None;
                                    }
                                }
                                None => {
                                    ui.horizontal(|ui| {
                                        ui.colored_label(palette.error, tr("Missing translation"));
                                        if ui
                                            .small_button("+")
                                            .on_hover_text(tr("Add translation"))
                                            .clicked()
                                        {
                                            *slot = Some(ResourceCell {
                                                value: String::new(),
                                                comment: None,
                                            });
                                        }
                                    });
                                }
                            }
                        }
                        let comment_cell = &mut row.cells[comment_column];
                        let mut comment = comment_cell
                            .as_ref()
                            .and_then(|cell| cell.comment.clone())
                            .unwrap_or_default();
                        if ui
                            .add(
                                egui::TextEdit::multiline(&mut comment)
                                    .desired_width(190.0)
                                    .desired_rows(2),
                            )
                            .changed()
                        {
                            let cell = comment_cell.get_or_insert(ResourceCell {
                                value: String::new(),
                                comment: None,
                            });
                            cell.comment = Some(comment);
                        }
                        if ui
                            .small_button("×")
                            .on_hover_text(tr("Remove key"))
                            .clicked()
                        {
                            remove = Some(index);
                        }
                        ui.end_row();
                    }
                });
        });
    if let Some(index) = remove {
        group.rows.remove(index);
    }
}

fn culture_label(culture: &Option<String>) -> String {
    culture.clone().unwrap_or_else(|| tr("Neutral").to_string())
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

    #[test]
    fn opening_resx_shows_grid_and_saves_a_translation() {
        let _lock = crate::screens::HOME_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let neutral = workspace.path().join("Messages.resx");
        let french = workspace.path().join("Messages.fr.resx");
        std::fs::write(
            &neutral,
            "<root>\n  <data name=\"Hello\"><value>Hello</value></data>\n</root>\n",
        )
        .unwrap();
        std::fs::write(&french, "<root>\n</root>\n").unwrap();
        let mut app = crate::screens::test_app(home.path(), true);
        app.open_workspace(workspace.path());
        app.open_file(&neutral);
        assert!(app.dock.is_open(ToolWindow::Resources));
        let mut harness = crate::screens::harness(app, egui::vec2(1200.0, 650.0));
        for _ in 0..50 {
            harness.run_steps(2);
            if harness.state().resource_editor.group.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(harness.state().resource_editor.group.is_some());
        harness.get_by_label(tr("Missing translation"));
        harness
            .state_mut()
            .resource_editor
            .group
            .as_mut()
            .unwrap()
            .rows[0]
            .cells[1] = Some(ResourceCell {
            value: "Bonjour".into(),
            comment: None,
        });
        harness.run_steps(2);
        harness.get_by_label(tr("Save")).click();
        for _ in 0..50 {
            harness.run_steps(2);
            if !harness.state().resource_editor.saving && !harness.state().resource_editor.dirty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!harness.state().resource_editor.dirty());
        assert!(std::fs::read_to_string(french).unwrap().contains("Bonjour"));
    }
}
