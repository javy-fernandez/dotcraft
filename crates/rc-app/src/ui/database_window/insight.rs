//! Native SQLite metadata, comparison, and relationship views.
use super::*;
use crate::database::{ChangeKind, Database, Grid, Relations};
use egui::{Align2, Rect, Sense, pos2, vec2};
use std::collections::BTreeMap;

pub(super) fn panels(db: &mut Database, root: Option<&std::path::Path>, ui: &mut Ui) -> Option<Action> {
    let mut action = None;
    match db.view {
        View::Query => {}
        View::Details => {
            kit::toolbar(ui, |ui| {
                if kit::button(ui, Some(ico::SEARCH), tr("Inspect Table"), !db.busy() && !db.tables.is_empty()).clicked()
                    && let Some(table) = db.tables.get(db.selected) { action = Some(Action::Inspect(table.name.clone())); }
                kit::note(ui, tr("Inspect the selected table to load its columns, indexes, and foreign keys."));
            });
            if let Some(details) = &db.details {
                kit::content(ui, |ui| {
                    ui.horizontal(|ui| {
                        rc_kit::icons::image(ui, ico::DATA_TABLE, 16.0);
                        ui.label(RichText::new(&details.table).font(theme::bold(13.0)));
                    });
                });
                metadata(ui, ico::DATA_COLUMN, tr("Columns"), &details.columns);
                metadata(ui, ico::SORT, tr("Indexes"), &details.indexes);
                metadata(ui, ico::TW_HIERARCHY, tr("Foreign Keys"), &details.foreign_keys);
            }
        }
        View::Relations => {
            kit::toolbar(ui, |ui| {
                if kit::button(ui, Some(ico::TW_HIERARCHY), tr("Load Relationships"), !db.busy()).clicked() { action = Some(Action::Relations); }
                kit::note(ui, tr("Arrows point from foreign-key tables to referenced tables. Click a table to inspect it."));
            });
            if let Some(relations) = &db.relations {
                if relations.truncated { kit::content(ui, |ui| kit::message(ui, tr("Relationship diagram limited to 200 tables, 500 column links, or 2 MiB."), false)); }
                if let Some(name) = diagram(ui, relations) {
                    db.selected = db.tables.iter().position(|table| table.name == name).unwrap_or(db.selected);
                    db.view = View::Details;
                    action = Some(Action::Inspect(name));
                }
            }
        }
        View::Comparison => {
            kit::content(ui, |ui| {
                kit::note(ui, tr("Compare the current schema with another SQLite file. Differences describe the other file; neither database is changed."));
                ui.add_space(6.0);
                let can = !db.busy() && !db.compare_path.trim().is_empty();
                let (_, compare) = field_row(ui, tr("SQLite file to compare"), &mut db.compare_path, 340.0, true, |ui| kit::button(ui, Some(ico::DIFF), tr("Compare Schemas"), can).clicked());
                if compare {
                    let path = std::path::PathBuf::from(db.compare_path.trim());
                    if path.is_absolute() { action = Some(Action::Compare(path)); }
                    else if let Some(root) = root { action = Some(Action::Compare(root.join(path))); }
                    else { db.error = Some(Error::Path); }
                }
            });
            if let Some(comparison) = &db.comparison {
                kit::content(ui, |ui| {
                    kit::note(ui, &comparison.other.display().to_string());
                    kit::note(ui, &trf("{count} unchanged schema objects", &[("count", &comparison.unchanged)]));
                    if comparison.changes.is_empty() { kit::note(ui, tr("Schemas match.")); }
                });
                egui::ScrollArea::vertical().id_salt("sqlite_schema_comparison").max_height(320.0).show(ui, |ui| {
                    for (index, change) in comparison.changes.iter().enumerate() {
                        let (status, icon) = match change.kind {
                            ChangeKind::Added => (tr("Added in other file"), ico::ADD),
                            ChangeKind::Removed => (tr("Missing in other file"), ico::REMOVE),
                            ChangeKind::Changed => (tr("Definition changed"), ico::EDIT),
                        };
                        if section(ui, ("sqlite_change", index), icon, &change.name, &format!("{} · {status}", change.object_type), false) {
                            kit::content(ui, |ui| {
                                if let Some(sql) = &change.current { ui.label(RichText::new(tr("Current Definition")).font(theme::bold(12.5))); definition(ui, sql); }
                                if let Some(sql) = &change.other { ui.label(RichText::new(tr("Other Definition")).font(theme::bold(12.5))); definition(ui, sql); }
                            });
                        }
                    }
                });
            }
        }
    }
    action
}

fn definition(ui: &mut Ui, sql: &str) {
    let preview: String = sql.chars().take(8192).collect();
    let truncated = preview.len() < sql.len();
    ui.label(RichText::new(preview).monospace().size(12.0));
    if truncated { kit::note(ui, tr("Definition preview limited to 8192 characters.")); }
}

fn metadata(ui: &mut Ui, icon: Ico, title: &str, grid: &Grid) {
    if !section(ui, ("sqlite_metadata", title), icon, title, &grid.rows.len().to_string(), false) { return; }
    kit::content(ui, |ui| {
        if grid.rows.is_empty() { kit::note(ui, tr("No metadata entries.")); return; }
        if grid.truncated { kit::message(ui, tr("Metadata results are truncated."), false); }
        let headings: Vec<String> = grid.columns.iter().map(|column| match column.as_str() {
            "name" => tr("Name"), "type" => tr("Type"), "required" => tr("Required"),
            "default_value" => tr("Default Value"), "primary_key" => tr("Primary Key Position"), "hidden" => tr("Hidden Column Flag"),
            "unique" => tr("Unique Index"), "origin" => tr("Index Origin"), "partial" => tr("Partial Index"),
            "seqno" | "seq" => tr("Sequence"), "cid" => tr("Column ID"), "column_name" => tr("Column Name"),
            "id" => tr("Key ID"), "target_table" => tr("Target Table"), "source_column" => tr("Source Column"),
            "target_column" => tr("Target Column"), "on_update" => tr("On Update"), "on_delete" => tr("On Delete"),
            _ => column,
        }.to_string()).collect();
        let rows: Vec<Vec<String>> = grid.rows.iter().map(|row| row.iter().map(|cell| cell.display()).collect()).collect();
        cells(ui, ("sqlite_metadata_cells", title), &headings, &rows, 160.0);
    });
}

/// A painted grid like `kit::table`, whose cells are also exposed as labels (cut to 100
/// characters; hover shows the whole value).
fn cells(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug + Copy, columns: &[String], rows: &[Vec<String>], max_height: f32) {
    let p = rc_kit::pal(ui);
    let font = FontId::monospace(12.0);
    let mut widths: Vec<f32> = columns.iter().map(|c| ui.painter().layout_no_wrap(c.clone(), theme::bold(12.0), p.text).size().x + 20.0).collect();
    for row in rows.iter().take(200) {
        for (i, cell) in row.iter().enumerate().take(widths.len()) {
            let w = ui.painter().layout_no_wrap(cell.chars().take(80).collect(), font.clone(), p.text).size().x + 20.0;
            widths[i] = widths[i].max(w).min(360.0);
        }
    }
    let total: f32 = widths.iter().sum::<f32>().max(ui.available_width());
    let h = 24.0;
    egui::ScrollArea::both().id_salt(id).max_height(max_height).auto_shrink([false, true]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let (hr, _) = ui.allocate_exact_size(vec2(total, h), Sense::hover());
        ui.painter().rect_filled(hr, 0.0, p.header_bg);
        let mut x = hr.left();
        for (c, w) in columns.iter().zip(&widths) {
            ui.painter().text(pos2(x + 10.0, hr.center().y), Align2::LEFT_CENTER, c, theme::bold(12.0), p.text);
            x += w;
        }
        for (n, row) in rows.iter().enumerate() {
            let (rr, _) = ui.allocate_exact_size(vec2(total, h), Sense::hover());
            if ui.rect_contains_pointer(rr) {
                ui.painter().rect_filled(rr, 0.0, p.row_hover);
            } else if n % 2 == 1 {
                ui.painter().rect_filled(rr, 0.0, p.row_hover.gamma_multiply(0.5));
            }
            let mut x = rr.left();
            for (i, (cell, w)) in row.iter().zip(&widths).enumerate() {
                let shown: String = cell.chars().take(100).collect::<String>().replace(['\n', '\r', '\t'], " ");
                let mut job = egui::text::LayoutJob::simple_singleline(shown.clone(), font.clone(), p.text);
                job.wrap.max_width = w - 16.0;
                job.wrap.max_rows = 1;
                job.wrap.break_anywhere = true;
                job.wrap.overflow_character = Some('…');
                let g = ui.painter().layout_job(job);
                ui.painter().galley(pos2(x + 10.0, rr.center().y - g.size().y / 2.0), g, p.text);
                let resp = ui.interact(Rect::from_min_size(pos2(x, rr.top()), vec2(*w, h)), ui.id().with((n, i)), Sense::hover());
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &shown));
                resp.on_hover_text(cell.as_str());
                x += w;
            }
        }
    });
}

/// Draw a bounded graph of actual SQLite foreign-key metadata, retaining labels for
/// accessibility and links to metadata inspection. No schema discovery runs here.
fn diagram(ui: &mut Ui, relations: &Relations) -> Option<String> {
    let mut clicked = None;
    if relations.tables.is_empty() { kit::content(ui, |ui| kit::note(ui, tr("No user tables found."))); return None; }
    let p = rc_kit::pal(ui);
    let columns = 3usize;
    let cell = vec2(240.0, 92.0);
    let rows = relations.tables.len().div_ceil(columns);
    let size = vec2(cell.x * columns as f32, cell.y * rows as f32);
    egui::Frame::new().inner_margin(egui::Margin { left: 10, right: 10, top: 0, bottom: 4 }).show(ui, |ui| {
        egui::ScrollArea::both().id_salt("sqlite_relationship_diagram").max_height(320.0).show(ui, |ui| {
            let (canvas, _) = ui.allocate_exact_size(size, Sense::hover());
            let boxes: BTreeMap<_,_> = relations.tables.iter().enumerate().map(|(index,name)| {
                let offset = vec2((index%columns) as f32*cell.x+20.0, (index/columns) as f32*cell.y+20.0);
                (name.as_str(), Rect::from_min_size(canvas.min+offset, vec2(190.0, 44.0)))
            }).collect();
            let stroke = egui::Stroke::new(1.5, p.muted);
            for edge in &relations.edges {
                if let (Some(from),Some(to)) = (boxes.get(edge.from.as_str()),boxes.get(edge.to.as_str())) {
                    if edge.from == edge.to {
                        let points = vec![from.right_center(), from.right_center()+vec2(15.0,0.0), from.right_top()+vec2(15.0,-10.0), from.center_top()+vec2(0.0,-10.0), from.center_top()];
                        ui.painter().add(egui::Shape::line(points,stroke));
                    } else {
                        // Side to side on the same row, otherwise bottom or top edge to the other.
                        let (start, end) = if (from.center().y - to.center().y).abs() < 1.0 {
                            if from.center().x < to.center().x { (from.right_center(), to.left_center()) } else { (from.left_center(), to.right_center()) }
                        } else if from.center().y < to.center().y { (from.center_bottom(), to.center_top()) } else { (from.center_top(), to.center_bottom()) };
                        ui.painter().arrow(start, end-start, stroke);
                    }
                }
            }
            for (name,rect) in boxes {
                let resp = ui.interact(rect, ui.id().with(("sqlite_table_box", name)), Sense::click());
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name));
                let border = if resp.hovered() { p.accent } else { p.control_border };
                ui.painter().rect(rect, 6.0, if resp.hovered() { p.hover } else { p.control_bg }, egui::Stroke::new(1.0, border), egui::StrokeKind::Inside);
                rc_kit::icons::paint(ui.painter(), Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), vec2(16.0, 16.0)), ico::DATA_TABLE, &p, true);
                let mut job = egui::text::LayoutJob::simple_singleline(name.to_string(), theme::bold(12.5), p.text);
                job.wrap.max_width = rect.width() - 42.0;
                job.wrap.max_rows = 1;
                job.wrap.break_anywhere = true;
                job.wrap.overflow_character = Some('…');
                let g = ui.painter().layout_job(job);
                ui.painter().galley(pos2(rect.left() + 34.0, rect.center().y - g.size().y / 2.0), g, p.text);
                if resp.on_hover_text(name).clicked() { clicked = Some(name.to_string()); }
            }
        });
    });
    if section(ui, "sqlite_relationship_links", ico::TW_HIERARCHY, tr("Foreign-key Links"), &relations.edges.len().to_string(), false) {
        kit::content(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("sqlite_relationship_labels").max_height(140.0).show(ui, |ui| {
                if relations.edges.is_empty() { kit::note(ui, tr("No declared foreign keys.")); }
                for edge in &relations.edges {
                    let target = if edge.to_column.is_empty() { tr("primary key") } else { &edge.to_column };
                    ui.label(RichText::new(format!("{}.{} → {}.{}", edge.from, edge.from_column, edge.to, target)).monospace().size(12.0));
                }
            });
        });
    }
    clicked
}
