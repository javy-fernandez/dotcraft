//! Native read-only SQLite connection, schema and explicit query controls.

use crate::{app::DotCraftApp, database::{Action, Error, MAX_SQL}, toolwindow::ToolWindow};
mod insight;

use egui::Ui;
use rc_kit::i18n::{tr, trf};

pub fn show(app: &mut DotCraftApp, ui: &mut Ui) {
    if crate::ui::toolwindows::header(ui, tr("Database")) { app.toggle_tool(ToolWindow::Database); }
    let root = app.workspace.as_ref().map(|workspace| workspace.ws.root.clone());
    let db = &mut app.database;
    ui.label(tr("SQLite connections are read-only. Queries run only when requested."));
    let connected = db.connected.is_some();
    let busy = db.busy();
    let mut connect = false;
    let mut disconnect = false;
    ui.horizontal_wrapped(|ui| {
        ui.add_enabled(!connected && !busy, egui::TextEdit::singleline(&mut db.path).hint_text(tr("SQLite database file")).char_limit(8192).desired_width(340.0));
        connect = ui.add_enabled(!connected && !busy, egui::Button::new(tr("Connect"))).clicked();
        disconnect = ui.add_enabled(connected || busy, egui::Button::new(tr("Disconnect"))).clicked();
    });
    if connect { db.connect(root.as_deref()); }
    if disconnect { db.disconnect(); }
    if db.busy() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(tr("Reading SQLite database…"));
            if ui.button(tr("Cancel Query")).clicked() { db.cancel(); }
        });
    }
    if let Some(error) = &db.error { ui.colored_label(rc_kit::pal(ui).error, error_text(error)); }
    let Some(path) = &db.connected else { return };
    ui.label(path.display().to_string());
    ui.separator();
    let mut action = None;
    ui.horizontal_wrapped(|ui| {
        if ui.add_enabled(!db.busy(), egui::Button::new(tr("Refresh Schema"))).clicked() { action = Some(Action::Schema); }
        let selected = db.tables.get(db.selected).map(|table| table.name.as_str()).unwrap_or_else(|| tr("Select a database table"));
        egui::ComboBox::from_id_salt("sqlite_table").selected_text(selected).width(240.0).show_ui(ui, |ui| {
            for (index, table) in db.tables.iter().enumerate() { ui.selectable_value(&mut db.selected, index, format!("{} ({})", table.name, table.kind)); }
        });
        if ui.add_enabled(!db.busy() && !db.tables.is_empty(), egui::Button::new(tr("Browse Table"))).clicked()
            && let Some(table) = db.tables.get(db.selected)
        { action = Some(Action::Browse(table.name.clone())); }
    });
    if db.schema_truncated { ui.label(tr("Schema list truncated to 200 tables and views.")); }
    if db.tables.is_empty() { ui.label(tr("No user tables or views found.")); }
    if let Some(table) = db.tables.get(db.selected) {
        ui.collapsing(tr("Table Schema"), |ui| {
            egui::ScrollArea::vertical().id_salt("sqlite_schema").max_height(120.0).show(ui, |ui| { ui.monospace(&table.sql); });
        });
    }
    ui.separator();
    if let Some(panel_action) = insight::panels(db, root.as_deref(), ui) { action = Some(panel_action); }
    if db.view != crate::database::View::Query {
        if let Some(action) = action { db.execute(action); }
        return;
    }
    ui.label(tr("SQL Console"));
    let mut editor = egui::TextEdit::multiline(&mut db.sql).id_salt("sqlite_sql_editor").code_editor().desired_rows(3).desired_width(f32::INFINITY).char_limit(MAX_SQL).show(ui);
    if let Some(range) = editor.cursor_range { db.cursor = range.primary.index.into(); }
    ui.horizontal_wrapped(|ui| {
        if ui.button(tr("SQL Suggestions")).clicked() { db.show_suggestions = !db.show_suggestions; }
        if ui.add_enabled(!db.busy() && !db.sql.trim().is_empty(), egui::Button::new(tr("Explain Query Plan"))).clicked() { action = Some(Action::Explain(db.sql.clone())); }
    });
    if db.show_suggestions {
        let choices = crate::database::completions(&db.sql, db.cursor, &db.tables, db.details.as_ref());
        if choices.is_empty() { ui.label(tr("No local suggestions at the caret.")); }
        egui::ScrollArea::vertical().id_salt("sqlite_suggestions").max_height(120.0).show(ui, |ui| { ui.horizontal_wrapped(|ui| {
            for choice in choices {
                if ui.button(&choice.label).clicked()
                    && let Some(cursor) = crate::database::apply_completion(&mut db.sql, &choice) {
                    db.cursor = cursor;
                    editor.state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(cursor))));
                    editor.state.clone().store(ui.ctx(), editor.response.id);
                    editor.response.request_focus();
                    db.show_suggestions = false;
                }
            }
        }); });
        ui.label(tr("Suggestions use loaded table names and inspected columns; they do not validate SQL."));
    }
    if ui.add_enabled(!db.busy() && !db.sql.trim().is_empty() && db.sql.len() <= MAX_SQL, egui::Button::new(tr("Run SQL"))).clicked() { action = Some(Action::Query(db.sql.clone())); }
    if let Some(action) = action { db.execute(action); }
    if let Some(grid) = &db.grid {
        if db.grid_is_plan { ui.strong(tr("SQLite Query Plan")); }
        ui.label(trf("{count} rows displayed", &[("count", &grid.rows.len())]));
        if grid.truncated { ui.label(tr("Results truncated at 500 rows or 2 MiB.")); }
        ui.label(tr("Cells are clipped to one line. Hover for up to 4096 text characters or a BLOB preview."));
        result_table(ui, grid, None);
    }
}

/// Fixed-height virtual rows keep large results from creating every cell each frame.
fn result_table(ui: &mut Ui, grid: &crate::database::Grid, scroll_to: Option<usize>) -> usize {
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
        table.header(22.0, |mut header| {
            for name in &grid.columns { header.col(|ui| { ui.add(egui::Label::new(egui::RichText::new(name).strong()).truncate()).on_hover_text(name); }); }
        }).body(|body| {
            body.rows(22.0, grid.rows.len(), |mut row| {
                rendered += 1;
                for cell in &grid.rows[row.index()] {
                    row.col(|ui| {
                        let preview = cell.display();
                        // Keep one line in the grid; preserve bounded multiline text in the tooltip.
                        let line = preview.replace(['\n', '\r', '\t'], " ");
                        ui.add(egui::Label::new(line).truncate().selectable(true)).on_hover_text(preview);
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
