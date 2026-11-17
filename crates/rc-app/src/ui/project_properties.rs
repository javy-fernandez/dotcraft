//! Common SDK-style project properties, edited without rewriting the project XML.

use std::path::{Path, PathBuf};

use egui::{Context, TextEdit, Ui};
use rc_core::{Inbox, Runtime, Waker};
use rc_dotnet::project_properties::ProjectPropertiesEditor;
use rc_kit::i18n::tr;
use rc_kit::theme;

use super::{dialogs, kit};

pub struct ProjectPropertiesDialog {
    path: PathBuf,
    loaded: Inbox<Result<(ProjectPropertiesEditor, String), String>>,
    saved: Inbox<Result<(), String>>,
    original: Option<ProjectPropertiesEditor>,
    source: Option<String>,
    draft: ProjectPropertiesEditor,
    loading: bool,
    saving: bool,
    error: Option<String>,
}

impl ProjectPropertiesDialog {
    pub fn new(path: PathBuf, waker: Waker) -> Self {
        let loaded = Inbox::new(waker.clone());
        let out = loaded.sender();
        let source = path.clone();
        Runtime::spawn(async move {
            out.send(ProjectPropertiesEditor::load(&source).map_err(|error| error.to_string()));
        });
        Self { path, loaded, saved: Inbox::new(waker), original: None, source: None, draft: ProjectPropertiesEditor::default(), loading: true, saving: false, error: None }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    #[cfg(test)]
    pub fn loading(&self) -> bool {
        self.loading
    }

    fn save(&mut self) {
        let (Some(original), Some(source)) = (&self.original, &self.source) else { return };
        let (draft, original, source, path) = (self.draft.clone(), original.clone(), source.clone(), self.path.clone());
        let out = self.saved.sender();
        self.saving = true;
        self.error = None;
        Runtime::spawn(async move {
            out.send(draft.save(&path, &original, &source).map(|_| ()).map_err(|error| error.to_string()));
        });
    }

    /// Returns `Some(true)` after saving or `Some(false)` when cancelled.
    pub fn show(&mut self, ctx: &Context) -> Option<bool> {
        for result in self.loaded.drain() {
            self.loading = false;
            match result {
                Ok((model, source)) => {
                    self.draft = model.clone();
                    self.original = Some(model);
                    self.source = Some(source);
                }
                Err(error) => self.error = Some(error),
            }
        }
        for result in self.saved.drain() {
            self.saving = false;
            match result {
                Ok(()) => return Some(true),
                Err(error) => self.error = Some(error),
            }
        }
        let width = (ctx.content_rect().width() - 48.0).clamp(340.0, 600.0);
        let field_width = (width - 170.0).max(160.0);
        let ((save, cancel), escape) = dialogs::modal(ctx, "project_properties", width, |ui| {
            dialogs::title(ui, tr("Project Properties"));
            kit::note(ui, &self.path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default());
            ui.add_space(8.0);
            if self.loading {
                ui.horizontal(|ui| {
                    ui.spinner();
                    kit::note(ui, tr("Loading project properties…"));
                });
                return dialogs::buttons(ui, tr("Save"), false);
            }
            if self.original.is_some() {
                egui::ScrollArea::vertical().max_height((ctx.content_rect().height() - 220.0).max(160.0)).auto_shrink([false, true]).show(ui, |ui| {
                    ui.add_enabled_ui(!self.saving, |ui| self.form(ui, field_width));
                });
            }
            if let Some(error) = &self.error {
                ui.add_space(6.0);
                kit::message(ui, error, true);
            }
            if self.saving {
                ui.add_space(6.0);
                ui.spinner();
            }
            dialogs::buttons(ui, tr("Save"), self.original.is_some() && !self.saving)
        });
        if save {
            self.save();
        }
        ((cancel || escape) && !self.saving).then_some(false)
    }

    fn form(&mut self, ui: &mut Ui, field_width: f32) {
        let draft = &mut self.draft;
        heading(ui, tr("Build properties"));
        egui::Grid::new("project_build_properties").num_columns(2).spacing([12.0, 10.0]).min_col_width(140.0).show(ui, |ui| {
            line(ui, tr("Target framework(s)"), &mut draft.target_frameworks, "net10.0;net8.0", field_width);
            choice(ui, tr("Nullable"), &mut draft.nullable, &["", "enable", "disable", "annotations", "warnings"]);
            line(ui, tr("Language version"), &mut draft.language_version, "latest", field_width);
            choice(ui, tr("Implicit usings"), &mut draft.implicit_usings, &["", "enable", "disable"]);
            choice(ui, tr("Warnings as errors"), &mut draft.warnings_as_errors, &["", "true", "false"]);
        });
        ui.add_space(14.0);
        heading(ui, tr("Package metadata"));
        egui::Grid::new("project_package_properties").num_columns(2).spacing([12.0, 10.0]).min_col_width(140.0).show(ui, |ui| {
            line(ui, tr("Package ID"), &mut draft.package_id, "", field_width);
            line(ui, tr("Version"), &mut draft.version, "1.0.0", field_width);
            line(ui, tr("Authors"), &mut draft.authors, "", field_width);
            ui.label(tr("Description"));
            ui.add(TextEdit::multiline(&mut draft.description).desired_rows(3).desired_width(field_width).margin(egui::vec2(6.0, 4.0)));
            ui.end_row();
            line(ui, tr("License expression"), &mut draft.license_expression, "MIT", field_width);
        });
    }
}

fn heading(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text).font(theme::bold(12.5)));
    ui.add_space(6.0);
}

fn line(ui: &mut Ui, label: &str, value: &mut String, hint: &str, width: f32) {
    let caption = ui.label(label);
    ui.add(TextEdit::singleline(value).hint_text(hint).desired_width(width).margin(egui::vec2(6.0, 4.0))).labelled_by(caption.id);
    ui.end_row();
}

/// A drop-down of MSBuild values; the empty value leaves the property to the SDK.
fn choice(ui: &mut Ui, label: &str, value: &mut String, options: &[&str]) {
    ui.label(label);
    let labels: Vec<String> = options.iter().map(|option| choice_label(option).to_string()).collect();
    if let Some(index) = kit::dropdown(ui, label, choice_label(value), &labels, true) {
        *value = options[index].to_string();
    }
    ui.end_row();
}

fn choice_label(value: &str) -> &str {
    match value {
        "" => tr("Inherited"),
        "enable" => tr("Enable"),
        "disable" => tr("Disable"),
        "annotations" => tr("Annotations"),
        "warnings" => tr("Warnings"),
        "true" => tr("True"),
        "false" => tr("False"),
        _ => value,
    }
}
