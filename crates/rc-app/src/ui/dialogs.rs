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
    VersionControl,
}

pub enum Dialog {
    Settings { draft: Box<Settings>, page: SettingsPage },
    About,
    SdkSelection { state: Box<crate::ui::sdk_selection::SdkSelection>, parent_draft: Box<Settings>, parent_page: SettingsPage },
    AttachProcess { pid: String, focus: bool },
    GotoLine { text: String, focus: bool },
    /// ⌘J / ⌥⌘T: live templates at the caret, or to surround the selection.
    Templates { surround: bool, filter: String, selected: usize },
    /// ⌥F3: pick the bookmark's digit or letter.
    BookmarkMnemonic { path: PathBuf, line: usize },
    NewFile { dir: PathBuf, name: String, kind: crate::item_templates::ItemKind, error: Option<String> },
    Rename { path: PathBuf, name: String, error: Option<String> },
    ConfirmDelete { path: PathBuf },
    RecentFiles { selected: usize, filter: String },
    /// ⌥⏎ context actions for the active editor.
    CodeActions { list: Vec<rc_lsp::lsp_types::CodeActionOrCommand>, selected: usize },
    /// Shift+F6 on a symbol.
    RenameSymbol { doc: u64, pos: usize, original: String, name: String, focus: bool },
    /// Confirm rolling back files to HEAD.
    Rollback { paths: Vec<String> },
    /// A new branch at `start` (default HEAD), checked out when `checkout`.
    NewBranch { name: String, start: Option<String>, checkout: bool },
    RenameBranch { old: String, name: String },
    /// Delete a local branch; `force` is the second confirmation for an unmerged one.
    DeleteBranch { name: String, force: bool },
    /// Delete a branch on its remote (`origin/feature`).
    DeleteRemoteBranch { name: String },
    /// The commits that differ between a branch and the current one.
    CompareBranch { branch: String },
    /// ⇧⌘E: the places the caret jumped from.
    RecentLocations { selected: usize },
    /// ⌘F12 file structure popup.
    FileStructure { filter: String, selected: usize },
    /// Right-click on a breakpoint: condition and enabled state (0-based line).
    BreakpointProperties { path: PathBuf, line: usize, condition: String, enabled: bool, focus: bool },
    /// Edit an application run configuration: launch profile, arguments, environment, folder.
    RunOptions { name: String, project: PathBuf, profiles: Vec<String>, options: rc_core::session::RunOptions, web: bool },
    Publish(Box<crate::ui::publish::PublishDialog>),
    /// A compound run configuration: its name and which applications it starts.
    Compound { old: Option<String>, name: String, members: Vec<(String, bool)> },
    /// ⌥F8: evaluate an expression in the selected frame.
    Evaluate { expression: String, focus: bool },
    /// "Interactively Rebase from Here…" on a commit of the Git log.
    InteractiveRebase(Box<crate::ui::rebase::RebaseDialog>),
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
        Dialog::About => about(app, ctx),
        Dialog::SdkSelection { state, parent_draft, parent_page } => {
            if state.show(ctx) {
                app.dialog = Some(Dialog::Settings { draft: parent_draft.clone(), page: *parent_page });
            }
            Outcome::Keep
        }
        Dialog::AttachProcess { pid, focus } => attach_process(app, ctx, pid, focus),
        Dialog::GotoLine { text, focus } => goto_line(app, ctx, text, focus),
        Dialog::BookmarkMnemonic { path, line } => bookmark_mnemonic(app, ctx, path, *line),
        Dialog::Templates { surround, filter, selected } => templates(app, ctx, *surround, filter, selected),
        Dialog::NewFile { dir, name, kind, error } => new_file(app, ctx, dir, name, *kind, error),
        Dialog::Rename { path, name, error } => rename(app, ctx, path, name, error),
        Dialog::ConfirmDelete { path } => confirm_delete(app, ctx, path),
        Dialog::RecentFiles { selected, filter } => recent_files(app, ctx, selected, filter),
        Dialog::CodeActions { list, selected } => code_actions(app, ctx, list, selected),
        Dialog::RenameSymbol { doc, pos, original, name, focus } => rename_symbol(app, ctx, *doc, *pos, original, name, focus),
        Dialog::FileStructure { filter, selected } => file_structure(app, ctx, filter, selected),
        Dialog::Rollback { paths } => rollback(app, ctx, paths),
        Dialog::NewBranch { name, start, checkout } => new_branch(app, ctx, name, start.as_deref(), checkout),
        Dialog::RenameBranch { old, name } => rename_branch(app, ctx, old, name),
        Dialog::DeleteBranch { name, force } => delete_branch(app, ctx, name, *force),
        Dialog::DeleteRemoteBranch { name } => delete_remote_branch(app, ctx, name),
        Dialog::CompareBranch { branch } => compare_branch(app, ctx, branch),
        Dialog::RecentLocations { selected } => recent_locations(app, ctx, selected),
        Dialog::BreakpointProperties { path, line, condition, enabled, focus } => breakpoint_properties(app, ctx, path, *line, condition, enabled, focus),
        Dialog::Evaluate { expression, focus } => evaluate(app, ctx, expression, focus),
        Dialog::Compound { old, name, members } => compound(app, ctx, old.as_deref(), name, members),
        Dialog::RunOptions { name, project, profiles, options, web } => run_options(app, ctx, name, project, profiles, options, *web),
        Dialog::Publish(state) => match state.show(ctx) {
            crate::ui::publish::PublishOutcome::Keep => Outcome::Keep,
            crate::ui::publish::PublishOutcome::Close => Outcome::Close,
            crate::ui::publish::PublishOutcome::Start(request) => {
                let outputs = app.runner.build_request(request);
                crate::ui::run_windows::handle_outputs(app, outputs);
                Outcome::Close
            }
        },
        Dialog::InteractiveRebase(state) => if crate::ui::rebase::dialog(app, ctx, state) { Outcome::Close } else { Outcome::Keep },
    };
    if matches!(outcome, Outcome::Keep) && app.dialog.is_none() {
        app.dialog = Some(dialog);
    } else if matches!(outcome, Outcome::Close) {
        app.focus_editor = true;
    }
}

pub(crate) fn modal<R>(ctx: &Context, id: &str, width: f32, add: impl FnOnce(&mut Ui) -> R) -> (R, bool) {
    let resp = egui::Modal::new(Id::new(id)).show(ctx, |ui| {
        ui.set_width(width);
        add(ui)
    });
    let escape = resp.should_close();
    (resp.inner, escape)
}

pub(crate) fn title(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text).font(theme::bold(15.0)));
    ui.add_space(10.0);
}

pub(crate) fn buttons(ui: &mut Ui, ok: &str, enabled: bool) -> (bool, bool) {
    ui.add_space(14.0);
    let mut res = (false, false);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        res.0 = ui.add_enabled_ui(enabled, |ui| theme::primary_button(ui, ok)).inner.clicked();
        res.1 = theme::secondary_button(ui, tr("Cancel")).clicked();
    });
    res
}

/// Responsive geometry adapted from CrabCraft's native settings dialog.
/// The viewport, rather than page contents, determines the modal's bounds.
struct SettingsLayout { width: f32, nav: f32, page: f32, height: f32, compact: bool }

impl SettingsLayout {
    fn for_viewport(viewport: egui::Vec2) -> Self {
        let width = (viewport.x - 32.0).clamp(1.0, 720.0);
        let compact = width < 620.0;
        let nav = if compact { 0.0 } else { 170.0 };
        let page = if compact { width } else { width - nav - 24.0 };
        let chrome = if compact { 156.0 } else { 120.0 };
        let height = (viewport.y * 0.7).min(420.0).min((viewport.y - chrome).max(1.0));
        Self { width, nav, page, height, compact }
    }
}

fn settings_pages() -> [(SettingsPage, &'static str); 5] {
    [
        (SettingsPage::Appearance, tr("Appearance")),
        (SettingsPage::Editor, tr("Editor")),
        (SettingsPage::Keymap, tr("Keymap")),
        (SettingsPage::Tools, tr("Tools")),
        (SettingsPage::VersionControl, tr("Version Control")),
    ]
}

fn settings(app: &mut DotCraftApp, ctx: &Context, draft: &mut Settings, page: &mut SettingsPage) -> Outcome {
    let layout = SettingsLayout::for_viewport(ctx.content_rect().size());
    let ((ok, cancel), esc) = modal(ctx, "settings", layout.width, |ui| {
        title(ui, tr("Settings"));
        if layout.compact {
            // Narrow window: the pages in a drop-down instead of the list on the left.
            let selected = settings_pages().into_iter().find(|(pg, _)| pg == page).map_or("", |(_, label)| label);
            let labels: Vec<String> = settings_pages().iter().map(|(_, label)| label.to_string()).collect();
            if let Some(i) = crate::ui::kit::dropdown(ui, "settings_category", selected, &labels, true) {
                *page = settings_pages()[i].0;
            }
            ui.add_space(8.0);
        }
        ui.horizontal_top(|ui| {
            if !layout.compact {
                egui::ScrollArea::vertical().id_salt("settings_navigation").max_height(layout.height).min_scrolled_height(layout.height).auto_shrink([false, false]).max_width(layout.nav).show(ui, |ui| ui.vertical(|ui| {
                    ui.set_width(layout.nav);
                    let p = rc_kit::pal(ui);
                    for (pg, label) in settings_pages() {
                        let (r, resp) = ui.allocate_exact_size(vec2(layout.nav, 28.0), egui::Sense::click());
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
                }));
                ui.add_space(16.0);
            }
            // Scroll both axes when a grid needs more width, while keeping the footer fixed.
            // Tools stack their fields and use the available width instead of widening the modal.
            let content_width = (layout.page - ui.spacing().scroll.allocated_width() - ui.spacing().scroll.bar_width - 12.0).max(40.0);
            egui::ScrollArea::new([!matches!(*page, SettingsPage::Tools | SettingsPage::VersionControl), true]).id_salt("settings_fixed_page").max_height(layout.height).min_scrolled_height(layout.height).max_width(layout.page).auto_shrink([false, false]).show(ui, |ui| ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(content_width);
                ui.set_height(layout.height);
                if *page == SettingsPage::Tools {
                    settings_tools(ui, draft, app);
                    return;
                }
                if *page == SettingsPage::VersionControl {
                    crate::ui::work_item_links::settings_page(ui, draft, app);
                    return;
                }
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
                        ui.label("");
                        ui.checkbox(&mut draft.reopen_last, tr("Reopen the last project on startup"));
                        ui.end_row();
                        ui.label("");
                        ui.checkbox(&mut draft.check_updates, tr("Check for updates at startup"));
                        ui.end_row();
                        ui.label("");
                        ui.checkbox(&mut draft.tabs_multirow, tr("Show tabs in multiple rows"));
                        ui.end_row();
                        ui.label("");
                        ui.checkbox(&mut draft.tab_colors, tr("Color tabs by kind of file"));
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
                    SettingsPage::Tools | SettingsPage::VersionControl => {}

                });
            }));
        });
        buttons(ui, tr("OK"), true)
    });
    if ok {
        app.apply_settings(ctx, draft.clone());
        return Outcome::Close;
    }
    if cancel || esc { Outcome::Close } else { Outcome::Keep }
}

fn settings_tools(ui: &mut Ui, draft: &mut Settings, app: &mut DotCraftApp) {
    ui.label(tr(".NET SDK"));
    if ui.add_enabled_ui(app.workspace.is_some(), |ui| theme::secondary_button(ui, tr("Manage project SDK…"))).inner.clicked()
        && let Some(workspace) = &app.workspace
    {
        app.dialog = Some(Dialog::SdkSelection {
            state: Box::new(crate::ui::sdk_selection::SdkSelection::new(
                workspace.ws.root.clone(), draft.dotnet_path.clone().unwrap_or_else(|| PathBuf::from("dotnet")), app.waker.clone(),
            )),
            parent_draft: Box::new(draft.clone()),
            parent_page: SettingsPage::Tools,
        });
    }
    ui.add_space(10.0);
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
    for (label, value) in [
        (tr(".NET CLI"), &mut draft.dotnet_path),
        (tr("Roslyn language server"), &mut draft.roslyn_path),
        (tr("netcoredbg debugger"), &mut draft.netcoredbg_path),
    ] {
        let label_response = ui.label(label);
        let browse_width = (ui.fonts_mut(|fonts| fonts.layout_no_wrap(tr("Browse…").to_string(), FontId::proportional(13.0), rc_kit::pal(ui).text).size().x) + 24.0).max(72.0);
        let field_width = (ui.available_width() - browse_width - ui.spacing().item_spacing.x).max(24.0);
        ui.horizontal(|ui| {
            let mut text = value.as_ref().map(|path| path.display().to_string()).unwrap_or_default();
            if ui.add(egui::TextEdit::singleline(&mut text).hint_text(tr("Detect automatically")).desired_width(field_width)).labelled_by(label_response.id).changed() {
                *value = (!text.trim().is_empty()).then(|| PathBuf::from(text.trim()));
            }
            if theme::secondary_button(ui, tr("Browse…")).clicked()
                && let Some(path) = rfd::FileDialog::new().pick_file()
            {
                *value = Some(path);
            }
        });
        ui.add_space(10.0);
    }
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

fn about(app: &mut DotCraftApp, ctx: &Context) -> Outcome {
    let mut check = false;
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
            ui.horizontal(|ui| {
                // Centred pair of buttons.
                let w = 72.0 * 2.0 + 8.0 + 90.0;
                ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
                ui.spacing_mut().item_spacing.x = 8.0;
                check = theme::secondary_button(ui, tr("Check for Updates…")).clicked();
                theme::primary_button(ui, tr("Close")).clicked()
            })
            .inner
        })
        .inner
    });
    if check {
        app.check_for_updates(ctx);
        return Outcome::Close;
    }
    if close || esc { Outcome::Close } else { Outcome::Keep }
}

/// Rider's mnemonic picker: a grid of 1-9, 0 and A-Z; typing the key also picks it.
fn bookmark_mnemonic(app: &mut DotCraftApp, ctx: &Context, path: &std::path::Path, line: usize) -> Outcome {
    let current = app.docs.all_bookmarks().into_iter().find(|b| b.path == path && b.line == line).and_then(|b| b.mnemonic);
    let used: Vec<char> = app.docs.all_bookmarks().into_iter().filter_map(|b| b.mnemonic).collect();
    let keys: Vec<char> = "1234567890ABCDEFGHIJKLMNOPQRSTUVWXYZ".chars().collect();
    let typed = ctx.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Text(t) => t.chars().next().map(|c| c.to_ascii_uppercase()).filter(|c| keys.contains(c)),
            _ => None,
        })
    });
    let ((picked, cancel), esc) = modal(ctx, "bookmark_mnemonic", 400.0, |ui| {
        title(ui, tr("Bookmark Mnemonic"));
        let mut picked = typed;
        let p = rc_kit::pal(ui);
        for row in keys.chunks(10) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for &c in row {
                    let selected = current == Some(c);
                    let (r, resp) = ui.allocate_exact_size(vec2(32.0, 28.0), egui::Sense::click());
                    let fill = if selected { p.selection } else if resp.hovered() { p.hover } else { p.control_bg };
                    ui.painter().rect(r, 4.0, fill, egui::Stroke::new(1.0, p.control_border), egui::StrokeKind::Inside);
                    let color = if used.contains(&c) && !selected { p.muted } else { p.text };
                    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, c.to_string(), egui::FontId::monospace(13.0), color);
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, c.to_string()));
                    if resp.clicked() {
                        picked = Some(c);
                    }
                }
            });
            ui.add_space(4.0);
        }
        ui.label(egui::RichText::new(tr("Type a digit or letter. Taken ones move here.")).size(12.0).color(p.muted));
        let (_, cancel) = buttons(ui, tr("OK"), false);
        (picked, cancel)
    });
    if let Some(c) = picked {
        // The same key again removes the mnemonic (the bookmark stays).
        let mark = if current == Some(c) { None } else { Some(c) };
        app.docs.set_bookmark(path, line, Some(mark));
        app.session_dirty = true;
        return Outcome::Close;
    }
    if cancel || esc { Outcome::Close } else { Outcome::Keep }
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

fn run_options(app: &mut DotCraftApp, ctx: &Context, name: &str, project: &std::path::Path, profiles: &[String], o: &mut rc_core::session::RunOptions, web: bool) -> Outcome {
    let settings_file = project.parent().map(|d| d.join("Properties/launchSettings.json")).filter(|p| p.exists());
    let mut open_settings = false;
    let ((ok, cancel), esc) = modal(ctx, "run_options", 560.0, |ui| {
        title(ui, &trf("Run Configuration: {name}", &[("name", &name)]));
        let p = rc_kit::pal(ui);
        egui::Grid::new("run_options_grid").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
            ui.label(tr("Launch profile"));
            let label = |v: &Option<String>| match v.as_deref() {
                None => tr("Default").to_string(),
                Some("") => tr("None (ignore launchSettings.json)").to_string(),
                Some(n) => n.to_string(),
            };
            egui::ComboBox::from_id_salt("run_profile").selected_text(label(&o.profile)).width(300.0).show_ui(ui, |ui| {
                ui.selectable_value(&mut o.profile, None, tr("Default"));
                for pr in profiles {
                    ui.selectable_value(&mut o.profile, Some(pr.clone()), pr);
                }
                ui.selectable_value(&mut o.profile, Some(String::new()), tr("None (ignore launchSettings.json)"));
            });
            ui.end_row();
            ui.label(tr("Program arguments"));
            ui.add(egui::TextEdit::singleline(&mut o.args).hint_text(tr("The profile's arguments")).font(egui::TextStyle::Monospace).desired_width(380.0));
            ui.end_row();
            ui.label(tr("Environment variables"));
            ui.add(egui::TextEdit::multiline(&mut o.env).hint_text("ASPNETCORE_ENVIRONMENT=Development\nASPNETCORE_URLS=http://localhost:5000").font(egui::TextStyle::Monospace).desired_rows(4).desired_width(380.0));
            ui.end_row();
            ui.label(tr("Working directory"));
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut o.working_dir).hint_text(tr("The project folder")).desired_width(346.0));
                if theme::icon_button(ui, rc_kit::ico::OPEN, true, tr("Choose a folder")).clicked()
                    && let Some(d) = rfd::FileDialog::new().set_directory(project.parent().unwrap_or(project)).pick_folder()
                {
                    o.working_dir = d.display().to_string();
                }
            });
            ui.end_row();
            ui.label("");
            let mut watch = o.hot_reload.unwrap_or(web);
            if ui.checkbox(&mut watch, tr("Hot Reload: apply saved changes and refresh the browser (dotnet watch)")).changed() {
                o.hot_reload = if watch == web { None } else { Some(watch) };
            }
            ui.end_row();
            ui.label("");
            ui.add_enabled(!watch, egui::Checkbox::new(&mut o.build_first, tr("Build before running")));
            ui.end_row();
        });
        ui.add_space(6.0);
        ui.label(egui::RichText::new(tr("Variables here are added to the profile's; one NAME=value per line.")).size(12.0).color(p.muted));
        if settings_file.is_some() {
            ui.add_space(4.0);
            if ui.link(tr("Open launchSettings.json")).clicked() {
                open_settings = true;
            }
        }
        buttons(ui, tr("OK"), true)
    });
    if open_settings && let Some(f) = settings_file {
        app.open_file(&f);
        return Outcome::Close;
    }
    if ok {
        if *o == rc_core::session::RunOptions::default() {
            app.runner.options.remove(name);
        } else {
            app.runner.options.insert(name.to_string(), o.clone());
        }
        app.session_dirty = true;
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
fn new_file(app: &mut DotCraftApp, ctx: &Context, dir: &std::path::Path, name: &mut String, kind: crate::item_templates::ItemKind, error: &mut Option<String>) -> Outcome {
    let heading = trf("New {item}", &[("item", &tr(kind.title()))]);
    let ((ok, cancel), esc) = modal(ctx, "new_file", 380.0, |ui| {
        title(ui, &heading);
        let resp = ui.add(egui::TextEdit::singleline(name).hint_text(tr("Name")).desired_width(f32::INFINITY));
        resp.request_focus();
        if !kind.is_folder() && !name.trim().is_empty() {
            let files = kind.files(dir, name);
            let detail = if kind.is_plain() {
                // Where it lands, from the solution folder.
                let root = app.workspace.as_ref().map(|w| w.ws.root.clone()).unwrap_or_default();
                files[0].0.strip_prefix(&root).unwrap_or(&files[0].0).display().to_string()
            } else {
                let shown: Vec<String> = files.iter().filter_map(|(p, _)| p.file_name().map(|n| n.to_string_lossy().into_owned())).collect();
                format!("{} · {}", shown.join(", "), crate::item_templates::namespace_for(dir))
            };
            ui.add(egui::Label::new(egui::RichText::new(detail).size(12.0).color(rc_kit::pal(ui).muted)).truncate());
        }
        if let Some(e) = error.as_ref() {
            ui.colored_label(rc_kit::pal(ui).error, e);
        }
        let enter = ui.input(|i| i.key_pressed(Key::Enter));
        let (ok, cancel) = buttons(ui, tr("Create"), !name.trim().is_empty());
        (ok || enter && !name.trim().is_empty(), cancel)
    });
    if ok {
        let files = kind.files(dir, name);
        if files.iter().any(|(p, _)| p.exists()) {
            *error = Some(tr("A file or folder with this name already exists").to_string());
            return Outcome::Keep;
        }
        let res: std::io::Result<()> = files.iter().try_for_each(|(p, content)| {
            if kind.is_folder() {
                std::fs::create_dir_all(p)
            } else {
                // "Folder/name.ext" creates the folders too.
                p.parent().map_or(Ok(()), std::fs::create_dir_all)?;
                std::fs::write(p, content.replace("$0", ""))
            }
        });
        match res {
            Ok(()) => {
                let (first, content) = &files[0];
                app.explorer.invalidate(first);
                if !kind.is_folder() {
                    app.open_file(first);
                    // The caret where the template put `$0`.
                    if let (Some(d), Some(at)) = (app.docs.active_doc_mut(), content.find("$0")) {
                        let pos = content[..at].chars().count();
                        d.editor.set_selection(rc_text::Selection::point(pos));
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
                app.docs.follow_rename(path, &target);
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
    *selected = (*selected).min(items.len().saturating_sub(1));
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
            let accessible_path = path.display().to_string();
            resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, i == *selected, &accessible_path));
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
/// The live template list at the caret: typing filters, Enter or a click inserts.
fn templates(app: &mut DotCraftApp, ctx: &Context, surround: bool, filter: &mut String, selected: &mut usize) -> Outcome {
    let p = rc_kit::theme::palette(ctx.global_style().visuals.dark_mode);
    let pos = app.caret_screen.unwrap_or(egui::pos2(200.0, 200.0));
    let all = if surround { rc_editor::templates::SURROUND } else { rc_editor::templates::CSHARP };
    for ev in ctx.input(|i| i.events.clone()) {
        match ev {
            egui::Event::Text(t) => {
                filter.push_str(&t);
                *selected = 0;
            }
            egui::Event::Key { key: Key::Backspace, pressed: true, .. } => {
                filter.pop();
                *selected = 0;
            }
            _ => {}
        }
    }
    let f = filter.to_lowercase();
    let list: Vec<&rc_editor::templates::Template> = all.iter().filter(|t| t.key.contains(&f) || t.description.to_lowercase().contains(&f)).collect();
    let n = list.len();
    let mut chosen = None;
    let mut close = false;
    if ctx.input(|i| i.key_pressed(Key::ArrowDown)) {
        *selected = (*selected + 1) % n.max(1);
    }
    if ctx.input(|i| i.key_pressed(Key::ArrowUp)) {
        *selected = (*selected + n.max(1) - 1) % n.max(1);
    }
    if ctx.input(|i| i.key_pressed(Key::Enter) || i.key_pressed(Key::Tab)) && n > 0 {
        chosen = Some(*selected);
    }
    if ctx.input(|i| i.key_pressed(Key::Escape)) {
        close = true;
    }
    let title = if surround { tr("Surround With") } else { tr("Insert Live Template") };
    let area = egui::Area::new(Id::new("templates")).order(egui::Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
        egui::Frame::NONE.fill(p.popup_bg).stroke(egui::Stroke::new(1.0, p.popup_border)).corner_radius(8).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
            ui.set_min_width(340.0);
            let head = if filter.is_empty() { title.to_string() } else { format!("{title}: {filter}") };
            ui.label(egui::RichText::new(head).size(12.0).color(p.muted));
            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                for (i, t) in list.iter().enumerate() {
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(340.0), 24.0), egui::Sense::click());
                    if i == *selected {
                        ui.painter().rect_filled(r, 4.0, p.selection);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(r, 4.0, p.row_hover);
                    }
                    let k = ui.painter().text(r.left_center() + vec2(10.0, 0.0), Align2::LEFT_CENTER, t.key, FontId::monospace(12.5), p.text);
                    ui.painter().text(egui::pos2(k.right().max(r.left() + 90.0) + 8.0, r.center().y), Align2::LEFT_CENTER, t.description, FontId::proportional(12.5), p.muted);
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, t.key));
                    if resp.clicked() {
                        chosen = Some(i);
                    }
                }
            });
        });
    });
    // A press outside the list closes it (the rows themselves take the clicks).
    let outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !area.response.rect.contains(p)));
    if outside && chosen.is_none() {
        close = true;
    }
    if let Some(i) = chosen
        && let Some(t) = list.get(i).copied()
        && let Some(doc) = app.docs.active_doc_mut()
    {
        if surround {
            rc_editor::templates::surround(&mut doc.editor, t);
        } else {
            rc_editor::templates::insert(&mut doc.editor, t);
        }
        app.focus_editor = true;
        return Outcome::Close;
    }
    if close { Outcome::Close } else { Outcome::Keep }
}

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
        // The menu holds the keyboard focus while open, so Enter, the arrows and Esc reach it
        // and not the editor below (which would insert a newline or move the caret).
        let menu = ui.interact(ui.max_rect(), Id::new("code_actions_menu"), egui::Sense::focusable_noninteractive());
        if !menu.has_focus() {
            menu.request_focus();
        }
        ui.memory_mut(|m| m.set_focus_lock_filter(menu.id, egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true }));
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
        app.focus_editor = true;
        let action = list.swap_remove(i);
        // Package fixes of project files are applied here, not by the language server.
        if let Some(fix) = crate::package_insight::fix_of(&action) {
            app.apply_package_fix(fix);
            return Outcome::Close;
        }
        let mut out = Vec::new();
        app.insight.run_code_action(&mut app.docs, action, &mut out);
        app.handle_insight(out);
        return Outcome::Close;
    }
    if close {
        app.focus_editor = true;
        Outcome::Close
    } else {
        Outcome::Keep
    }
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
    *selected = (*selected).min(items.len().saturating_sub(1));
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
            let accessible_location = format!("{}:{}", path.display(), line + 1);
            resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, i == *selected, &accessible_location));
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

/// A branch name git may accept (git itself validates it when the branch is created).
fn plausible_branch_name(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty() && !name.starts_with('-') && !name.contains(char::is_whitespace) && !name.contains("..")
}

fn new_branch(app: &mut DotCraftApp, ctx: &Context, name: &mut String, start: Option<&str>, checkout: &mut bool) -> Outcome {
    let valid = plausible_branch_name(name);
    let ((ok, cancel), esc) = modal(ctx, "new_branch", 400.0, |ui| {
        match start {
            Some(start) => title(ui, &trf("New Branch from '{branch}'", &[("branch", &start)])),
            None => title(ui, tr("New Branch")),
        }
        let resp = ui.add(egui::TextEdit::singleline(name).hint_text(tr("Branch name")).desired_width(f32::INFINITY));
        resp.request_focus();
        ui.add_space(6.0);
        ui.checkbox(checkout, tr("Checkout branch"));
        let enter = ui.input(|i| i.key_pressed(Key::Enter));
        let (ok, cancel) = buttons(ui, tr("Create"), valid);
        (ok || enter && valid, cancel)
    });
    if ok {
        if *checkout && !crate::ui::git::save_before_branch_change(app) {
            return Outcome::Close;
        }
        app.vcs.new_branch_from(name.trim(), start, *checkout);
        return Outcome::Close;
    }
    if cancel || esc { Outcome::Close } else { Outcome::Keep }
}

fn rename_branch(app: &mut DotCraftApp, ctx: &Context, old: &str, name: &mut String) -> Outcome {
    let valid = plausible_branch_name(name) && name.trim() != old;
    let ((ok, cancel), esc) = modal(ctx, "rename_branch", 400.0, |ui| {
        title(ui, &trf("Rename '{branch}'", &[("branch", &old)]));
        let resp = ui.add(egui::TextEdit::singleline(name).hint_text(tr("Branch name")).desired_width(f32::INFINITY));
        resp.request_focus();
        let enter = ui.input(|i| i.key_pressed(Key::Enter));
        let (ok, cancel) = buttons(ui, tr("Rename"), valid);
        (ok || enter && valid, cancel)
    });
    if ok {
        app.vcs.rename_branch(old, name.trim());
        return Outcome::Close;
    }
    if cancel || esc { Outcome::Close } else { Outcome::Keep }
}

/// A confirmation with a warning icon, a question and a grey detail.
fn confirm(ctx: &Context, id: &str, question: &str, detail: &str, ok: &str) -> ((bool, bool), bool) {
    modal(ctx, id, 440.0, |ui| {
        ui.horizontal(|ui| {
            icons::image(ui, rc_kit::ico::DIALOG_WARNING, 20.0);
            ui.add_space(4.0);
            ui.label(egui::RichText::new(question).font(theme::bold(14.0)));
        });
        ui.add_space(6.0);
        ui.label(egui::RichText::new(detail).size(12.5).color(rc_kit::pal(ui).muted));
        buttons(ui, ok, true)
    })
}

fn delete_branch(app: &mut DotCraftApp, ctx: &Context, name: &str, force: bool) -> Outcome {
    let ((ok, cancel), esc) = if force {
        confirm(ctx, "force_delete_branch", &trf("'{branch}' is not fully merged", &[("branch", &name)]), tr("Its commits are not in the current branch or its upstream and will be lost. Delete it anyway?"), tr("Force Delete"))
    } else {
        confirm(ctx, "delete_branch", &trf("Delete branch '{branch}'?", &[("branch", &name)]), tr("The local branch is deleted; its remote branch is kept."), tr("Delete"))
    };
    if ok {
        app.vcs.delete_branch(name, force);
        return Outcome::Close;
    }
    if cancel || esc { Outcome::Close } else { Outcome::Keep }
}

fn delete_remote_branch(app: &mut DotCraftApp, ctx: &Context, name: &str) -> Outcome {
    let remote = name.split_once('/').map_or(name, |(r, _)| r);
    let ((ok, cancel), esc) = confirm(
        ctx,
        "delete_remote_branch",
        &trf("Delete '{branch}' on the remote?", &[("branch", &name)]),
        &trf("The branch is deleted on the server ({remote}) for everyone who uses it. This cannot be undone from DotCraft.", &[("remote", &remote)]),
        tr("Delete on Remote"),
    );
    if ok {
        app.vcs.delete_remote_branch(name);
        return Outcome::Close;
    }
    if cancel || esc { Outcome::Close } else { Outcome::Keep }
}

/// The commits that differ between `branch` and the current branch; clicking one shows it in
/// the Git window.
fn compare_branch(app: &mut DotCraftApp, ctx: &Context, branch: &str) -> Outcome {
    let current = app.vcs.branch_name().unwrap_or_else(|| "HEAD".into());
    let compare = app.vcs.compare.clone().filter(|c| c.branch == branch);
    let mut open = None;
    let mut close = false;
    let (_, esc) = modal(ctx, "compare_branch", 620.0, |ui| {
        title(ui, &trf("Compare '{branch}' with '{current}'", &[("branch", &branch), ("current", &current)]));
        let p = rc_kit::pal(ui);
        match &compare {
            None => crate::ui::kit::note(ui, tr("Loading…")),
            Some(c) if c.loading => crate::ui::kit::note(ui, tr("Loading…")),
            Some(c) if c.error.is_some() => crate::ui::kit::message(ui, c.error.as_deref().unwrap_or_default(), true),
            Some(c) => {
                egui::ScrollArea::vertical().max_height(420.0).auto_shrink([false, true]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (heading, commits) in [
                        (trf("In '{branch}' but not in '{current}' ({n})", &[("branch", &branch), ("current", &current), ("n", &c.incoming.len())]), &c.incoming),
                        (trf("In '{current}' but not in '{branch}' ({n})", &[("branch", &branch), ("current", &current), ("n", &c.outgoing.len())]), &c.outgoing),
                    ] {
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new(heading).font(theme::bold(12.5)));
                        ui.add_space(4.0);
                        if commits.is_empty() {
                            ui.label(egui::RichText::new(tr("No commits")).size(12.0).color(p.muted));
                        }
                        for commit in commits.iter() {
                            let secondary = format!("{}  {}  {}", commit.short_hash(), commit.author_name, crate::vcs::format_date(commit.author_time));
                            if crate::ui::kit::row(ui, 0, Some(rc_kit::ico::COMMIT), &commit.subject, &secondary, false).clicked() {
                                open = Some(commit.hash.clone());
                            }
                        }
                    }
                });
            }
        }
        ui.add_space(12.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            close = theme::primary_button(ui, tr("Close")).clicked();
        });
    });
    if let Some(hash) = open {
        app.dock.show(crate::toolwindow::ToolWindow::Git);
        app.vcs.select_commit(&hash);
        app.vcs.compare = None;
        return Outcome::Close;
    }
    if close || esc {
        app.vcs.compare = None;
        return Outcome::Close;
    }
    Outcome::Keep
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
    use super::*;
    use egui_kittest::kittest::Queryable;
    #[test]
    fn recent_files_filter_clamps_selection_and_opens_a_dotnet_file() {
        let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        let fixtures = crate::screens::fixtures().join("Shop");
        let solution = fixtures.join("Shop.sln");
        let order = fixtures.join("src/Shop.Core/Order.cs");
        let program = fixtures.join("src/Shop.Cli/Program.cs");
        let weather = fixtures.join("src/Shop.Api/WeatherForecast.cs");
        let mut app = crate::screens::test_app(home.path(), true);
        app.workspace = Some(crate::app::WorkspaceState {
            ws: rc_core::Workspace::open(&solution).unwrap(),
            solution: Some(rc_dotnet::Solution::load(&solution).unwrap()),
        });
        for path in [&order, &program, &weather] {
            app.open_file(path);
        }
        assert_eq!(app.docs.active_doc().unwrap().path, weather);
        app.dialog = Some(Dialog::RecentFiles { selected: 12, filter: "Order.cs".into() });
        let mut harness = crate::screens::harness(app, vec2(1000.0, 700.0));
        assert!(matches!(harness.state().dialog, Some(Dialog::RecentFiles { selected: 0, .. })));
        harness.get_by_role_and_label(egui::accesskit::Role::Button, &order.display().to_string());
        harness.key_press(Key::Enter);
        harness.run_steps(3);
        assert_eq!(harness.state().docs.active_doc().unwrap().path, order);
        assert!(harness.state().dialog.is_none());
    }

    #[test]
    fn code_actions_keys_do_not_reach_the_editor() {
        let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        let file = crate::screens::fixtures().join("Shop/src/Shop.Core/Order.cs");
        let mut app = crate::screens::test_app(home.path(), true);
        app.open_file(&file);
        app.focus_editor = true;
        let mut harness = crate::screens::harness(app, vec2(1000.0, 700.0));
        harness.run_steps(2);
        let editor = rc_editor::EditorView::id_for(harness.state().docs.active_doc().unwrap().editor.id);
        assert!(harness.ctx.memory(|m| m.has_focus(editor)));
        let original = harness.state().docs.active_doc().unwrap().editor.buffer.rope().to_string();
        let caret = harness.state().docs.active_doc().unwrap().editor.selection.clone();
        let action = |title: &str| rc_lsp::lsp_types::CodeActionOrCommand::Command(rc_lsp::lsp_types::Command { title: title.into(), command: "noop".into(), arguments: None });
        harness.state_mut().dialog = Some(Dialog::CodeActions { list: vec![action("First"), action("Second")], selected: 0 });
        harness.run_steps(2);
        harness.key_press(Key::ArrowDown);
        harness.run_steps(1);
        assert!(matches!(harness.state().dialog, Some(Dialog::CodeActions { selected: 1, .. })));
        harness.key_press(Key::Enter);
        harness.run_steps(2);
        let doc = harness.state().docs.active_doc().unwrap();
        assert!(harness.state().dialog.is_none());
        assert_eq!(doc.editor.buffer.rope().to_string(), original, "Enter on the code actions menu edited the document");
        assert_eq!(doc.editor.selection, caret, "arrow keys on the code actions menu moved the caret");
        assert!(harness.ctx.memory(|m| m.has_focus(editor)), "editor focus not restored after the menu closed");
    }

    #[test]
    fn recent_locations_selects_and_opens_a_dotnet_source_position() {
        let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        let fixtures = crate::screens::fixtures().join("Shop");
        let solution = fixtures.join("Shop.sln");
        let file = fixtures.join("src/Shop.Core/Order.cs");
        let mut app = crate::screens::test_app(home.path(), true);
        app.workspace = Some(crate::app::WorkspaceState {
            ws: rc_core::Workspace::open(&solution).unwrap(),
            solution: Some(rc_dotnet::Solution::load(&solution).unwrap()),
        });
        app.open_file(&file);
        let pos = app.docs.active_doc().unwrap().editor.buffer.rope().line_to_char(3);
        app.nav_back = vec![(file.clone(), pos)];
        app.dialog = Some(Dialog::RecentLocations { selected: 99 });
        let mut harness = crate::screens::harness(app, vec2(1000.0, 700.0));
        assert!(matches!(harness.state().dialog, Some(Dialog::RecentLocations { selected: 0 })));
        harness.get_by_role_and_label(egui::accesskit::Role::Button, &format!("{}:4", file.display()));
        harness.key_press(Key::Enter);
        harness.run_steps(3);
        assert_eq!(harness.state().docs.active_doc().unwrap().editor.caret_line_col().0, 3);
        assert!(harness.state().dialog.is_none());
    }

    #[test]
    fn settings_apply_and_cancel_with_shop_solution_open() {
        let _home = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        let solution = crate::screens::fixtures().join("Shop/Shop.sln");
        let mut app = crate::screens::test_app(home.path(), true);
        app.workspace = Some(crate::app::WorkspaceState {
            ws: rc_core::Workspace::open(&solution).unwrap(),
            solution: Some(rc_dotnet::Solution::load(&solution).unwrap()),
        });
        let original = app.settings.tab_colors;
        let mut harness = crate::screens::harness(app, vec2(1000.0, 700.0));
        let draft = Dialog::settings(&harness.state().settings);
        harness.state_mut().dialog = Some(draft);
        harness.run_steps(3);
        harness.get_by_label(tr("Editor")).click();
        harness.run_steps(2);
        harness.get_by_label(tr("Color tabs by kind of file")).click();
        harness.get_by_label(tr("OK")).click();
        harness.run_steps(3);
        assert_eq!(harness.state().settings.tab_colors, !original);
        assert_eq!(Settings::load().tab_colors, !original);
        assert_eq!(harness.state().workspace.as_ref().unwrap().solution.as_ref().unwrap().projects().len(), 6);

        let draft = Dialog::settings(&harness.state().settings);
        harness.state_mut().dialog = Some(draft);
        harness.run_steps(3);
        harness.get_by_label(tr("Editor")).click();
        harness.run_steps(2);
        harness.get_by_label(tr("Color tabs by kind of file")).click();
        harness.get_by_label(tr("Cancel")).click();
        harness.run_steps(3);
        assert_eq!(harness.state().settings.tab_colors, !original);
        assert_eq!(Settings::load().tab_colors, !original);
    }

    #[test]
    fn responsive_settings_keep_pages_and_confirmation_inside_the_viewport() {
        let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        for size in [vec2(1280.0, 800.0), vec2(800.0, 500.0), vec2(800.0, 300.0), vec2(640.0, 400.0), vec2(480.0, 320.0)] {
            let mut harness = crate::screens::harness(crate::screens::test_app(home.path(), true), size);
            let mut widths = Vec::new();
            for (page, _) in settings_pages() {
                harness.state_mut().dialog = Some(Dialog::Settings { draft: Box::new(harness.state().settings.clone()), page });
                harness.run_steps(5);
                let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
                let mut heights = Vec::new();
                for _ in 0..15 {
                    harness.run_steps(1);
                    let modal = harness.ctx.memory(|memory| memory.area_rect(Id::new("settings"))).unwrap();
                    assert!(viewport.contains_rect(modal), "settings outside {size:?}: {modal:?}");
                    heights.push(modal.height());
                }
                assert!(heights.windows(2).all(|pair| (pair[0] - pair[1]).abs() < 1.0), "settings height must settle: {heights:?}");
                let modal = harness.ctx.memory(|memory| memory.area_rect(Id::new("settings"))).unwrap();
                widths.push(modal.width());
                for label in [tr("OK"), tr("Cancel")] {
                    assert!(viewport.contains_rect(harness.get_by_label(label).rect()), "{label} must remain reachable at {size:?}");
                }
            }
            assert!(widths.windows(2).all(|pair| (pair[0] - pair[1]).abs() < 1.0), "settings width must remain stable across pages: {widths:?}");
        }
    }

    #[test]
    fn responsive_settings_scroll_to_the_last_editor_option_and_cancel_the_draft() {
        let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        for size in [vec2(480.0, 320.0), vec2(800.0, 300.0)] {
            let app = crate::screens::test_app(home.path(), true);
            let original = app.settings.tab_colors;
            let mut harness = crate::screens::harness(app, size);
            harness.state_mut().dialog = Some(Dialog::Settings { draft: Box::new(harness.state().settings.clone()), page: SettingsPage::Appearance });
            harness.run_steps(3);
            if SettingsLayout::for_viewport(size).compact {
                harness.get_by_label(tr("Appearance")).click();
                harness.run_steps(2);
            }
            harness.get_by_label(tr("Editor")).click();
            harness.run_steps(3);
            harness.input_mut().events.push(egui::Event::PointerMoved(egui::pos2(size.x * 0.6, size.y * 0.5)));
            harness.input_mut().events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, -1000.0),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            });
            harness.run_steps(20);
            let option = harness.get_by_label(tr("Color tabs by kind of file"));
            assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(option.rect()));
            option.click();
            harness.run_steps(2);
            match &harness.state().dialog {
                Some(Dialog::Settings { draft, page: SettingsPage::Editor }) => assert_eq!(draft.tab_colors, !original),
                _ => panic!("editor settings must remain open after editing"),
            }
            harness.get_by_label(tr("Cancel")).click();
            harness.run_steps(2);
            assert!(harness.state().dialog.is_none());
            assert_eq!(harness.state().settings.tab_colors, original);
        }
    }

    #[test]
    fn responsive_settings_tool_fields_fit_long_paths_without_horizontal_scrolling() {
        let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        for size in [vec2(480.0, 320.0), vec2(800.0, 300.0)] {
            let app = crate::screens::test_app(home.path(), true);
            let mut draft = app.settings.clone();
            let paths: Vec<String> = ["dotnet", "roslyn", "netcoredbg"].into_iter().map(|tool| format!("/tools/{}/{tool}", "long-folder/".repeat(20))).collect();
            draft.dotnet_path = Some(paths[0].clone().into());
            draft.roslyn_path = Some(paths[1].clone().into());
            draft.netcoredbg_path = Some(paths[2].clone().into());
            let mut harness = crate::screens::harness(app, size);
            harness.state_mut().dialog = Some(Dialog::Settings { draft: Box::new(draft), page: SettingsPage::Tools });
            harness.run_steps(5);
            let modal = harness.ctx.memory(|memory| memory.area_rect(Id::new("settings"))).unwrap();
            for path in paths {
                let field = harness.get(egui_kittest::kittest::By::new().value(&path).role(egui::accesskit::Role::TextInput)).rect();
                assert!(field.left() >= modal.left() && field.right() <= modal.right(), "tool path field outside modal: {field:?}");
                assert!(field.width() > 60.0, "path field must remain usable at {size:?}: {field:?}");
            }
            for browse in harness.get_all_by_label(tr("Browse…")) {
                assert!(browse.rect().left() >= modal.left() && browse.rect().right() <= modal.right());
            }
        }
    }

    #[test]
    fn skeleton_namespace_follows_the_project_folder() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("Shop.Core");
        std::fs::create_dir_all(proj.join("Models")).unwrap();
        std::fs::write(proj.join("Shop.Core.csproj"), "<Project/>").unwrap();
        let s = crate::item_templates::ItemKind::Class.files(&proj.join("Models"), "Order")[0].1.clone();
        assert!(s.starts_with("namespace Shop.Core.Models;"), "{s}");
        assert!(s.contains("public class Order"));
    }

    #[test]
    fn shop_file_and_interface_templates_create_and_open_their_files() {
        let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let root = project.path().canonicalize().unwrap().join("Shop.Core");
        std::fs::create_dir(&root).unwrap();
        std::fs::copy(crate::screens::fixtures().join("Shop/src/Shop.Core/Shop.Core.csproj"), root.join("Shop.Core.csproj")).unwrap();
        let dir = root.join("Orders");
        std::fs::create_dir(&dir).unwrap();
        let mut app = crate::screens::test_app(home.path(), true);
        app.settings.roslyn_path = Some("/nonexistent".into());
        app.open_workspace(&crate::screens::fixtures().join("Shop/Shop.sln"));
        let mut harness = crate::screens::harness(app, vec2(1000.0, 700.0));
        for (kind, name, relative, expected) in [
            (crate::item_templates::ItemKind::File, "docs/notes.md", "docs/notes.md", ""),
            (crate::item_templates::ItemKind::Interface, "IClock", "IClock.cs", "namespace Shop.Core.Orders;\n\npublic interface IClock"),
        ] {
            harness.state_mut().dialog = Some(Dialog::NewFile { dir: dir.clone(), name: name.into(), kind, error: None });
            harness.run_steps(2);
            harness.get_by_label(tr("Create")).click();
            harness.run_steps(2);
            let path = dir.join(relative);
            let content = std::fs::read_to_string(&path).unwrap();
            assert!(content.contains(expected), "{path:?}: {content}");
            assert_eq!(harness.state().docs.active_doc().unwrap().path, path);
        }
    }
}

#[cfg(test)]
mod responsive_settings_tests {
    use super::*;
    use egui_kittest::kittest::Queryable;

    #[test]
    fn settings_stay_bounded_and_keep_the_footer_visible_on_every_page() {
        let _home = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        for size in [vec2(1280.0, 800.0), vec2(800.0, 500.0), vec2(800.0, 300.0), vec2(640.0, 400.0), vec2(480.0, 320.0)] {
            let mut app = crate::screens::test_app(home.path(), true);
            // Load the real solution model without starting external SDK/server processes.
            let solution_path = crate::screens::fixtures().join("TenProjects/TenProjects.slnx");
            app.workspace = Some(crate::app::WorkspaceState {
                ws: rc_core::Workspace::open(&solution_path).unwrap(),
                solution: Some(rc_dotnet::Solution::load(&solution_path).unwrap()),
            });
            let mut harness = crate::screens::harness(app, size);
            let mut widths = Vec::new();
            for page in [SettingsPage::Appearance, SettingsPage::Editor, SettingsPage::Keymap, SettingsPage::Tools, SettingsPage::VersionControl] {
                let mut draft = Settings::default();
                if page == SettingsPage::Tools {
                    let long_path = PathBuf::from(format!("/opt/{}/dotnet", "long-installation-folder/".repeat(20)));
                    draft.dotnet_path = Some(long_path.clone());
                    draft.roslyn_path = Some(long_path.clone());
                    draft.netcoredbg_path = Some(long_path);
                }
                harness.state_mut().dialog = Some(Dialog::Settings { draft: Box::new(draft), page });
                harness.run_steps(5);
                let modal = harness.ctx.memory(|memory| memory.area_rect(Id::new("settings"))).unwrap();
                let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
                let ok = harness.get_by_label(tr("OK")).rect();
                let cancel = harness.get_by_label(tr("Cancel")).rect();
                assert!(viewport.contains_rect(modal), "modal outside {size:?}: {modal:?}");
                assert!(viewport.contains_rect(ok) && viewport.contains_rect(cancel), "footer outside {size:?}: {ok:?} {cancel:?}");
                for _ in 0..20 {
                    harness.run_steps(1);
                    let settled = harness.ctx.memory(|memory| memory.area_rect(Id::new("settings"))).unwrap();
                    assert!((settled.height() - modal.height()).abs() < 1.0, "modal height changed: {modal:?} -> {settled:?}");
                }
                if page == SettingsPage::Tools {
                    for browse in harness.get_all_by_label(tr("Browse…")) {
                        assert!(viewport.contains_rect(browse.rect()), "tool browse button outside viewport");
                    }
                }
                widths.push(modal.width());
            }
            assert!(widths.windows(2).all(|pair| (pair[0] - pair[1]).abs() < 1.0), "page widths differ at {size:?}: {widths:?}");
        }
    }

    #[test]
    fn cancelling_settings_preserves_the_original_configuration() {
        let _home = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        let app = crate::screens::test_app(home.path(), true);
        let original_size = app.settings.editor_font_size;
        let mut draft = app.settings.clone();
        draft.editor_font_size = original_size + 4.0;
        let mut harness = crate::screens::harness(app, vec2(480.0, 320.0));
        harness.state_mut().dialog = Some(Dialog::Settings { draft: Box::new(draft), page: SettingsPage::Appearance });
        harness.run_steps(5);
        harness.get_by_label(tr("Appearance")).click();
        harness.run_steps(2);
        harness.get_by_label(tr("Editor")).click();
        harness.run_steps(2);
        assert!(matches!(harness.state().dialog, Some(Dialog::Settings { page: SettingsPage::Editor, .. })));
        harness.get_by_label(tr("Cancel")).click();
        harness.run_steps(2);
        assert!(harness.state().dialog.is_none());
        assert_eq!(harness.state().settings.editor_font_size, original_size);
    }
}

/// Accept a decimal PID; exclude the IDE itself to avoid self-debugging deadlocks.
fn attach_pid(text: &str) -> Option<u32> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse::<u32>().ok().filter(|&pid| pid != 0 && pid != std::process::id())
}

fn attach_process(app: &mut DotCraftApp, ctx: &Context, text: &mut String, focus: &mut bool) -> Outcome {
    let pid = attach_pid(text);
    let available = app.workspace.is_some() && !app.debugger.active() && app.debugger.downloading.is_none();
    let mut enter = false;
    let ((attach, cancel), esc) = modal(ctx, "attach_process", 460.0, |ui| {
        let p = rc_kit::pal(ui);
        title(ui, tr("Attach to Process…"));
        ui.label(tr("Attach to a running .NET process by its process ID."));
        ui.add_space(10.0);
        let response = ui.add(egui::TextEdit::singleline(text).hint_text(tr("Process ID")).desired_width(f32::INFINITY).margin(vec2(6.0, 4.0)));
        if std::mem::take(focus) {
            response.request_focus();
        }
        enter = (response.has_focus() || response.lost_focus()) && ui.input(|i| i.key_pressed(Key::Enter));
        ui.add_space(4.0);
        ui.label(egui::RichText::new(tr("Stopping debugging detaches and leaves the process running.")).size(12.0).color(p.muted));
        if app.workspace.is_none() {
            ui.label(egui::RichText::new(tr("Open a project before attaching to a process")).size(12.5).color(p.warn));
        } else if !available {
            ui.label(egui::RichText::new(tr("Stop the current debug session before attaching")).size(12.5).color(p.warn));
        } else if !text.is_empty() && pid.is_none() {
            ui.label(egui::RichText::new(tr("Enter a positive process ID other than DotCraft's own process")).size(12.5).color(p.error));
        }
        buttons(ui, tr("Attach"), pid.is_some() && available)
    });
    if cancel || esc {
        return Outcome::Close;
    }
    // Parse after the input widget updates the text, including Enter in the same frame.
    if (attach || enter) && available && let Some(pid) = attach_pid(text) {
        let outputs = app.debugger.attach(pid, app.settings.netcoredbg_path.as_deref(), &app.docs);
        app.handle_debug(outputs);
        return Outcome::Close;
    }
    Outcome::Keep
}

#[cfg(test)]
mod attach_tests {
    use super::attach_pid;

    #[test]
    fn validates_external_process_ids() {
        assert_eq!(attach_pid(" 42 "), Some(42).filter(|&pid| pid != std::process::id()));
        for text in ["", "0", "-1", "+1", "1.5", "42abc", "4294967296", &std::process::id().to_string()] {
            assert_eq!(attach_pid(text), None, "{text}");
        }
    }

    #[test]
    fn attach_dialog_requires_a_workspace_for_button_and_enter() {
        use super::{Dialog, Settings};
        use egui_kittest::kittest::{NodeT, Queryable};
        use rc_core::{Waker, Workspace};
        let _lock = crate::screens::HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        rc_kit::i18n::set_lang(rc_kit::i18n::Lang::En);
        for mode in ["welcome", "standalone", "workspace"] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("project");
            std::fs::create_dir(&root).unwrap();
            let adapter = temp.path().join("not-an-executable-adapter");
            // Adapter selection is deterministic, but this non-executable file cannot
            // start an adapter or attach to an unrelated real operating-system process.
            std::fs::write(&adapter, b"").unwrap();
            let mut app = crate::app::DotCraftApp::with_settings(
                Settings { netcoredbg_path: Some(adapter), ..Settings::default() }, Waker::noop(), None,
            );
            if mode == "standalone" {
                let file = root.join("notes.txt");
                std::fs::write(&file, "Standalone file").unwrap();
                app.open_file(&file);
            } else if mode == "workspace" {
                app.workspace = Some(crate::app::WorkspaceState { ws: Workspace::open(&root).unwrap(), solution: None });
            }
            assert!(app.runner.selected_config().is_none());
            app.run_ide_action(&egui::Context::default(), rc_editor::Action::AttachProcess);
            let pid = if std::process::id() == 42 { "43" } else { "42" };
            app.dialog = Some(Dialog::AttachProcess { pid: pid.into(), focus: true });
            let mut first = true;
            let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(800.0, 600.0)).build_ui_state(
                move |ui, app: &mut crate::app::DotCraftApp| {
                    if std::mem::take(&mut first) { rc_kit::theme::install_fonts(ui.ctx()); return; }
                    super::show(app, ui.ctx());
                }, app,
            );
            harness.run_steps(3);
            assert_eq!(harness.get_by_label("Attach").accesskit_node().is_disabled(), mode != "workspace", "{mode}");
            if mode != "workspace" { harness.get_by_label("Open a project before attaching to a process"); }
            harness.get_by_role(egui::accesskit::Role::TextInput).focus();
            harness.run_steps(1);
            harness.key_press(egui::Key::Enter);
            harness.run_steps(2);
            if mode == "workspace" {
                assert!(harness.state().debugger.session.is_some());
                assert_eq!(harness.state().dock.bottom, Some(crate::toolwindow::ToolWindow::Debug));
            } else {
                assert!(matches!(harness.state().dialog, Some(Dialog::AttachProcess { .. })));
                assert!(harness.state().debugger.session.is_none());
                assert!(harness.state().debugger.downloading.is_none());
            }
        }
    }

}

#[cfg(all(test, unix))]
mod sdk_settings_return_tests {
    use super::*;
    use egui_kittest::kittest::{NodeT, Queryable};
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn closing_or_saving_sdk_chooser_restores_settings_draft_and_cancel_discards_it() {
        let _home = crate::screens::HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        for save in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("project");
            std::fs::create_dir(&root).unwrap();
            let fixture = crate::screens::fixtures().join("Shop/src/Shop.Api/Shop.Api.csproj");
            std::fs::copy(fixture, root.join("Shop.Api.csproj")).unwrap();
            let host = temp.path().join("dotnet");
            std::fs::write(&host, "#!/bin/sh\nprintf '10.0.100 [/sdk]\\n'\n").unwrap();
            std::fs::set_permissions(&host, std::fs::Permissions::from_mode(0o755)).unwrap();
            let mut app = crate::screens::test_app(&temp.path().join("home"), true);
            app.workspace = Some(crate::app::WorkspaceState { ws: rc_core::Workspace::open(&root).unwrap(), solution: None });
            let original_path = app.settings.dotnet_path.clone();
            let original_font = app.settings.editor_font_size;
            let mut draft = app.settings.clone();
            draft.dotnet_path = None;
            draft.editor_font_size = original_font + 2.0;
            app.dialog = Some(Dialog::Settings { draft: Box::new(draft), page: SettingsPage::Tools });
            let mut h = crate::screens::harness(app, vec2(800.0, 500.0));
            h.run_steps(5);
            h.get_by_role_and_label(egui::accesskit::Role::TextInput, tr(".NET CLI")).focus();
            h.run_steps(1);
            h.get_by_role_and_label(egui::accesskit::Role::TextInput, tr(".NET CLI")).type_text(host.to_str().unwrap());
            h.run_steps(2);
            h.get_by_label(tr("Manage project SDK…")).click();
            h.run_steps(3);
            assert!(matches!(h.state().dialog, Some(Dialog::SdkSelection { .. })));
            if save {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                while h.get_by_role(egui::accesskit::Role::ComboBox).accesskit_node().is_disabled() && std::time::Instant::now() < deadline {
                    h.run_steps(1);
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                h.get_by_role(egui::accesskit::Role::ComboBox).click();
                h.run_steps(2);
                h.get_by_label("10.0.100").click();
                h.run_steps(2);
                h.get_by_label(tr("Save SDK selection")).click();
                h.run_steps(2);
                while matches!(h.state().dialog, Some(Dialog::SdkSelection { .. })) && std::time::Instant::now() < deadline {
                    h.run_steps(1);
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            } else {
                h.get_by_label(tr("Close")).click();
                h.run_steps(3);
            }
            h.run_steps(3);
            match h.state().dialog.as_ref() {
                Some(Dialog::Settings { draft, page: SettingsPage::Tools }) => {
                    assert_eq!(draft.dotnet_path.as_ref(), Some(&host));
                    assert_eq!(draft.editor_font_size, original_font + 2.0);
                }
                _ => panic!("SDK close must return to the Tools settings draft"),
            }
            assert_eq!(h.state().settings.dotnet_path, original_path);
            assert_eq!(h.state().settings.editor_font_size, original_font);
            h.get_by_label(tr("Cancel")).click();
            h.run_steps(2);
            assert!(h.state().dialog.is_none());
            assert_eq!(h.state().settings.dotnet_path, original_path);
            assert_eq!(h.state().settings.editor_font_size, original_font);
            assert_eq!(root.join("global.json").exists(), save);
        }
    }
}
