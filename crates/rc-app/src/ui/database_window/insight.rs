//! Native SQLite metadata, comparison, and relationship views.
use super::*;
use crate::database::{ChangeKind, Database, Grid, Relations, View};
use std::collections::BTreeMap;

pub(super) fn panels(db: &mut Database, root: Option<&std::path::Path>, ui: &mut Ui) -> Option<Action> {
    ui.horizontal_wrapped(|ui| {
        ui.selectable_value(&mut db.view, View::Query, tr("SQL Console"));
        ui.selectable_value(&mut db.view, View::Details, tr("Schema Details"));
        ui.selectable_value(&mut db.view, View::Relations, tr("Relationships"));
        ui.selectable_value(&mut db.view, View::Comparison, tr("Schema Comparison"));
    });
    let mut action = None;
    match db.view {
        View::Query => {}
        View::Details => {
            if ui.add_enabled(!db.busy() && !db.tables.is_empty(), egui::Button::new(tr("Inspect Table"))).clicked()
                && let Some(table) = db.tables.get(db.selected) { action = Some(Action::Inspect(table.name.clone())); }
            ui.label(tr("Inspect the selected table to load its columns, indexes, and foreign keys."));
            if let Some(details) = &db.details {
                ui.strong(&details.table);
                metadata(ui, tr("Columns"), &details.columns);
                metadata(ui, tr("Indexes"), &details.indexes);
                metadata(ui, tr("Foreign Keys"), &details.foreign_keys);
            }
        }
        View::Relations => {
            if ui.add_enabled(!db.busy(), egui::Button::new(tr("Load Relationships"))).clicked() { action = Some(Action::Relations); }
            ui.label(tr("Arrows point from foreign-key tables to referenced tables. Click a table to inspect it."));
            if let Some(relations) = &db.relations {
                if relations.truncated { ui.label(tr("Relationship diagram limited to 200 tables, 500 column links, or 2 MiB.")); }
                if let Some(name) = diagram(ui, relations) {
                    db.selected = db.tables.iter().position(|table| table.name == name).unwrap_or(db.selected);
                    db.view = View::Details;
                    action = Some(Action::Inspect(name));
                }
            }
        }
        View::Comparison => {
            ui.label(tr("Compare the current schema with another SQLite file. Differences describe the other file; neither database is changed."));
            ui.horizontal_wrapped(|ui| {
                ui.add(egui::TextEdit::singleline(&mut db.compare_path).hint_text(tr("SQLite file to compare")).char_limit(8192).desired_width(340.0));
                if ui.add_enabled(!db.busy() && !db.compare_path.trim().is_empty(), egui::Button::new(tr("Compare Schemas"))).clicked() {
                    let path = std::path::PathBuf::from(db.compare_path.trim());
                    if path.is_absolute() { action = Some(Action::Compare(path)); }
                    else if let Some(root) = root { action = Some(Action::Compare(root.join(path))); }
                    else { db.error = Some(Error::Path); }
                }
            });
            if let Some(comparison) = &db.comparison {
                ui.label(comparison.other.display().to_string());
                ui.label(trf("{count} unchanged schema objects", &[("count", &comparison.unchanged)]));
                if comparison.changes.is_empty() { ui.label(tr("Schemas match.")); }
                egui::ScrollArea::vertical().id_salt("sqlite_schema_comparison").max_height(320.0).show(ui, |ui| {
                    for change in &comparison.changes {
                        let status = match change.kind { ChangeKind::Added => tr("Added in other file"), ChangeKind::Removed => tr("Missing in other file"), ChangeKind::Changed => tr("Definition changed") };
                        ui.collapsing(format!("{status}: {} ({})", change.name, change.object_type), |ui| {
                            if let Some(sql) = &change.current { ui.strong(tr("Current Definition")); definition(ui, sql); }
                            if let Some(sql) = &change.other { ui.strong(tr("Other Definition")); definition(ui, sql); }
                        });
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
    ui.monospace(preview);
    if truncated { ui.label(tr("Definition preview limited to 8192 characters.")); }
}

fn metadata(ui: &mut Ui, title: &str, grid: &Grid) {
    ui.collapsing(title, |ui| {
        if grid.rows.is_empty() { ui.label(tr("No metadata entries.")); }
        if grid.truncated { ui.label(tr("Metadata results are truncated.")); }
        egui::ScrollArea::both().id_salt(("sqlite_metadata", title)).max_height(160.0).show(ui, |ui| {
            egui::Grid::new(("sqlite_metadata_cells", title)).striped(true).show(ui, |ui| {
                for column in &grid.columns {
                    let heading = match column.as_str() {
                        "name" => tr("Name"), "type" => tr("Type"), "required" => tr("Required"),
                        "default_value" => tr("Default Value"), "primary_key" => tr("Primary Key Position"), "hidden" => tr("Hidden Column Flag"),
                        "unique" => tr("Unique Index"), "origin" => tr("Index Origin"), "partial" => tr("Partial Index"),
                        "seqno" | "seq" => tr("Sequence"), "cid" => tr("Column ID"), "column_name" => tr("Column Name"),
                        "id" => tr("Key ID"), "target_table" => tr("Target Table"), "source_column" => tr("Source Column"),
                        "target_column" => tr("Target Column"), "on_update" => tr("On Update"), "on_delete" => tr("On Delete"),
                        _ => column,
                    };
                    ui.strong(heading);
                } ui.end_row();
                for row in &grid.rows {
                    for value in row {
                        let text = value.display();
                        ui.add(egui::Label::new(text.chars().take(100).collect::<String>()).truncate()).on_hover_text(text);
                    }
                    ui.end_row();
                }
            });
        });
    });
}

/// Draw a bounded graph of actual SQLite foreign-key metadata, retaining labels for
/// accessibility and links to metadata inspection. No schema discovery runs here.
fn diagram(ui: &mut Ui, relations: &Relations) -> Option<String> {
    let mut clicked = None;
    if relations.tables.is_empty() { ui.label(tr("No user tables found.")); return None; }
    let columns = 3usize;
    let cell = egui::vec2(240.0, 92.0);
    let rows = relations.tables.len().div_ceil(columns);
    let size = egui::vec2(cell.x * columns as f32, cell.y * rows as f32);
    egui::ScrollArea::both().id_salt("sqlite_relationship_diagram").max_height(320.0).show(ui, |ui| {
        let (canvas, _) = ui.allocate_exact_size(size, egui::Sense::hover());
        let boxes: BTreeMap<_,_> = relations.tables.iter().enumerate().map(|(index,name)| {
            let offset = egui::vec2((index%columns) as f32*cell.x+20.0, (index/columns) as f32*cell.y+20.0);
            (name.as_str(), egui::Rect::from_min_size(canvas.min+offset, egui::vec2(190.0, 44.0)))
        }).collect();
        let stroke = egui::Stroke::new(1.5, rc_kit::pal(ui).muted);
        for edge in &relations.edges {
            if let (Some(from),Some(to)) = (boxes.get(edge.from.as_str()),boxes.get(edge.to.as_str())) {
                if edge.from == edge.to {
                    let points = vec![from.right_center(), from.right_center()+egui::vec2(15.0,0.0), from.right_top()+egui::vec2(15.0,-10.0), from.center_top()+egui::vec2(0.0,-10.0), from.center_top()];
                    ui.painter().add(egui::Shape::line(points,stroke));
                } else {
                    let start = from.center_bottom(); let end = to.center_top();
                    ui.painter().arrow(start, end-start, stroke);
                }
            }
        }
        for (name,rect) in boxes {
            if ui.put(rect, egui::Button::new(name).wrap()).on_hover_text(name).clicked() { clicked = Some(name.to_string()); }
        }
    });
    ui.collapsing(tr("Foreign-key Links"), |ui| {
        egui::ScrollArea::vertical().id_salt("sqlite_relationship_labels").max_height(140.0).show(ui, |ui| {
            if relations.edges.is_empty() { ui.label(tr("No declared foreign keys.")); }
            for edge in &relations.edges {
                let target = if edge.to_column.is_empty() { tr("primary key") } else { &edge.to_column };
                ui.label(format!("{}.{} → {}.{}", edge.from, edge.from_column, edge.to, target));
            }
        });
    });
    clicked
}
