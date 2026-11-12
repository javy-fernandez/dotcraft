//! Common SDK-style project properties, edited without rewriting the project XML.

use std::path::{Path, PathBuf};

use egui::{Context, TextEdit};
use rc_core::{Inbox, Runtime, Waker};
use rc_dotnet::project_properties::ProjectPropertiesEditor;
use rc_kit::i18n::tr;

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
        Self {
            path,
            loaded,
            saved: Inbox::new(waker),
            original: None,
            source: None,
            draft: ProjectPropertiesEditor::default(),
            loading: true,
            saving: false,
            error: None,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn save(&mut self) {
        let (Some(original), Some(source)) = (&self.original, &self.source) else {
            return;
        };
        let draft = self.draft.clone();
        let original = original.clone();
        let source = source.clone();
        let path = self.path.clone();
        let out = self.saved.sender();
        self.saving = true;
        self.error = None;
        Runtime::spawn(async move {
            out.send(
                draft
                    .save(&path, &original, &source)
                    .map(|_| ())
                    .map_err(|error| error.to_string()),
            );
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
        let mut save = false;
        let mut cancel = false;
        let response = egui::Modal::new(egui::Id::new("project_properties")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 48.0).clamp(340.0, 650.0));
            super::dialogs::title(ui, tr("Project Properties"));
            ui.label(
                self.path
                    .file_name()
                    .map(|name| name.to_string_lossy())
                    .unwrap_or_default(),
            );
            if self.loading {
                ui.spinner();
                ui.label(tr("Loading project properties…"));
            } else if self.original.is_some() {
                egui::ScrollArea::vertical()
                    .max_height((ctx.content_rect().height() - 180.0).max(160.0))
                    .show(ui, |ui| {
                        ui.add_enabled_ui(!self.saving, |ui| {
                            ui.heading(tr("Build properties"));
                            egui::Grid::new("project_build_properties")
                                .num_columns(2)
                                .spacing([12.0, 8.0])
                                .show(ui, |ui| {
                                    ui.label(tr("Target framework(s)"));
                                    ui.add(
                                        TextEdit::singleline(&mut self.draft.target_frameworks)
                                            .hint_text("net10.0;net8.0")
                                            .desired_width(380.0),
                                    );
                                    ui.end_row();
                                    ui.label(tr("Nullable"));
                                    choice(
                                        ui,
                                        "nullable_property",
                                        &mut self.draft.nullable,
                                        &["", "enable", "disable", "annotations", "warnings"],
                                    );
                                    ui.end_row();
                                    ui.label(tr("Language version"));
                                    ui.add(
                                        TextEdit::singleline(&mut self.draft.language_version)
                                            .hint_text("latest")
                                            .desired_width(380.0),
                                    );
                                    ui.end_row();
                                    ui.label(tr("Implicit usings"));
                                    choice(
                                        ui,
                                        "implicit_usings_property",
                                        &mut self.draft.implicit_usings,
                                        &["", "enable", "disable"],
                                    );
                                    ui.end_row();
                                    ui.label(tr("Warnings as errors"));
                                    choice(
                                        ui,
                                        "warnings_as_errors_property",
                                        &mut self.draft.warnings_as_errors,
                                        &["", "true", "false"],
                                    );
                                    ui.end_row();
                                });
                            ui.add_space(10.0);
                            ui.heading(tr("Package metadata"));
                            egui::Grid::new("project_package_properties")
                                .num_columns(2)
                                .spacing([12.0, 8.0])
                                .show(ui, |ui| {
                                    field(ui, tr("Package ID"), &mut self.draft.package_id);
                                    field(ui, tr("Version"), &mut self.draft.version);
                                    field(ui, tr("Authors"), &mut self.draft.authors);
                                    ui.label(tr("Description"));
                                    ui.add(
                                        TextEdit::multiline(&mut self.draft.description)
                                            .desired_rows(3)
                                            .desired_width(380.0),
                                    );
                                    ui.end_row();
                                    field(
                                        ui,
                                        tr("License expression"),
                                        &mut self.draft.license_expression,
                                    );
                                });
                        });
                    });
            }
            if let Some(error) = &self.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        self.original.is_some() && !self.saving,
                        egui::Button::new(tr("Save")),
                    )
                    .clicked()
                {
                    save = true;
                }
                if ui
                    .add_enabled(!self.saving, egui::Button::new(tr("Cancel")))
                    .clicked()
                {
                    cancel = true;
                }
                if self.saving {
                    ui.spinner();
                }
            });
        });
        if save {
            self.save();
        }
        (cancel || (response.should_close() && !self.saving)).then_some(false)
    }
}

fn field(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.label(label);
    ui.add(TextEdit::singleline(value).desired_width(380.0));
    ui.end_row();
}

fn choice(ui: &mut egui::Ui, id: &str, value: &mut String, options: &[&str]) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(choice_label(value))
        .show_ui(ui, |ui| {
            for option in options {
                ui.selectable_value(value, (*option).to_string(), choice_label(option));
            }
        });
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
