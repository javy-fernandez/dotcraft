//! Native read-only SQLite connection, schema and explicit query controls.

use crate::{app::DotCraftApp, database::{Action, Error, View, MAX_SQL}, toolwindow::ToolWindow, ui::kit};
mod insight;

use egui::{FontId, RichText, Ui};
use rc_kit::i18n::{tr, trf};
use rc_kit::{Ico, ico, theme};

/// The views, shown as header tabs.
fn views() -> [(Ico, &'static str, View); 4] {
    [
        (ico::TW_DATABASE, tr("SQL Console"), View::Query),
        (ico::DATA_COLUMN, tr("Schema Details"), View::Details),
        (ico::TW_HIERARCHY, tr("Relationships"), View::Relations),
        (ico::DIFF, tr("Schema Comparison"), View::Comparison),
    ]
}

pub fn show(app: &mut DotCraftApp, ui: &mut Ui) {
    let db = &app.database;
    let connected = db.connected.is_some();
    let busy = db.busy();
    let status = db.connected.as_ref().and_then(|path| path.file_name()).map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let views = views();
    let tabs: Vec<(Ico, &str, bool)> = if connected { views.iter().map(|(icon, label, view)| (*icon, *label, db.view == *view)).collect() } else { Vec::new() };
    // The tabs move to a toolbar row when the header is too narrow for them.
    let p = rc_kit::pal(ui);
    let tabs_w: f32 = tabs.iter().map(|(_, label, _)| ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(12.5), p.text).size().x + 38.0).sum();
    let in_header = ui.available_width() > tabs_w + 260.0;
    let mut actions = vec![(ico::HIDE, tr("Hide"), true), (ico::REFRESH, tr("Refresh Schema"), connected && !busy)];
    if connected || busy { actions.push((ico::CLOSE, tr("Disconnect"), true)); }
    let click = kit::header(ui, tr("Database"), if in_header { &tabs } else { &[] }, &status, &actions);
    match click.action {
        Some(0) => app.toggle_tool(ToolWindow::Database),
        Some(1) => app.database.execute(Action::Schema),
        Some(2) => app.database.disconnect(),
        _ => {}
    }
    if let Some(tab) = click.tab { app.database.view = views[tab].2; }
    // The connection, schema, console and results are taller than a docked panel.
    egui::ScrollArea::vertical().id_salt("database_window").auto_shrink([false, false]).show(ui, |ui| {
        if !in_header && !tabs.is_empty() {
            kit::toolbar(ui, |ui| {
                for (icon, label, view) in views {
                    if theme::tool_button(ui, icon, label, app.database.view == view).clicked() { app.database.view = view; }
                }
            });
        }
        body(app, ui)
    });
}

fn body(app: &mut DotCraftApp, ui: &mut Ui) {
    let root = app.workspace.as_ref().map(|workspace| workspace.ws.root.clone());
    let db = &mut app.database;
    if db.connected.is_none() {
        let busy = db.busy();
        let mut connect = false;
        kit::content(ui, |ui| {
            kit::note(ui, tr("SQLite connections are read-only. Queries run only when requested."));
            ui.add_space(6.0);
            let (field, clicked) = field_row(ui, tr("SQLite database file"), &mut db.path, 340.0, !busy, |ui| kit::button(ui, Some(ico::OPEN), tr("Connect"), !busy).clicked());
            connect = clicked || (field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)));
        });
        if connect { db.connect(root.as_deref()); }
    }
    if db.busy() {
        kit::toolbar(ui, |ui| {
            ui.spinner();
            kit::note(ui, tr("Reading SQLite database…"));
            if kit::button(ui, Some(ico::STOP), tr("Cancel Query"), true).clicked() { db.cancel(); }
        });
    }
    if let Some(error) = &db.error { kit::content(ui, |ui| kit::message(ui, &error_text(error), true)); }
    if db.connected.is_none() { return; }
    let busy = db.busy();
    let mut action = None;
    kit::toolbar(ui, |ui| {
        let options: Vec<String> = db.tables.iter().map(|table| format!("{} ({})", table.name, table.kind)).collect();
        let selected = options.get(db.selected).map(String::as_str).unwrap_or_else(|| tr("Select a database table"));
        if let Some(index) = kit::dropdown(ui, "sqlite_table", selected, &options, !options.is_empty()) { db.selected = index; }
        if kit::button(ui, Some(ico::PREVIEW), tr("Browse Table"), !busy && !db.tables.is_empty()).clicked()
            && let Some(table) = db.tables.get(db.selected)
        { action = Some(Action::Browse(table.name.clone())); }
    });
    if db.schema_truncated || db.tables.is_empty() {
        kit::content(ui, |ui| {
            if db.schema_truncated { kit::note(ui, tr("Schema list truncated to 200 tables and views.")); }
            if db.tables.is_empty() { kit::note(ui, tr("No user tables or views found.")); }
        });
    }
    if let Some(table) = db.tables.get(db.selected)
        && section(ui, "sqlite_table_schema", ico::DATA_TABLE, tr("Table Schema"), &table.name, false)
    {
        kit::content(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("sqlite_schema").max_height(120.0).show(ui, |ui| { ui.label(RichText::new(&table.sql).monospace().size(12.0)); });
        });
    }
    ui.add_space(4.0);
    if let Some(panel_action) = insight::panels(db, root.as_deref(), ui) { action = Some(panel_action); }
    if db.view != View::Query {
        if let Some(action) = action { db.execute(action); }
        return;
    }
    let mut editor = kit::content(ui, |ui| {
        egui::TextEdit::multiline(&mut db.sql).id_salt("sqlite_sql_editor").code_editor().desired_rows(4).desired_width(f32::INFINITY).margin(egui::vec2(6.0, 4.0)).char_limit(MAX_SQL).show(ui)
    });
    if let Some(range) = editor.cursor_range { db.cursor = range.primary.index.into(); }
    kit::toolbar(ui, |ui| {
        let can_run = !db.busy() && !db.sql.trim().is_empty();
        if kit::button(ui, Some(ico::RUN), tr("Run SQL"), can_run && db.sql.len() <= MAX_SQL).clicked() { action = Some(Action::Query(db.sql.clone())); }
        if kit::button(ui, Some(ico::EVALUATE), tr("Explain Query Plan"), can_run).clicked() { action = Some(Action::Explain(db.sql.clone())); }
        if kit::button(ui, Some(ico::BULB), tr("SQL Suggestions"), true).clicked() { db.show_suggestions = !db.show_suggestions; }
    });
    if db.show_suggestions {
        kit::content(ui, |ui| {
            let choices = crate::database::completions(&db.sql, db.cursor, &db.tables, db.details.as_ref());
            if choices.is_empty() { kit::note(ui, tr("No local suggestions at the caret.")); }
            egui::ScrollArea::vertical().id_salt("sqlite_suggestions").max_height(120.0).show(ui, |ui| { ui.horizontal_wrapped(|ui| {
                for choice in choices {
                    if kit::button(ui, None, &choice.label, true).clicked()
                        && let Some(cursor) = crate::database::apply_completion(&mut db.sql, &choice) {
                        db.cursor = cursor;
                        editor.state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(cursor))));
                        editor.state.clone().store(ui.ctx(), editor.response.id);
                        editor.response.request_focus();
                        db.show_suggestions = false;
                    }
                }
            }); });
            kit::note(ui, tr("Suggestions use loaded table names and inspected columns; they do not validate SQL."));
        });
    }
    if let Some(action) = action { db.execute(action); }
    if let Some(grid) = &db.grid {
        kit::toolbar(ui, |ui| {
            if db.grid_is_plan { ui.label(RichText::new(tr("SQLite Query Plan")).font(theme::bold(12.5))); }
            kit::note(ui, &trf("{count} rows displayed", &[("count", &grid.rows.len())]));
            if grid.truncated { kit::message(ui, tr("Results truncated at 500 rows or 2 MiB."), false); }
            kit::note(ui, tr("Cells are clipped to one line. Hover for up to 4096 text characters or a BLOB preview."));
        });
        egui::Frame::new().inner_margin(egui::Margin { left: 10, right: 10, top: 0, bottom: 8 }).show(ui, |ui| result_table(ui, grid, None));
    }
}

/// A section row whose open state lives in egui memory.
pub(super) fn section(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, icon: Ico, title: &str, secondary: &str, default_open: bool) -> bool {
    let id = ui.make_persistent_id(id);
    let mut open = ui.data(|data| data.get_temp::<bool>(id)).unwrap_or(default_open);
    let was = open;
    kit::section(ui, icon, title, secondary, &mut open);
    if open != was { ui.data_mut(|data| data.insert_temp(id, open)); }
    open
}

/// A grey caption over a single-line path field, with trailing controls (buttons) on its row.
pub(super) fn field_row<R>(ui: &mut Ui, caption: &str, text: &mut String, width: f32, enabled: bool, trailing: impl FnOnce(&mut Ui) -> R) -> (egui::Response, R) {
    let p = rc_kit::pal(ui);
    let caption = ui.label(RichText::new(caption).size(12.0).color(p.muted));
    ui.horizontal(|ui| {
        let field = ui.add_enabled(enabled, egui::TextEdit::singleline(text).char_limit(8192).desired_width(width).margin(egui::vec2(6.0, 4.0))).labelled_by(caption.id);
        (field, trailing(ui))
    })
    .inner
}

/// Fixed-height virtual rows keep large results from creating every cell each frame.
fn result_table(ui: &mut Ui, grid: &crate::database::Grid, scroll_to: Option<usize>) -> usize {
    let p = rc_kit::pal(ui);
    let mut rendered = 0;
    let height = ui.available_height().max(40.0);
    egui::ScrollArea::horizontal().id_salt("sqlite_results_horizontal").show(ui, |ui| {
        ui.set_min_width(grid.columns.len() as f32 * 160.0);
        let mut table = egui_extras::TableBuilder::new(ui)
            .id_salt("sqlite_results_table")
            .striped(true)
            .resizable(true)
            .columns(egui_extras::Column::initial(160.0).range(80.0..=320.0).clip(true), grid.columns.len())
            .min_scrolled_height(40.0)
            .max_scroll_height(height);
        if let Some(row) = scroll_to { table = table.scroll_to_row(row, Some(egui::Align::BOTTOM)); }
        table.header(24.0, |mut header| {
            for name in &grid.columns {
                header.col(|ui| {
                    ui.painter().rect_filled(ui.max_rect().expand2(egui::vec2(4.0, 0.0)), 0.0, p.header_bg);
                    ui.add_space(6.0);
                    ui.add(egui::Label::new(RichText::new(name).strong().size(12.0)).truncate()).on_hover_text(name);
                });
            }
        }).body(|body| {
            body.rows(24.0, grid.rows.len(), |mut row| {
                rendered += 1;
                for cell in &grid.rows[row.index()] {
                    row.col(|ui| {
                        let preview = cell.display();
                        // Keep one line in the grid; preserve bounded multiline text in the tooltip.
                        let line = preview.replace(['\n', '\r', '\t'], " ");
                        let color = if matches!(cell, crate::database::Cell::Null) { p.muted } else { p.text };
                        ui.add_space(6.0);
                        ui.add(egui::Label::new(RichText::new(line).monospace().size(12.0).color(color)).truncate().selectable(true)).on_hover_text(preview);
                    });
                }
            });
        });
    });
    rendered
}

fn error_text(error: &Error) -> String {
    match error {
        Error::Path => tr("Choose an existing SQLite file; use an absolute path outside a workspace.").into(),
        Error::Input => tr("Enter one supported SQL statement or select an existing table.").into(),
        Error::ReadOnly => tr("This database connection only accepts read-only queries.").into(),
        Error::Limit => tr("SQLite input or results exceed the supported limits.").into(),
        Error::Cancelled => tr("Query cancelled.").into(),
        Error::TimedOut => tr("Query exceeded the 10 second time limit.").into(),
        Error::Sql(error) => trf("SQLite error: {error}", &[("error", error)]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rc_core::{Settings, Waker};
    use egui_kittest::{Harness, kittest::Queryable};
    use std::time::{Duration, Instant};

    fn pump(harness: &mut Harness<DotCraftApp>, ready: impl Fn(&DotCraftApp) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready(harness.state()) {
            assert!(Instant::now() < deadline, "SQLite UI timed out");
            harness.run_steps(1);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn sqlite_insight_controls_inspect_compare_navigate_and_explain_explicitly() {
        let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ui-insight.sqlite");
        rusqlite::Connection::open(&path).unwrap().execute_batch("CREATE TABLE authors(id INTEGER PRIMARY KEY); CREATE TABLE books(title TEXT, author INTEGER REFERENCES authors(id)); CREATE INDEX books_title ON books(title);").unwrap();
        let other = dir.path().join("other.sqlite");
        std::fs::copy(&path,&other).unwrap();
        rusqlite::Connection::open(&other).unwrap().execute_batch("CREATE TABLE editions(id INTEGER);").unwrap();
        let original = (std::fs::read(&path).unwrap(),std::fs::read(&other).unwrap());
        let mut app = DotCraftApp::with_settings(Settings { language: Some("en".into()), ..Settings::default() }, Waker::noop(), None);
        app.database.path = path.display().to_string();
        let mut first = true;
        let mut harness = Harness::builder().with_size(egui::vec2(1000.0,900.0)).build_ui_state(move |ui,app:&mut DotCraftApp| {
            if std::mem::take(&mut first) { rc_kit::theme::install_fonts(ui.ctx()); return; }
            app.database.update(); show(app,ui);
        },app);
        harness.run_steps(2);
        harness.get_by_label("Connect").click();
        pump(&mut harness,|app|app.database.connected.is_some());
        assert!(harness.state().database.details.is_none());
        harness.state_mut().database.selected=1;
        harness.get_by_label("Schema Details").click(); harness.run_steps(2);
        assert!(harness.state().database.details.is_none(), "switching panels must not start queries");
        harness.get_by_label("Inspect Table").click();
        pump(&mut harness,|app|app.database.details.is_some());
        assert_eq!(harness.state().database.details.as_ref().unwrap().table,"books");
        harness.get_by_label("Columns").click(); harness.run_steps(2);
        harness.get_by_label("title");
        harness.get_by_label("Relationships").click(); harness.run_steps(2);
        assert!(harness.state().database.relations.is_none());
        harness.get_by_label("Load Relationships").click();
        pump(&mut harness,|app|app.database.relations.is_some());
        assert_eq!(harness.state().database.relations.as_ref().unwrap().edges.len(),1);
        harness.get_by_role_and_label(egui::accesskit::Role::Button,"authors").click();
        pump(&mut harness,|app|app.database.details.as_ref().is_some_and(|details|details.table=="authors"));
        assert_eq!(harness.state().database.view,crate::database::View::Details);
        harness.get_by_label("Schema Comparison").click(); harness.run_steps(2);
        harness.state_mut().database.compare_path=other.display().to_string(); harness.run_steps(2);
        assert!(harness.state().database.comparison.is_none());
        harness.get_by_label("Compare Schemas").click();
        pump(&mut harness,|app|app.database.comparison.is_some());
        assert_eq!(harness.state().database.comparison.as_ref().unwrap().changes[0].name,"editions");
        harness.get_by_label("SQL Console").click(); harness.run_steps(2);
        harness.state_mut().database.sql="SELECT * FROM books WHERE title='test'".into(); harness.run_steps(2);
        harness.get_by_label("Explain Query Plan").click();
        pump(&mut harness,|app|app.database.grid_is_plan);
        harness.get_by_label("SQLite Query Plan");
        assert!(harness.state().database.grid.as_ref().unwrap().rows.iter().flatten().any(|cell|matches!(cell,crate::database::Cell::Text(value) if value.contains("books_title"))));
        harness.state_mut().database.sql="SEL".into(); harness.run_steps(2);
        harness.get_by_label("SQL Suggestions").click(); harness.run_steps(2);
        harness.get_by_role_and_label(egui::accesskit::Role::Button,"SELECT").click(); harness.run_steps(2);
        assert_eq!(harness.state().database.sql,"SELECT");
        assert!(!harness.state().database.busy());
        harness.get_by_label("Disconnect").click(); harness.run_steps(2);
        assert!(harness.state().database.details.is_none() && harness.state().database.relations.is_none() && harness.state().database.comparison.is_none());
        assert_eq!(original,(std::fs::read(&path).unwrap(),std::fs::read(&other).unwrap()));
    }

    #[test]
    fn sqlite_result_table_virtualizes_and_scrolls_to_the_last_row() {
        let grid = crate::database::Grid { columns: vec!["value".into()], rows: (0..500).map(|i| vec![crate::database::Cell::Text(format!("row {i}"))]).collect(), truncated: false };
        let mut harness = Harness::builder().with_size(egui::vec2(600.0, 240.0)).build_ui_state(
            |ui, rendered: &mut usize| { *rendered = result_table(ui, &grid, Some(499)); }, 0usize,
        );
        harness.run_steps(5);
        assert!(*harness.state() < 20, "only visible rows should render");
        harness.get_by_label("row 499");
    }

    #[test]
    fn sqlite_window_connects_browses_and_runs_only_explicit_queries() {
        let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ui.sqlite");
        rusqlite::Connection::open(&path).unwrap().execute_batch("CREATE TABLE books(title TEXT); INSERT INTO books VALUES('Native SQLite');").unwrap();
        let mut app = DotCraftApp::with_settings(Settings { language: Some("en".into()), ..Settings::default() }, Waker::noop(), None);
        app.database.path = path.display().to_string();
        let mut first = true;
        let mut harness = Harness::builder().with_size(egui::vec2(1000.0, 800.0)).build_ui_state(move |ui, app: &mut DotCraftApp| {
            if std::mem::take(&mut first) { rc_kit::theme::install_fonts(ui.ctx()); return; }
            app.database.update();
            show(app, ui);
        }, app);
        harness.run_steps(2);
        assert!(harness.state().database.connected.is_none());
        assert!(harness.state().database.grid.is_none());
        harness.get_by_label("Connect").click();
        pump(&mut harness, |app| app.database.connected.is_some());
        assert_eq!(harness.state().database.tables[0].name, "books");
        assert!(harness.state().database.grid.is_none());
        harness.get_by_label("Browse Table").click();
        pump(&mut harness, |app| app.database.grid.is_some());
        assert_eq!(harness.state().database.grid.as_ref().unwrap().rows[0][0], crate::database::Cell::Text("Native SQLite".into()));
        harness.state_mut().database.sql = "SELECT 42 AS answer".into();
        harness.get_by_label("Run SQL").click();
        pump(&mut harness, |app| app.database.grid.as_ref().is_some_and(|grid| grid.columns == ["answer"]));
        assert_eq!(harness.state().database.grid.as_ref().unwrap().rows[0][0], crate::database::Cell::Integer(42));
        harness.state_mut().database.sql = "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000000000) SELECT sum(x) FROM n".into();
        harness.get_by_label("Run SQL").click();
        harness.run_steps(2);
        assert!(harness.state().database.busy());
        harness.get_by_label("Cancel Query").click();
        pump(&mut harness, |app| app.database.error == Some(Error::Cancelled));
        harness.get_by_label("Disconnect").click();
        harness.run_steps(2);
        assert!(harness.state().database.connected.is_none());
    }
}
