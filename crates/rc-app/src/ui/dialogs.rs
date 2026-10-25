//! Modal dialogs and small popups: Settings, About, Go to Line, New File/Folder, Rename,
//! Delete and Recent Files.

use crate::app::DotCraftApp;
use egui::{Align2, Context, FontId, Id, Key, Ui, vec2};
use rc_core::Settings;
use rc_core::settings::{KeymapChoice, ThemeChoice};
use rc_kit::i18n::{Lang, tr, trf};
use rc_kit::{icons, theme};
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SettingsPage {
    Appearance,
    Editor,
    Keymap,
    Tools,
}

pub enum Dialog {
    Settings { draft: Box<Settings>, page: SettingsPage },
    About,
    GotoLine { text: String, focus: bool },
    NewFile { dir: PathBuf, name: String, folder: bool, error: Option<String> },
    Rename { path: PathBuf, name: String, error: Option<String> },
    ConfirmDelete { path: PathBuf },
    RecentFiles { selected: usize, filter: String },
    /// ⌥⏎ context actions for the active editor.
    CodeActions { list: Vec<rc_lsp::lsp_types::CodeActionOrCommand>, selected: usize },
    /// Shift+F6 on a symbol.
    RenameSymbol { doc: u64, pos: usize, original: String, name: String, focus: bool },
    /// Confirm rolling back files to HEAD.
    Rollback { paths: Vec<String> },
    NewBranch { name: String },
    /// ⇧⌘E: the places the caret jumped from.
    RecentLocations { selected: usize },
    /// ⌘F12 file structure popup.
    FileStructure { filter: String, selected: usize },
    /// Right-click on a breakpoint: condition and enabled state (0-based line).
    BreakpointProperties { path: PathBuf, line: usize, condition: String, enabled: bool, focus: bool },
    /// A compound run configuration: its name and which applications it starts.
    Compound { old: Option<String>, name: String, members: Vec<(String, bool)> },
    /// ⌥F8: evaluate an expression in the selected frame.
    Evaluate { expression: String, focus: bool },
}

impl Dialog {
    pub fn settings(s: &Settings) -> Dialog {
        Dialog::Settings { draft: Box::new(s.clone()), page: SettingsPage::Appearance }
    }
}

enum Outcome {
    Keep,
    Close,
}

pub fn show(app: &mut DotCraftApp, ctx: &Context) {
    let Some(mut dialog) = app.dialog.take() else { return };
    let outcome = match &mut dialog {
        Dialog::Settings { draft, page } => settings(app, ctx, draft, page),
        Dialog::About => about(ctx),
        Dialog::GotoLine { text, focus } => goto_line(app, ctx, text, focus),
        Dialog::NewFile { dir, name, folder, error } => new_file(app, ctx, dir, name, *folder, error),
        Dialog::Rename { path, name, error } => rename(app, ctx, path, name, error),
        Dialog::ConfirmDelete { path } => confirm_delete(app, ctx, path),
        Dialog::RecentFiles { selected, filter } => recent_files(app, ctx, selected, filter),
        Dialog::CodeActions { list, selected } => code_actions(app, ctx, list, selected),
        Dialog::RenameSymbol { doc, pos, original, name, focus } => rename_symbol(app, ctx, *doc, *pos, original, name, focus),
        Dialog::FileStructure { filter, selected } => file_structure(app, ctx, filter, selected),
        Dialog::Rollback { paths } => rollback(app, ctx, paths),
        Dialog::NewBranch { name } => new_branch(app, ctx, name),
        Dialog::RecentLocations { selected } => recent_locations(app, ctx, selected),
        Dialog::BreakpointProperties { path, line, condition, enabled, focus } => breakpoint_properties(app, ctx, path, *line, condition, enabled, focus),
        Dialog::Evaluate { expression, focus } => evaluate(app, ctx, expression, focus),
        Dialog::Compound { old, name, members } => compound(app, ctx, old.as_deref(), name, members),
    };
    if matches!(outcome, Outcome::Keep) && app.dialog.is_none() {
        app.dialog = Some(dialog);
    } else if matches!(outcome, Outcome::Close) {
        app.focus_editor = true;
    }
}

fn modal<R>(ctx: &Context, id: &str, width: f32, add: impl FnOnce(&mut Ui) -> R) -> (R, bool) {
    let resp = egui::Modal::new(Id::new(id)).show(ctx, |ui| {
        ui.set_width(width);
        add(ui)
    });
    let escape = resp.should_close();
    (resp.inner, escape)
}

fn title(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text).font(theme::bold(15.0)));
    ui.add_space(10.0);
}

fn buttons(ui: &mut Ui, ok: &str, enabled: bool) -> (bool, bool) {
    ui.add_space(14.0);
    let mut res = (false, false);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        res.0 = ui.add_enabled_ui(enabled, |ui| theme::primary_button(ui, ok)).inner.clicked();
        res.1 = theme::secondary_button(ui, tr("Cancel")).clicked();
    });
    res
}

fn settings(app: &mut DotCraftApp, ctx: &Context, draft: &mut Settings, page: &mut SettingsPage) -> Outcome {
    let p = rc_kit::theme::palette(ctx.global_style().visuals.dark_mode);
    let ((ok, cancel), esc) = modal(ctx, "settings", 720.0, |ui| {
        title(ui, tr("Settings"));
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(170.0);
                for (pg, label) in [
                    (SettingsPage::Appearance, tr("Appearance")),
                    (SettingsPage::Editor, tr("Editor")),
                    (SettingsPage::Keymap, tr("Keymap")),
                    (SettingsPage::Tools, tr("Tools")),
                ] {
                    let (r, resp) = ui.allocate_exact_size(vec2(170.0, 28.0), egui::Sense::click());
                    if *page == pg {
                        ui.painter().rect_filled(r, 6.0, p.selection);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(r, 6.0, p.row_hover);
                    }
                    ui.painter().text(r.left_center() + vec2(10.0, 0.0), Align2::LEFT_CENTER, label, FontId::proportional(13.0), p.text);
                    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, *page == pg, label));
                    if resp.clicked() {
                        *page = pg;
                    }
                }
            });
            ui.add_space(16.0);
            ui.vertical(|ui| {
                ui.set_min_height(320.0);
                ui.set_width(500.0);
                egui::Grid::new("settings_grid").num_columns(2).spacing(vec2(16.0, 10.0)).show(ui, |ui| match page {
                    SettingsPage::Appearance => {
                        ui.label(tr("Theme"));
                        egui::ComboBox::from_id_salt("theme")
                            .selected_text(match draft.theme {
                                ThemeChoice::Light => "Islands Light",
                                ThemeChoice::Dark => "Islands Dark",
                                ThemeChoice::System => tr("Sync with OS"),
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut draft.theme, ThemeChoice::Light, "Islands Light");
                                ui.selectable_value(&mut draft.theme, ThemeChoice::Dark, "Islands Dark");
                                ui.selectable_value(&mut draft.theme, ThemeChoice::System, tr("Sync with OS"));
                            });
                        ui.end_row();
                        ui.label(tr("Language"));
                        let current = draft.language.as_deref().map(Lang::from_locale);
                        egui::ComboBox::from_id_salt("lang")
                            .selected_text(current.map_or(tr("System language"), |l| l.native_name()))
                            .show_ui(ui, |ui| {
                                if ui.selectable_label(current.is_none(), tr("System language")).clicked() {
                                    draft.language = None;
                                }
                                for l in Lang::ALL {
                                    if ui.selectable_label(current == Some(l), l.native_name()).clicked() {
                                        draft.language = Some(lang_code(l).to_string());
                                    }
                                }
                            });
                        ui.end_row();
                        ui.label(tr("Zoom"));
                        theme::slider(ui, egui::Slider::new(&mut draft.ui_zoom, 0.5..=3.0).step_by(0.1).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
                        ui.end_row();
                    }
                    SettingsPage::Editor => {
                        ui.label(tr("Font size"));
                        ui.add(egui::DragValue::new(&mut draft.editor_font_size).range(8.0..=32.0).speed(0.5).suffix(" pt"));
                        ui.end_row();
                        ui.label(tr("Line height"));
                        ui.add(egui::DragValue::new(&mut draft.editor_line_spacing).range(1.0..=2.0).speed(0.05));
                        ui.end_row();
                        ui.label(tr("Hard wrap guide"));
                        ui.add(egui::DragValue::new(&mut draft.right_margin).range(0..=400).suffix(tr(" columns")));
                        ui.end_row();
                        ui.label("");
                        ui.checkbox(&mut draft.show_inlay_hints, tr("Show inlay hints"));
                        ui.end_row();
                        ui.label("");
                        ui.checkbox(&mut draft.show_indent_guides, tr("Show indent guides"));
                        ui.end_row();
                        ui.label("");
                        ui.checkbox(&mut draft.autosave, tr("Save files when switching to another application"));
                        ui.end_row();
                        ui.label("");
                        ui.checkbox(&mut draft.single_click_open, tr("Open files with a single click in the Explorer"));
                        ui.end_row();
                    }
                    SettingsPage::Keymap => {
                        ui.label(tr("Keymap"));
                        egui::ComboBox::from_id_salt("keymap")
                            .selected_text(match draft.keymap {
                                KeymapChoice::Rider => if cfg!(target_os = "macos") { "Rider (macOS)" } else { "Rider" },
                                KeymapChoice::VisualStudio => "Visual Studio",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut draft.keymap, KeymapChoice::Rider, if cfg!(target_os = "macos") { "Rider (macOS)" } else { "Rider" });
                                ui.selectable_value(&mut draft.keymap, KeymapChoice::VisualStudio, "Visual Studio");
                            });
                        ui.end_row();
                    }
                    SettingsPage::Tools => {
                        for (label, value) in [
                            (tr(".NET CLI"), &mut draft.dotnet_path),
                            (tr("Roslyn language server"), &mut draft.roslyn_path),
                            (tr("netcoredbg debugger"), &mut draft.netcoredbg_path),
                        ] {
                            ui.label(label);
                            ui.horizontal(|ui| {
                                let mut text = value.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
                                let resp = ui.add(egui::TextEdit::singleline(&mut text).hint_text(tr("Detect automatically")).desired_width(300.0));
                                if resp.changed() {
                                    *value = (!text.trim().is_empty()).then(|| PathBuf::from(text.trim()));
                                }
                                if ui.button(tr("Browse…")).clicked()
                                    && let Some(p) = rfd::FileDialog::new().pick_file()
                                {
                                    *value = Some(p);
                                }
                            });
                            ui.end_row();
                        }
                    }
                });
            });
        });
        buttons(ui, tr("OK"), true)
    });
    if ok {
        app.apply_settings(ctx, draft.clone());
        return Outcome::Close;
    }
    if cancel || esc { Outcome::Close } else { Outcome::Keep }
}

fn lang_code(l: Lang) -> &'static str {
    match l {
        Lang::En => "en",
        Lang::Es => "es",
        Lang::Fr => "fr",
        Lang::Pt => "pt-BR",
        Lang::De => "de",
        Lang::Zh => "zh-Hans",
        Lang::Ja => "ja",
        Lang::Pl => "pl",
    }
}

fn about(ctx: &Context) -> Outcome {
    let (close, esc) = modal(ctx, "about", 380.0, |ui| {
        ui.vertical_centered(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(64.0, 64.0), egui::Sense::hover());
            rc_kit::branding::paint(ui, r);
            ui.add_space(8.0);
            ui.label(egui::RichText::new("DotCraft").font(theme::bold(18.0)));
            ui.label(trf("Version {version}", &[("version", &env!("CARGO_PKG_VERSION"))]));
            ui.add_space(6.0);
            ui.label(egui::RichText::new(tr("A lightweight IDE for .NET")).color(rc_kit::pal(ui).muted));
            ui.add_space(6.0);
            ui.label(egui::RichText::new(tr("Icons: IntelliJ Platform (Apache 2.0)")).size(11.5).color(rc_kit::pal(ui).muted));
            ui.add_space(12.0);
            theme::primary_button(ui, tr("Close")).clicked()
        })
        .inner
    });
    if close || esc { Outcome::Close } else { Outcome::Keep }
}

fn goto_line(app: &mut DotCraftApp, ctx: &Context, text: &mut String, focus: &mut bool) -> Outcome {
    let Some(doc) = app.docs.active_doc() else { return Outcome::Close };
    let lines = doc.editor.buffer.len_lines();
    let ((ok, cancel), esc) = modal(ctx, "goto_line", 340.0, |ui| {
        title(ui, tr("Go to Line:Column"));
        let resp = ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY));
        if std::mem::take(focus) {
            resp.request_focus();
        }
        ui.label(egui::RichText::new(trf("1 to {n}", &[("n", &lines)])).size(12.0).color(rc_kit::pal(ui).muted));
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        let (ok, cancel) = buttons(ui, tr("OK"), true);
        (ok || enter, cancel)
    });
    if ok {
        let mut parts = text.split([':', ',']);
        let line = parts.next().and_then(|s| s.trim().parse::<usize>().ok());
        let col = parts.next().and_then(|s| s.trim().parse::<usize>().ok()).unwrap_or(1);
        if let Some(line) = line {
            app.push_nav();
            if let Some(d) = app.docs.active_doc_mut() {
                d.editor.goto(line.saturating_sub(1), col.saturating_sub(1));
            }
        }
        return Outcome::Close;
    }
    if cancel || esc { Outcome::Close } else { Outcome::Keep }
}

fn compound(app: &mut DotCraftApp, ctx: &Context, old: Option<&str>, name: &mut String, members: &mut [(String, bool)]) -> Outcome {
    let taken = app.runner.configs.iter().any(|c| c.name == name.trim() && Some(c.name.as_str()) != old);
    let chosen = members.iter().filter(|(_, on)| *on).count();
    let valid = !name.trim().is_empty() && !taken && chosen > 0;
    let ((ok, cancel), esc) = modal(ctx, "compound", 420.0, |ui| {
        title(ui, if old.is_some() { tr("Edit Compound Configuration") } else { tr("Add Compound Configuration") });
        ui.label(tr("Name"));
        ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
        ui.add_space(8.0);
        ui.label(tr("Start these applications together:"));
        for (n, on) in members.iter_mut() {
            ui.checkbox(on, n.as_str());
        }
        if taken {
            ui.label(egui::RichText::new(tr("A configuration with that name already exists")).size(12.5).color(rc_kit::pal(ui).error));
        }
        buttons(ui, tr("OK"), valid)
    });
    if ok && valid {
        let list = members.iter().filter(|(_, on)| *on).map(|(n, _)| n.clone()).collect();
        app.runner.save_compound(old, name.trim().to_string(), list);
        app.session_dirty = true;
        return Outcome::Close;
    }
    if cancel || esc { Outcome::Close } else { Outcome::Keep }
}

fn breakpoint_properties(app: &mut DotCraftApp, ctx: &Context, path: &std::path::Path, line: usize, condition: &mut String, enabled: &mut bool, focus: &mut bool) -> Outcome {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let ((ok, cancel, remove), esc) = modal(ctx, "breakpoint_properties", 420.0, |ui| {
        title(ui, &trf("Breakpoint at {file}:{line}", &[("file", &name), ("line", &(line + 1))]));
        ui.checkbox(enabled, tr("Enabled"));
        ui.add_space(6.0);
        ui.label(tr("Condition"));
        let resp = ui.add(egui::TextEdit::singleline(condition).hint_text(tr("Stop only when this C# expression is true")).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY));
        if std::mem::take(focus) {
            resp.request_focus();
        }
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        let mut remove = false;
        ui.add_space(14.0);
        let mut res = (false, false);
        ui.horizontal(|ui| {
            remove = theme::secondary_button(ui, tr("Remove")).clicked();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                res.0 = theme::primary_button(ui, tr("Done")).clicked();
                res.1 = theme::secondary_button(ui, tr("Cancel")).clicked();
            });
        });
        (res.0 || enter, res.1, remove)
    });
    if ok || remove {
        let key = (path.to_path_buf(), line);
        let cond = condition.trim().to_string();
        if let Some(i) = app.docs.index_of(path) {
            let bps = &mut app.docs.list[i].editor.breakpoints;
            if remove {
                bps.remove(&line);
                app.debugger.conditions.remove(&key);
            } else {
                let kind = if !*enabled {
                    rc_editor::BreakpointKind::Disabled
                } else if cond.is_empty() {
                    rc_editor::BreakpointKind::Normal
                } else {
                    rc_editor::BreakpointKind::Conditional
                };
                bps.insert(line, kind);
                if cond.is_empty() {
                    app.debugger.conditions.remove(&key);
                } else {
                    app.debugger.conditions.insert(key, cond);
                }
            }
            app.session_dirty = true;
        }
        return Outcome::Close;
    }
    if cancel || esc { Outcome::Close } else { Outcome::Keep }
}

fn evaluate(app: &mut DotCraftApp, ctx: &Context, expression: &mut String, focus: &mut bool) -> Outcome {
    let stopped = app.debugger.stopped();
    let mut eval = false;
    let mut watch = false;
    let (close, esc) = modal(ctx, "evaluate", 560.0, |ui| {
        title(ui, tr("Evaluate Expression"));
        let resp = ui.add(egui::TextEdit::singleline(expression).hint_text(tr("Expression")).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY));
        if std::mem::take(focus) {
            resp.request_focus();
        }
        if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            eval = true;
            resp.request_focus();
        }
        ui.add_space(8.0);
        let p = rc_kit::pal(ui);
        egui::Frame::new().fill(p.control_bg).corner_radius(6.0).inner_margin(8.0).show(ui, |ui| {
            ui.set_min_height(120.0);
            ui.set_width(ui.available_width());
            if !stopped {
                ui.label(egui::RichText::new(tr("The program must be paused to evaluate expressions")).color(p.muted));
            }
            match app.debugger.eval.clone() {
                Some((_, e, None)) => {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(12.0));
                        ui.label(egui::RichText::new(e).monospace());
                    });
                }
                Some((_, e, Some(Ok(v)))) => {
                    crate::ui::debug_window::eval_tree(ui, &mut app.debugger, &e, &v);
                }
                Some((_, _, Some(Err(err)))) => {
                    ui.label(egui::RichText::new(err).color(p.error));
                }
                None => {}
            }
        });
        ui.add_space(14.0);
        let mut close = false;
        ui.horizontal(|ui| {
            if ui.add_enabled_ui(stopped && !expression.trim().is_empty(), |ui| theme::secondary_button(ui, tr("Add to Watches"))).inner.clicked() {
                watch = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                close = theme::secondary_button(ui, tr("Close")).clicked();
                if ui.add_enabled_ui(stopped && !expression.trim().is_empty(), |ui| theme::primary_button(ui, tr("Evaluate"))).inner.clicked() {
                    eval = true;
                }
            });
        });
        close
    });
    if eval && stopped && !expression.trim().is_empty() {
        app.debugger.evaluate(expression.trim().to_string());
    }
    if watch && let Some(s) = &mut app.debugger.session {
        s.add_watch(expression.trim().to_string());
    }
    if close || esc { Outcome::Close } else { Outcome::Keep }
}

/// A new file name gets `.cs` when it has no extension, and a C# file gets a class skeleton
/// in the namespace of its folder.
fn new_file(app: &mut DotCraftApp, ctx: &Context, dir: &std::path::Path, name: &mut String, folder: bool, error: &mut Option<String>) -> Outcome {
    let heading = if folder { tr("New Folder") } else { tr("New C# File") };
    let ((ok, cancel), esc) = modal(ctx, "new_file", 380.0, |ui| {
        title(ui, heading);
        let resp = ui.add(egui::TextEdit::singleline(name).hint_text(if folder { tr("Folder name") } else { tr("Class name or file name") }).desired_width(f32::INFINITY));
        resp.request_focus();
        if let Some(e) = error.as_ref() {
            ui.colored_label(rc_kit::pal(ui).error, e);
        }
        let enter = ui.input(|i| i.key_pressed(Key::Enter));
        let (ok, cancel) = buttons(ui, tr("Create"), !name.trim().is_empty());
        (ok || enter && !name.trim().is_empty(), cancel)
    });
    if ok {
        let mut file = name.trim().to_string();
        if !folder && !file.contains('.') {
            file.push_str(".cs");
        }
        let path = dir.join(&file);
        if path.exists() {
            *error = Some(tr("A file or folder with this name already exists").to_string());
            return Outcome::Keep;
        }
        let res = if folder {
            std::fs::create_dir_all(&path)
        } else {
            let content = if file.ends_with(".cs") { csharp_skeleton(dir, &file) } else { String::new() };
            std::fs::write(&path, content)
        };
        match res {
            Ok(()) => {
                app.explorer.invalidate(&path);
                if !folder {
                    app.open_file(&path);
                    if let Some(d) = app.docs.active_doc_mut() {
                        // Caret inside the class body.
                        let text = d.editor.buffer.rope().to_string();
                        if let Some(b) = text.rfind("{\n") {
                            let pos = d.editor.buffer.rope().byte_to_char(b + 2);
                            d.editor.set_selection(rc_text::Selection::point(pos));
                        }
                    }
                }
                return Outcome::Close;
            }
            Err(e) => {
                *error = Some(e.to_string());
                return Outcome::Keep;
            }
        }
    }
    if cancel || esc { Outcome::Close } else { Outcome::Keep }
}

/// `namespace` from the nearest project folder and the relative path, like `dotnet new class`.
fn csharp_skeleton(dir: &std::path::Path, file: &str) -> String {
    let class = file.trim_end_matches(".cs");
    let mut parts: Vec<String> = Vec::new();
    let mut d = Some(dir);
    while let Some(cur) = d {
        let has_project = std::fs::read_dir(cur).map(|rd| rd.flatten().any(|e| e.path().extension().is_some_and(|x| x == "csproj"))).unwrap_or(false);
        let name = cur.file_name().map(|n| n.to_string_lossy().replace([' ', '-'], "_")).unwrap_or_default();
        parts.push(name);
        if has_project {
            break;
        }
        d = cur.parent();
    }
    parts.reverse();
    let ns = parts.join(".");
    format!("namespace {ns};\n\npublic class {class}\n{{\n    \n}}\n")
}

fn rename(app: &mut DotCraftApp, ctx: &Context, path: &mut PathBuf, name: &mut String, error: &mut Option<String>) -> Outcome {
    let ((ok, cancel), esc) = modal(ctx, "rename", 380.0, |ui| {
        title(ui, tr("Rename"));
        let resp = ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
        resp.request_focus();
        if let Some(e) = error.as_ref() {
            ui.colored_label(rc_kit::pal(ui).error, e);
        }
        let enter = ui.input(|i| i.key_pressed(Key::Enter));
        let (ok, cancel) = buttons(ui, tr("Rename"), !name.trim().is_empty());
        (ok || enter && !name.trim().is_empty(), cancel)
    });
    if ok {
        let target = path.with_file_name(name.trim());
        if target == *path {
            return Outcome::Close;
        }
        if target.exists() {
            *error = Some(tr("A file or folder with this name already exists").to_string());
            return Outcome::Keep;
        }
        match std::fs::rename(&*path, &target) {
            Ok(()) => {
                app.explorer.invalidate(path);
                app.explorer.invalidate(&target);
                // Open editors follow the file.
                for d in &mut app.docs.list {
                    if let Ok(rest) = d.path.strip_prefix(&*path) {
                        d.path = target.join(rest);
                        d.editor.path = Some(d.path.clone());
                    }
                }
                return Outcome::Close;
            }
            Err(e) => {
                *error = Some(e.to_string());
                return Outcome::Keep;
            }
        }
    }
    if cancel || esc { Outcome::Close } else { Outcome::Keep }
}

fn confirm_delete(app: &mut DotCraftApp, ctx: &Context, path: &mut PathBuf) -> Outcome {
    let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let ((ok, cancel), esc) = modal(ctx, "delete", 420.0, |ui| {
        title(ui, tr("Delete"));
        ui.horizontal(|ui| {
            icons::image(ui, rc_kit::ico::DIALOG_WARNING, 32.0);
            ui.label(trf("Move \"{name}\" to the Trash?", &[("name", &name)]));
        });
        buttons(ui, tr("Delete"), true)
    });
    if ok {
        // To the system trash: deleting from the IDE can be undone in Finder / Explorer.
        match trash(path) {
            Ok(()) => {
                app.explorer.invalidate(path);
                let mut i = app.docs.list.len();
                while i > 0 {
                    i -= 1;
                    if app.docs.list[i].path.starts_with(&*path) {
                        app.docs.close(i);
                    }
                }
            }
            Err(e) => app.notify(trf("Could not delete {name}: {error}", &[("name", &name), ("error", &e)]), true),
        }
        return Outcome::Close;
    }
    if cancel || esc { Outcome::Close } else { Outcome::Keep }
}

fn trash(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        let script = format!("tell application \"Finder\" to delete POSIX file \"{}\"", path.display().to_string().replace('"', "\\\""));
        let status = std::process::Command::new("osascript").arg("-e").arg(script).output()?;
        if status.status.success() {
            return Ok(());
        }
        Err(std::io::Error::other(String::from_utf8_lossy(&status.stderr).trim().to_string()))
    }
    #[cfg(not(target_os = "macos"))]
    {
        if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) }
    }
}

/// ⌘E: recent files, most recent first (the current one second, so Enter switches back).
fn recent_files(app: &mut DotCraftApp, ctx: &Context, selected: &mut usize, filter: &mut String) -> Outcome {
    let items: Vec<PathBuf> = app
        .docs
        .recent
        .iter()
        .filter(|p| filter.is_empty() || p.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase().contains(&filter.to_lowercase())))
        .cloned()
        .collect();
    let p = rc_kit::theme::palette(ctx.global_style().visuals.dark_mode);
    let mut open = None;
    let (_, esc) = modal(ctx, "recent_files", 480.0, |ui| {
        title(ui, tr("Recent Files"));
        let resp = ui.add(egui::TextEdit::singleline(filter).hint_text(tr("Type to filter")).desired_width(f32::INFINITY));
        resp.request_focus();
        if ui.input(|i| i.key_pressed(Key::ArrowDown)) {
            *selected = (*selected + 1).min(items.len().saturating_sub(1));
        }
        if ui.input(|i| i.key_pressed(Key::ArrowUp)) {
            *selected = selected.saturating_sub(1);
        }
        if ui.input(|i| i.key_pressed(Key::Enter)) {
            open = items.get(*selected).cloned();
        }
        ui.add_space(6.0);
        if items.is_empty() {
            ui.label(egui::RichText::new(tr("No recent files")).color(p.muted));
        }
        for (i, path) in items.iter().enumerate().take(20) {
            let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), egui::Sense::click());
            if i == *selected {
                ui.painter().rect_filled(r, 4.0, p.selection);
            } else if resp.hovered() {
                ui.painter().rect_filled(r, 4.0, p.row_hover);
            }
            icons::paint(ui.painter(), egui::Rect::from_center_size(r.left_center() + vec2(14.0, 0.0), vec2(16.0, 16.0)), icons::for_file(&name), &p, true);
            ui.painter().text(r.left_center() + vec2(30.0, 0.0), Align2::LEFT_CENTER, &name, FontId::proportional(13.0), p.text);
            let dir = path.parent().map(|d| d.display().to_string()).unwrap_or_default();
            ui.painter().text(r.right_center() - vec2(8.0, 0.0), Align2::RIGHT_CENTER, dir, FontId::proportional(11.5), p.muted);
            if resp.clicked() {
                open = Some(path.clone());
            }
        }
    });
    if let Some(path) = open {
        app.open_file(&path);
        return Outcome::Close;
    }
    if esc { Outcome::Close } else { Outcome::Keep }
}

/// The list of context actions, under the caret like IntelliJ's bulb popup.
fn code_actions(app: &mut DotCraftApp, ctx: &Context, list: &mut Vec<rc_lsp::lsp_types::CodeActionOrCommand>, selected: &mut usize) -> Outcome {
    use rc_lsp::lsp_types::CodeActionOrCommand as A;
    let p = rc_kit::theme::palette(ctx.global_style().visuals.dark_mode);
    let pos = app.caret_screen.unwrap_or(egui::pos2(200.0, 200.0));
    let mut chosen = None;
    let mut close = false;
    let n = list.len();
    if ctx.input(|i| i.key_pressed(Key::ArrowDown)) {
        *selected = (*selected + 1) % n.max(1);
    }
    if ctx.input(|i| i.key_pressed(Key::ArrowUp)) {
        *selected = (*selected + n.max(1) - 1) % n.max(1);
    }
    if ctx.input(|i| i.key_pressed(Key::Enter)) {
        chosen = Some(*selected);
    }
    if ctx.input(|i| i.key_pressed(Key::Escape)) {
        close = true;
    }
    let area = egui::Area::new(Id::new("code_actions")).order(egui::Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
        egui::Frame::NONE.fill(p.popup_bg).stroke(egui::Stroke::new(1.0, p.popup_border)).corner_radius(8).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
            ui.set_min_width(320.0);
            for (i, a) in list.iter().enumerate() {
                let (title, quickfix) = match a {
                    A::Command(c) => (c.title.clone(), false),
                    A::CodeAction(c) => (c.title.clone(), c.kind.as_ref().is_some_and(|k| k.as_str().starts_with("quickfix"))),
                };
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(320.0), 26.0), egui::Sense::click());
                if i == *selected {
                    ui.painter().rect_filled(r, 4.0, p.selection);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, 4.0, p.row_hover);
                }
                let icon = if quickfix { rc_kit::ico::BULB_QUICKFIX } else { rc_kit::ico::BULB };
                icons::paint(ui.painter(), egui::Rect::from_center_size(r.left_center() + vec2(14.0, 0.0), vec2(16.0, 16.0)), icon, &p, true);
                ui.painter().text(r.left_center() + vec2(30.0, 0.0), Align2::LEFT_CENTER, &title, FontId::proportional(13.0), p.text);
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &title));
                if resp.clicked() {
                    chosen = Some(i);
                }
            }
        });
    });
    if ctx.input(|i| i.pointer.any_pressed()) && !area.response.hovered() && chosen.is_none() {
        close = true;
    }
    if let Some(i) = chosen {
        let action = list.swap_remove(i);
        let mut out = Vec::new();
        app.insight.run_code_action(&mut app.docs, action, &mut out);
        app.handle_insight(out);
        return Outcome::Close;
    }
    if close { Outcome::Close } else { Outcome::Keep }
}

fn rename_symbol(app: &mut DotCraftApp, ctx: &Context, doc: u64, pos: usize, original: &mut String, name: &mut String, focus: &mut bool) -> Outcome {
    let valid = !name.trim().is_empty() && name.trim().chars().all(|c| c.is_alphanumeric() || c == '_' || c == '@');
    let ((ok, cancel), esc) = modal(ctx, "rename_symbol", 400.0, |ui| {
        title(ui, &trf("Rename '{name}'", &[("name", original)]));
        let resp = ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
        if std::mem::take(focus) {
            resp.request_focus();
            // Select the whole name, like IntelliJ's rename field.
            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), resp.id) {
                let n = name.chars().count();
                state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(n))));
                state.store(ui.ctx(), resp.id);
            }
        }
        ui.label(egui::RichText::new(tr("Renames the symbol and its usages in every project.")).size(12.0).color(rc_kit::pal(ui).muted));
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        let (ok, cancel) = buttons(ui, tr("Rename"), valid);
        (ok || enter && valid, cancel)
    });
    if ok {
        if *name != *original
            && let Some(d) = app.docs.list.iter().find(|d| d.editor.id == doc)
        {
            app.insight.rename(d, pos, name.trim());
        }
        return Outcome::Close;
    }
    if cancel || esc { Outcome::Close } else { Outcome::Keep }
}

fn recent_locations(app: &mut DotCraftApp, ctx: &Context, selected: &mut usize) -> Outcome {
    let items: Vec<(PathBuf, usize, String)> = app
        .nav_back
        .iter()
        .rev()
        .take(30)
        .map(|(path, pos)| {
            let line_text = match app.docs.index_of(path) {
                Some(i) => {
                    let e = &app.docs.list[i].editor;
                    let (l, _) = e.buffer.line_col((*pos).min(e.buffer.len_chars()));
                    (l, e.buffer.line_text(l))
                }
                None => (0, String::new()),
            };
            (path.clone(), line_text.0, line_text.1)
        })
        .collect();
    let p = rc_kit::theme::palette(ctx.global_style().visuals.dark_mode);
    let mut open = None;
    let (_, esc) = modal(ctx, "recent_locations", 620.0, |ui| {
        title(ui, tr("Recent Locations"));
        if ui.input(|i| i.key_pressed(Key::ArrowDown)) {
            *selected = (*selected + 1).min(items.len().saturating_sub(1));
        }
        if ui.input(|i| i.key_pressed(Key::ArrowUp)) {
            *selected = selected.saturating_sub(1);
        }
        if ui.input(|i| i.key_pressed(Key::Enter)) {
            open = items.get(*selected).cloned();
        }
        for (i, (path, line, text)) in items.iter().enumerate() {
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), egui::Sense::click());
            if i == *selected {
                ui.painter().rect_filled(r, 6.0, p.selection);
            } else if resp.hovered() {
                ui.painter().rect_filled(r, 6.0, p.row_hover);
            }
            let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            icons::paint(ui.painter(), egui::Rect::from_center_size(r.left_top() + vec2(14.0, 13.0), vec2(16.0, 16.0)), icons::for_file(&name), &p, true);
            ui.painter().text(r.left_top() + vec2(30.0, 13.0), Align2::LEFT_CENTER, format!("{name}:{}", line + 1), theme::bold(12.5), p.text);
            ui.painter().text(r.left_top() + vec2(30.0, 31.0), Align2::LEFT_CENTER, text.trim(), FontId::monospace(12.0), p.text_muted);
            if resp.clicked() {
                open = Some((path.clone(), *line, text.clone()));
            }
        }
    });
    if let Some((path, line, _)) = open {
        app.open_at(&path, line, 0);
        return Outcome::Close;
    }
    if esc { Outcome::Close } else { Outcome::Keep }
}

fn rollback(app: &mut DotCraftApp, ctx: &Context, paths: &mut Vec<String>) -> Outcome {
    let ((ok, cancel), esc) = modal(ctx, "rollback", 440.0, |ui| {
        title(ui, tr("Rollback Changes"));
        ui.label(trf("Discard the changes of {n} files? This cannot be undone.", &[("n", &paths.len())]));
        for p in paths.iter().take(8) {
            ui.label(egui::RichText::new(p).size(12.0).color(rc_kit::pal(ui).muted));
        }
        buttons(ui, tr("Rollback"), true)
    });
    if ok {
        app.vcs.rollback(std::mem::take(paths));
        return Outcome::Close;
    }
    if cancel || esc { Outcome::Close } else { Outcome::Keep }
}

fn new_branch(app: &mut DotCraftApp, ctx: &Context, name: &mut String) -> Outcome {
    let valid = !name.trim().is_empty() && !name.contains(' ') && !name.contains("..");
    let ((ok, cancel), esc) = modal(ctx, "new_branch", 380.0, |ui| {
        title(ui, tr("New Branch"));
        let resp = ui.add(egui::TextEdit::singleline(name).hint_text(tr("Branch name")).desired_width(f32::INFINITY));
        resp.request_focus();
        ui.label(egui::RichText::new(tr("The new branch is checked out.")).size(12.0).color(rc_kit::pal(ui).muted));
        let enter = ui.input(|i| i.key_pressed(Key::Enter));
        let (ok, cancel) = buttons(ui, tr("Create"), valid);
        (ok || enter && valid, cancel)
    });
    if ok {
        app.vcs.new_branch(name.trim());
        return Outcome::Close;
    }
    if cancel || esc { Outcome::Close } else { Outcome::Keep }
}

/// ⌘F12: the members of the active file, filtered by typing.
fn file_structure(app: &mut DotCraftApp, ctx: &Context, filter: &mut String, selected: &mut usize) -> Outcome {
    let Some(doc) = app.docs.active_doc() else { return Outcome::Close };
    let id = doc.editor.id;
    let Some(tree) = app.insight.structure.get(&id).cloned() else {
        app.notify(tr("The file structure is not available yet"), false);
        return Outcome::Close;
    };
    let mut flat: Vec<(usize, crate::insight::SymbolNode)> = Vec::new();
    fn walk(nodes: &[crate::insight::SymbolNode], depth: usize, out: &mut Vec<(usize, crate::insight::SymbolNode)>) {
        for n in nodes {
            out.push((depth, n.clone()));
            walk(&n.children, depth + 1, out);
        }
    }
    walk(&tree, 0, &mut flat);
    let items: Vec<(usize, crate::insight::SymbolNode)> = flat
        .into_iter()
        .filter(|(_, n)| filter.is_empty() || rc_editor::popup::fuzzy_score(&n.name, filter).is_some())
        .collect();
    let p = rc_kit::theme::palette(ctx.global_style().visuals.dark_mode);
    let mut open = None;
    let (_, esc) = modal(ctx, "file_structure", 460.0, |ui| {
        title(ui, &doc.name());
        let resp = ui.add(egui::TextEdit::singleline(filter).hint_text(tr("Type to filter")).desired_width(f32::INFINITY));
        resp.request_focus();
        if ui.input(|i| i.key_pressed(Key::ArrowDown)) {
            *selected = (*selected + 1).min(items.len().saturating_sub(1));
        }
        if ui.input(|i| i.key_pressed(Key::ArrowUp)) {
            *selected = selected.saturating_sub(1);
        }
        if ui.input(|i| i.key_pressed(Key::Enter)) {
            open = items.get(*selected).map(|(_, n)| (n.line, n.col));
        }
        ui.add_space(6.0);
        egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
            for (i, (depth, n)) in items.iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), egui::Sense::click());
                if i == *selected {
                    ui.painter().rect_filled(r, 4.0, p.selection);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, 4.0, p.row_hover);
                }
                let x = r.left() + 8.0 + *depth as f32 * 16.0;
                icons::paint(ui.painter(), egui::Rect::from_center_size(egui::pos2(x + 8.0, r.center().y), vec2(16.0, 16.0)), crate::convert::symbol_icon(n.kind), &p, true);
                let g = ui.painter().layout_no_wrap(n.name.clone(), FontId::proportional(13.0), p.text);
                ui.painter().galley(egui::pos2(x + 22.0, r.center().y - g.size().y / 2.0), g.clone(), p.text);
                if !n.detail.is_empty() {
                    ui.painter().text(egui::pos2(x + 30.0 + g.size().x, r.center().y), Align2::LEFT_CENTER, &n.detail, FontId::proportional(12.0), p.muted);
                }
                if resp.clicked() {
                    open = Some((n.line, n.col));
                }
            }
        });
    });
    if let Some((line, col)) = open {
        app.push_nav();
        if let Some(d) = app.docs.active_doc_mut() {
            d.editor.goto(line, col);
        }
        return Outcome::Close;
    }
    if esc { Outcome::Close } else { Outcome::Keep }
}

#[cfg(test)]
mod tests {
    #[test]
    fn skeleton_namespace_follows_the_project_folder() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("Shop.Core");
        std::fs::create_dir_all(proj.join("Models")).unwrap();
        std::fs::write(proj.join("Shop.Core.csproj"), "<Project/>").unwrap();
        let s = super::csharp_skeleton(&proj.join("Models"), "Order.cs");
        assert!(s.starts_with("namespace Shop.Core.Models;"), "{s}");
        assert!(s.contains("public class Order"));
    }
}
