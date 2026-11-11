//! SQL Server in the Database window: the connect form (with the workspace's appsettings
//! connection strings), the schema tree, the read-only console, results and plans.

use super::*;
use crate::database::{Source, mssql::{self, MsError, Output, Request, SqlServer, conn::{Auth, Kind, ParseError}, guard::GuardError, plan::Plan}};
use egui::{Align2, Rect, Sense, pos2, vec2};

pub(super) fn show(app: &mut DotCraftApp, ui: &mut Ui) {
    let root = app.workspace.as_ref().map(|workspace| workspace.ws.root.clone());
    app.database.mssql.scan(root.as_deref());
    let s = &app.database.mssql;
    let connected = s.connected();
    let busy = s.busy() || s.connecting;
    let tabs = if connected || busy { Vec::new() } else { super::source_tabs(Source::SqlServer) };
    let mut actions = vec![(ico::HIDE, tr("Hide"), true), (ico::REFRESH, tr("Refresh Schema"), connected && !busy)];
    if connected || busy { actions.push((ico::CLOSE, tr("Disconnect"), true)); }
    let status = if connected { s.title.clone() } else { String::new() };
    let click = kit::header(ui, tr("Database"), &tabs, &status, &actions);
    match click.action {
        Some(0) => app.toggle_tool(ToolWindow::Database),
        Some(1) => app.database.mssql.execute(Request::Schema),
        Some(2) => app.database.mssql.disconnect(),
        _ => {}
    }
    if let Some(tab) = click.tab { app.database.source = super::SOURCES[tab]; }
    let database = &mut app.database;
    if !database.mssql.connected() {
        egui::ScrollArea::vertical().id_salt("mssql_connect").auto_shrink([false, false]).show(ui, |ui| connect_form(database, root.as_deref(), ui));
        return;
    }
    connected_view(&mut database.mssql, ui);
}

fn connect_form(database: &mut crate::database::Database, root: Option<&std::path::Path>, ui: &mut Ui) {
    let s = &mut database.mssql;
    let busy = s.busy() || s.connecting;
    let mut pick = None;
    kit::content(ui, |ui| {
        kit::note(ui, tr("SQL Server connections are read-only: only SELECT and WITH queries run, inside a transaction that is always rolled back."));
        kit::note(ui, tr("Connect with a login that only has the db_datareader role."));
    });
    if !s.found.is_empty() && section(ui, "mssql_found", ico::FILE_JSON, tr("Connection Strings in appsettings"), &s.found.len().to_string(), true) {
        for (i, found) in s.found.iter().enumerate() {
            let file = found.file.display().to_string();
            let detail = match &found.kind {
                Kind::SqlServer(Ok(spec)) => format!("{file} · {} · {}", spec.server(), spec.database),
                Kind::SqlServer(Err(ParseError::LocalDb)) => format!("{file} · {}", tr("LocalDB is not supported")),
                Kind::SqlServer(Err(ParseError::NamedPipes)) => format!("{file} · {}", tr("Named pipes are not supported")),
                Kind::SqlServer(Err(_)) => format!("{file} · {}", tr("Not supported")),
                Kind::Sqlite(path) => format!("{file} · SQLite · {path}"),
            };
            let icon = if matches!(found.kind, Kind::Sqlite(_)) { ico::FILE_BINARY } else { ico::TW_DATABASE };
            if kit::row(ui, 1, Some(icon), &found.name, &detail, false).on_hover_text(&detail).clicked() && !busy { pick = Some(i); }
        }
        ui.add_space(4.0);
    }
    if let Some(i) = pick {
        let found = s.found[i].clone();
        match found.kind {
            Kind::SqlServer(Ok(spec)) => { s.form.fill(&spec); s.error = None; }
            Kind::SqlServer(Err(e)) => s.error = Some(MsError::Parse(e)),
            Kind::Sqlite(path) => {
                // Relative SQLite paths are relative to the project folder (the content root).
                let dir = found.file.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
                let full = std::path::Path::new(&path);
                let full = if full.is_absolute() { full.to_path_buf() } else { root.map(|r| r.join(&dir)).unwrap_or(dir).join(full) };
                database.path = full.display().to_string();
                database.source = Source::Sqlite;
                return;
            }
        }
    }
    let s = &mut database.mssql;
    let mut connect = false;
    kit::content(ui, |ui| {
        ui.spacing_mut().item_spacing = vec2(10.0, 8.0);
        ui.horizontal_wrapped(|ui| {
            kit::field(ui, tr("Server"), &mut s.form.server, r"host, host,1433 or host\instance", 260.0, !busy);
            kit::field(ui, tr("Database"), &mut s.form.database, "master", 180.0, !busy);
        });
        ui.horizontal_wrapped(|ui| {
            let p = rc_kit::pal(ui);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.label(RichText::new(tr("Authentication")).size(12.0).color(p.muted));
                let options = vec![tr("SQL Server authentication").to_string(), tr("Windows integrated authentication").to_string()];
                let selected = if s.form.auth == Auth::Sql { &options[0] } else { &options[1] };
                if let Some(i) = kit::dropdown(ui, "mssql_auth", selected, &options, !busy) { s.form.auth = if i == 0 { Auth::Sql } else { Auth::Integrated }; }
            });
            if s.form.auth == Auth::Sql {
                kit::field(ui, tr("User"), &mut s.form.user, "reader", 160.0, !busy);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    let caption = ui.label(RichText::new(tr("Password")).size(12.0).color(p.muted));
                    let field = ui.add_enabled(!busy, egui::TextEdit::singleline(&mut s.form.password).password(true).hint_text(tr("Saved in the keychain")).desired_width(180.0).margin(vec2(6.0, 4.0))).labelled_by(caption.id);
                    if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) { connect = true; }
                });
            }
        });
        if s.form.auth == Auth::Integrated && !cfg!(windows) { kit::message(ui, tr("Windows integrated authentication is only available on Windows."), false); }
        ui.horizontal_wrapped(|ui| {
            ui.add_enabled(!busy, egui::Checkbox::new(&mut s.form.trust_cert, tr("Trust server certificate")));
            let mut encrypt = !s.form.no_encrypt;
            if ui.add_enabled(!busy, egui::Checkbox::new(&mut encrypt, tr("Encrypt connection"))).changed() { s.form.no_encrypt = !encrypt; }
            if s.form.auth == Auth::Sql { ui.add_enabled(!busy, egui::Checkbox::new(&mut s.form.remember, tr("Remember password in the system keychain"))); }
        });
    });
    kit::toolbar(ui, |ui| {
        let can = !busy && !s.form.server.trim().is_empty();
        if kit::button(ui, Some(ico::OPEN), tr("Connect"), can).clicked() { connect = true; }
        if s.form.auth == Auth::Sql && kit::button(ui, Some(ico::DELETE), tr("Forget Saved Password"), can && !s.form.user.trim().is_empty()).clicked() { s.forget_password(); }
        if busy {
            ui.spinner();
            kit::note(ui, tr("Connecting to SQL Server…"));
            if kit::button(ui, Some(ico::STOP), tr("Cancel"), true).clicked() { s.cancel(); }
        }
    });
    if connect && !busy { s.connect(); }
    kit::content(ui, |ui| {
        if let Some(error) = &s.error { kit::message(ui, &error_text(error), true); }
        kit::note(ui, tr("LocalDB and named pipes are not supported: connect over TCP (enable TCP/IP for the instance)."));
    });
}

fn connected_view(s: &mut SqlServer, ui: &mut Ui) {
    let wide = ui.available_width() >= 760.0;
    kit::content(ui, |ui| {
        if s.info.as_ref().is_some_and(|i| i.can_write) {
            kit::message(ui, tr("This login can change the database. Queries are still guarded and rolled back, but a db_datareader login is recommended."), false);
        } else {
            kit::note(ui, tr("Read-only: only SELECT and WITH queries run, inside a transaction that is always rolled back."));
        }
        if let Some(e) = &s.keychain_error { kit::message(ui, &trf("Could not save the password in the keychain: {error}", &[("error", e)]), false); }
        if let Some(info) = &s.info { kit::note(ui, &format!("{} · {} · SQL Server {} {}", info.login, info.database, info.version, info.edition)); }
    });
    if wide {
        let height = ui.available_height();
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(vec2(300.0, height), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(300.0);
                // Long column types and key names are cut at the pane's edge (hover shows them).
                ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                tree(s, ui, height - 40.0);
            });
            ui.allocate_ui_with_layout(vec2(ui.available_width(), height), egui::Layout::top_down(egui::Align::Min), |ui| {
                egui::ScrollArea::vertical().id_salt("mssql_console").auto_shrink([false, false]).show(ui, |ui| console(s, ui));
            });
        });
    } else {
        egui::ScrollArea::vertical().id_salt("mssql_console").auto_shrink([false, false]).show(ui, |ui| {
            tree(s, ui, 220.0);
            console(s, ui);
        });
    }
}

fn tree(s: &mut SqlServer, ui: &mut Ui, height: f32) {
    let filter = s.filter.to_lowercase();
    let views = s.schema.tables.iter().filter(|t| t.view).count();
    kit::toolbar(ui, |ui| {
        kit::search(ui, &mut s.filter, tr("Filter tables"), 160.0);
        kit::note(ui, &trf("{tables} tables, {views} views", &[("tables", &(s.schema.tables.len() - views)), ("views", &views)]));
    });
    if s.schema.truncated { kit::content(ui, |ui| kit::message(ui, &trf("Schema list truncated to {count} tables and views.", &[("count", &mssql::MAX_TABLES)]), false)); }
    let mut toggle = None;
    let mut select = None;
    egui::ScrollArea::vertical().id_salt("mssql_tree").max_height(height.max(80.0)).auto_shrink([false, true]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        if s.schema.tables.is_empty() { kit::content(ui, |ui| kit::note(ui, tr("No user tables or views found."))); }
        let mut current = "";
        let mut open = false;
        for (i, table) in s.schema.tables.iter().enumerate() {
            if !filter.is_empty() && !table.name.to_lowercase().contains(&filter) { continue; }
            if table.schema != current {
                current = &table.schema;
                let key = format!("s:{current}");
                open = s.expanded.contains(&key) || !filter.is_empty();
                let count = s.schema.tables.iter().filter(|t| t.schema == table.schema).count();
                let mut now = open;
                kit::section(ui, ico::NAMESPACE, current, &count.to_string(), &mut now);
                if now != open { toggle = Some(key); }
            }
            if !open { continue; }
            let key = format!("t:{}", table.qualified());
            let selected = s.selected == Some(i);
            let secondary = if table.view { tr("view").to_string() } else { table.columns.len().to_string() };
            if kit::row(ui, 1, Some(if table.view { ico::PREVIEW } else { ico::DATA_TABLE }), &table.name, &secondary, selected).clicked() {
                select = Some(i);
                toggle = Some(key.clone());
            }
            if s.expanded.contains(&key) {
                for column in &table.columns {
                    let mut detail = column.data_type.clone();
                    if column.primary_key { detail.push_str(" · PK"); }
                    if column.nullable { detail.push_str(" · null"); }
                    if let Some(target) = &column.references { detail.push_str(&format!(" · → {target}")); }
                    kit::row(ui, 2, Some(ico::DATA_COLUMN), &column.name, &detail, false).on_hover_text(format!("{} {detail}", column.name));
                }
                for key in &table.keys {
                    let short = match key.kind.as_str() { "PRIMARY KEY" => "PK", "FOREIGN KEY" => "FK", "UNIQUE" => "UQ", other => other };
                    let detail = match &key.target { Some(t) => format!("{short} → {t}"), None => format!("{short} ({})", key.columns.join(", ")) };
                    let full = match &key.target { Some(t) => format!("{} {} ({}) → {t}", key.name, key.kind, key.columns.join(", ")), None => format!("{} {} ({})", key.name, key.kind, key.columns.join(", ")) };
                    kit::row(ui, 2, Some(ico::TAG), &key.name, &detail, false).on_hover_text(full);
                }
            }
        }
    });
    if let Some(key) = toggle && !s.expanded.remove(&key) { s.expanded.insert(key); }
    if let Some(i) = select { s.selected = Some(i); }
}

fn console(s: &mut SqlServer, ui: &mut Ui) {
    let busy = s.busy();
    kit::content(ui, |ui| {
        egui::TextEdit::multiline(&mut s.sql).id_salt("mssql_sql_editor").code_editor().desired_rows(5).desired_width(f32::INFINITY).margin(egui::vec2(6.0, 4.0)).char_limit(MAX_SQL).show(ui);
    });
    let mut request = None;
    kit::toolbar(ui, |ui| {
        let can = !busy && !s.sql.trim().is_empty();
        if kit::button(ui, Some(ico::RUN), tr("Run SQL"), can).clicked() { request = Some(Request::Query(s.sql.clone())); }
        if kit::button(ui, Some(ico::EVALUATE), tr("Execution Plan"), can).clicked() { request = Some(Request::Plan(s.sql.clone())); }
        let table = s.selected.and_then(|i| s.schema.tables.get(i));
        if kit::button(ui, Some(ico::PREVIEW), tr("Browse Table"), !busy && table.is_some()).clicked()
            && let Some(table) = table
        { request = Some(Request::Browse(table.schema.clone(), table.name.clone())); }
        if busy {
            ui.spinner();
            kit::note(ui, tr("Running on SQL Server…"));
            if kit::button(ui, Some(ico::STOP), tr("Cancel Query"), true).clicked() { s.cancel(); }
        }
    });
    if let Some(request) = request { s.execute(request); }
    if let Some(error) = &s.error { kit::content(ui, |ui| kit::message(ui, &error_text(error), true)); }
    match &s.output {
        Some(Output::Grid(grid)) => {
            kit::toolbar(ui, |ui| {
                kit::note(ui, &trf("{count} rows displayed", &[("count", &grid.rows.len())]));
                if grid.truncated { kit::message(ui, tr("Results truncated at 500 rows or 2 MiB."), false); }
                kit::note(ui, tr("The transaction was rolled back."));
            });
            egui::Frame::new().inner_margin(egui::Margin { left: 10, right: 10, top: 0, bottom: 8 }).show(ui, |ui| result_table(ui, grid, None));
        }
        Some(Output::Plan(plan)) => plan_view(ui, plan),
        None => {}
    }
}

/// The estimated plan as an indented operator tree with each operator's share of the cost.
fn plan_view(ui: &mut Ui, plan: &Plan) {
    kit::toolbar(ui, |ui| {
        ui.label(RichText::new(tr("Estimated Execution Plan")).font(theme::bold(12.5)));
        kit::note(ui, &trf("Subtree cost {cost}", &[("cost", &format!("{:.4}", plan.total_cost))]));
        kit::note(ui, tr("The query was compiled, not executed."));
    });
    if !plan.missing_indexes.is_empty() {
        kit::content(ui, |ui| {
            for index in &plan.missing_indexes { kit::message(ui, &trf("Missing index (impact {index})", &[("index", index)]), false); }
        });
    }
    let p = rc_kit::pal(ui);
    egui::Frame::new().inner_margin(egui::Margin { left: 6, right: 10, top: 0, bottom: 8 }).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for node in &plan.nodes {
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(420.0), 24.0), Sense::hover());
            if resp.hovered() { ui.painter().rect_filled(r.shrink2(vec2(4.0, 0.0)), 4.0, p.row_hover); }
            let x = r.left() + 10.0 + node.depth as f32 * 18.0;
            if node.depth > 0 {
                ui.painter().line_segment([pos2(x - 10.0, r.top()), pos2(x - 10.0, r.center().y)], egui::Stroke::new(1.0, p.control_border));
                ui.painter().line_segment([pos2(x - 10.0, r.center().y), pos2(x - 3.0, r.center().y)], egui::Stroke::new(1.0, p.control_border));
            }
            let icon = if node.warnings.is_empty() { ico::EVALUATE } else { ico::WARNING };
            rc_kit::icons::paint(ui.painter(), Rect::from_center_size(pos2(x + 8.0, r.center().y), vec2(16.0, 16.0)), icon, &p, true);
            let g = ui.painter().layout_no_wrap(node.physical.clone(), theme::bold(12.5), p.text);
            ui.painter().galley(pos2(x + 24.0, (r.center().y - g.size().y / 2.0).round()), g.clone(), p.text);
            let mut detail = Vec::new();
            if node.logical != node.physical && !node.logical.is_empty() { detail.push(node.logical.clone()); }
            if let Some(object) = &node.object { detail.push(object.clone()); }
            let detail = detail.join(" · ");
            // Cost bar and numbers on the right.
            let right = r.right() - 8.0;
            let rows = trf("{rows} rows", &[("rows", &format_rows(node.est_rows))]);
            let rows_g = ui.painter().layout_no_wrap(rows.clone(), FontId::proportional(12.0), p.muted);
            ui.painter().galley(pos2(right - rows_g.size().x, r.center().y - rows_g.size().y / 2.0), rows_g.clone(), p.muted);
            let pct = format!("{:.0}%", node.cost_percent);
            let bar = Rect::from_min_size(pos2(right - 110.0 - 70.0, r.center().y - 4.0), vec2(70.0, 8.0));
            ui.painter().rect_filled(bar, 3.0, p.row_hover);
            ui.painter().rect_filled(Rect::from_min_size(bar.min, vec2(70.0 * (node.cost_percent / 100.0).clamp(0.0, 1.0) as f32, 8.0)), 3.0, if node.cost_percent >= 50.0 { p.warn } else { p.accent });
            ui.painter().text(pos2(bar.left() - 8.0, r.center().y), Align2::RIGHT_CENTER, &pct, theme::bold(12.0), p.text);
            let detail_x = x + 32.0 + g.size().x;
            let detail_w = bar.left() - 50.0 - detail_x;
            if !detail.is_empty() && detail_w > 30.0 {
                let mut job = egui::text::LayoutJob::simple_singleline(detail.clone(), FontId::proportional(12.0), p.muted);
                job.wrap.max_width = detail_w;
                job.wrap.max_rows = 1;
                job.wrap.break_anywhere = true;
                job.wrap.overflow_character = Some('…');
                let dg = ui.painter().layout_job(job);
                ui.painter().galley(pos2(detail_x, r.center().y - dg.size().y / 2.0), dg, p.muted);
            }
            let label = format!("{} {detail} {pct} {rows}", node.physical);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &node.physical));
            let mut tip = label;
            for w in &node.warnings { tip.push_str(&format!("\n⚠ {w}")); }
            resp.on_hover_text(tip);
            for w in &node.warnings {
                let (wr, _) = ui.allocate_exact_size(vec2(ui.available_width().max(420.0), 20.0), Sense::hover());
                ui.painter().text(pos2(x + 24.0, wr.center().y), Align2::LEFT_CENTER, w, FontId::proportional(12.0), p.warn);
            }
        }
    });
    if !plan.statement.is_empty() {
        kit::content(ui, |ui| {
            let shown: String = plan.statement.chars().take(2000).collect();
            ui.label(RichText::new(shown).monospace().size(12.0).color(rc_kit::pal(ui).muted));
        });
    }
}

fn format_rows(rows: f64) -> String {
    if rows >= 1_000_000.0 { format!("{:.1}M", rows / 1_000_000.0) } else if rows >= 10_000.0 { format!("{:.0}K", rows / 1000.0) } else if rows.fract() == 0.0 { format!("{rows:.0}") } else { format!("{rows:.1}") }
}

fn error_text(error: &MsError) -> String {
    match error {
        MsError::Parse(ParseError::LocalDb) => tr("LocalDB is not supported: it only listens on a named pipe. Use a SQL Server instance with TCP/IP enabled (for example SQL Server Express or a Docker container).").into(),
        MsError::Parse(ParseError::NamedPipes) => tr("Named pipes and shared memory are not supported: connect over TCP.").into(),
        MsError::Parse(ParseError::Unsupported(what)) => trf("This connection string option is not supported: {option}", &[("option", what)]),
        MsError::Parse(ParseError::MissingServer) => tr("Enter the server name, optionally with a port or an instance name.").into(),
        MsError::Parse(ParseError::BadPort(port)) => trf("Invalid port: {port}", &[("port", port)]),
        MsError::Parse(ParseError::Syntax) => tr("The connection string is not valid.").into(),
        MsError::Guard(GuardError::Empty) => tr("Enter a SELECT or WITH query.").into(),
        MsError::Guard(GuardError::NotSelect(word)) => trf("Only SELECT and WITH queries are allowed, not {keyword}.", &[("keyword", word)]),
        MsError::Guard(GuardError::Forbidden(word)) => trf("{keyword} is not allowed in a read-only query.", &[("keyword", word)]),
        MsError::Guard(GuardError::Multiple) => tr("Run one statement at a time.").into(),
        MsError::Guard(GuardError::Unterminated) => tr("The query has an unterminated string, identifier or comment.").into(),
        MsError::IntegratedUnavailable => tr("Windows integrated authentication is only available on Windows.").into(),
        MsError::NoPassword => tr("Enter the password: none is saved in the keychain for this login.").into(),
        MsError::Connect(e) => trf("Could not reach SQL Server: {error}", &[("error", e)]),
        MsError::Tls(e) => trf("TLS error: {error}. If the server uses a self-signed certificate, enable Trust server certificate.", &[("error", e)]),
        MsError::Server(e) => trf("SQL Server error: {error}", &[("error", e)]),
        MsError::Cancelled => tr("Query cancelled.").into(),
        MsError::TimedOut => tr("SQL Server did not answer within the time limit.").into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::mssql::ServerInfo;
    use egui_kittest::{Harness, kittest::Queryable};
    use rc_core::{Settings, Waker};

    #[test]
    fn sql_server_form_tree_and_guard_work_without_a_server() {
        let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let mut app = DotCraftApp::with_settings(Settings { language: Some("en".into()), ..Settings::default() }, Waker::noop(), None);
        app.database.mssql = SqlServer::with_secrets(Waker::noop(), std::sync::Arc::new(mssql::secrets::Memory::default()));
        app.database.mssql.set_found(vec![mssql::conn::Found { file: "src/Api/appsettings.json".into(), name: "Shop".into(), kind: mssql::conn::classify("Server=sql01,1433;Database=Shop;User Id=reader;Password=x").unwrap() }]);
        let mut first = true;
        let mut harness = Harness::builder().with_size(egui::vec2(1100.0, 800.0)).build_ui_state(move |ui, app: &mut DotCraftApp| {
            if std::mem::take(&mut first) { rc_kit::theme::install_fonts(ui.ctx()); return; }
            app.database.update();
            super::super::show(app, ui);
        }, app);
        harness.run_steps(2);
        harness.get_by_label("SQL Server").click();
        harness.run_steps(2);
        assert_eq!(harness.state().database.source, Source::SqlServer);
        harness.get_by_label("Shop").click();
        harness.run_steps(2);
        assert_eq!((harness.state().database.mssql.form.server.as_str(), harness.state().database.mssql.form.database.as_str()), ("sql01,1433", "Shop"));
        harness.state_mut().database.mssql.form.server = r"(localdb)\MSSQLLocalDB".into();
        harness.run_steps(1);
        harness.get_by_label("Connect").click();
        harness.run_steps(2);
        harness.get_by_label_contains("LocalDB is not supported: it only listens");
        harness.state_mut().database.mssql.set_connected_for_test(ServerInfo { login: "reader".into(), database: "Shop".into(), ..ServerInfo::default() }, "sql01 · Shop", mssql::sample_schema());
        harness.run_steps(2);
        harness.get_by_label("Orders").click();
        harness.run_steps(2);
        harness.get_by_label("CustomerId");
        assert_eq!(harness.state().database.mssql.selected, Some(1));
        harness.state_mut().database.mssql.sql = "UPDATE dbo.Orders SET Total = 0".into();
        harness.run_steps(1);
        harness.get_by_label("Run SQL").click();
        harness.run_steps(2);
        harness.get_by_label("Only SELECT and WITH queries are allowed, not UPDATE.");
        assert!(!harness.state().database.mssql.busy());
        harness.get_by_label("Disconnect").click();
        harness.run_steps(2);
        assert!(!harness.state().database.mssql.connected());
    }
}
