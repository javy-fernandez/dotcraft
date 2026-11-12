//! Visual editing of a project's launchSettings.json profiles.

use std::path::PathBuf;

use egui::{Context, TextEdit};
use rc_core::{Inbox, Runtime, Waker};
use rc_dotnet::{LaunchProfile, LaunchSettings};
use rc_kit::i18n::tr;

pub struct LaunchProfileEditor {
    project: PathBuf,
    loaded: Inbox<Result<Vec<LaunchProfile>, String>>,
    saved: Inbox<Result<(), String>>,
    profiles: Vec<LaunchProfile>,
    original: Option<String>,
    draft: LaunchProfile,
    environment: String,
    loading: bool,
    saving: bool,
    error: Option<String>,
}

impl LaunchProfileEditor {
    pub fn new(project: PathBuf, waker: Waker) -> Self {
        let loaded = Inbox::new(waker.clone());
        let out = loaded.sender();
        let source = project.clone();
        Runtime::spawn(async move {
            out.send(
                LaunchSettings::load(&source)
                    .map(|settings| {
                        settings
                            .map(|settings| settings.profiles)
                            .unwrap_or_default()
                    })
                    .map_err(|error| error.to_string()),
            );
        });
        Self {
            project,
            loaded,
            saved: Inbox::new(waker),
            profiles: Vec::new(),
            original: None,
            draft: LaunchProfile::default(),
            environment: String::new(),
            loading: true,
            saving: false,
            error: None,
        }
    }

    fn select(&mut self, index: usize) {
        if let Some(profile) = self.profiles.get(index) {
            self.original = Some(profile.name.clone());
            self.draft = profile.clone();
            self.environment = profile
                .environment_variables
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>()
                .join("\n");
            self.error = None;
        }
    }

    fn create(&mut self) {
        self.original = None;
        self.draft = LaunchProfile {
            command_name: "Project".into(),
            ..Default::default()
        };
        self.environment.clear();
        self.error = None;
    }

    fn submit(&mut self, delete: bool) {
        let original = self.original.clone();
        let profile = if delete {
            None
        } else {
            let mut profile = self.draft.clone();
            profile.name = profile.name.trim().to_string();
            if profile.name.is_empty() {
                self.error = Some(tr("Enter a profile name").into());
                return;
            }
            if profile.command_name.trim().is_empty() {
                self.error = Some(tr("Enter a command name").into());
                return;
            }
            let mut variables = Vec::new();
            for line in self
                .environment
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
            {
                let Some((name, value)) = line.split_once('=') else {
                    self.error = Some(tr("Use NAME=value for each environment variable").into());
                    return;
                };
                let name = name.trim();
                if name.is_empty() || variables.iter().any(|(key, _)| key == name) {
                    self.error =
                        Some(tr("Environment variable names must be unique and nonempty").into());
                    return;
                }
                variables.push((name.to_string(), value.to_string()));
            }
            profile.environment_variables = variables;
            Some(profile)
        };
        if original.is_none() && profile.is_none() {
            return;
        }
        let project = self.project.clone();
        let out = self.saved.sender();
        self.saving = true;
        self.error = None;
        Runtime::spawn(async move {
            out.send(
                LaunchSettings::write_profile(&project, original.as_deref(), profile.as_ref())
                    .map_err(|error| error.to_string()),
            );
        });
    }

    /// Returns `Some(true)` after a write, or `Some(false)` when cancelled.
    pub fn show(&mut self, ctx: &Context) -> Option<bool> {
        for result in self.loaded.drain() {
            self.loading = false;
            match result {
                Ok(profiles) => {
                    self.profiles = profiles;
                    if self.profiles.is_empty() {
                        self.create();
                    } else {
                        self.select(0);
                    }
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
        let mut close = false;
        let mut add = false;
        let mut select = None;
        let mut save = false;
        let mut delete = false;
        let response = egui::Modal::new(egui::Id::new("launch_profile_editor")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 48.0).clamp(320.0, 640.0));
            super::dialogs::title(ui, tr("Edit launch profiles"));
            if self.loading {
                ui.spinner();
                ui.label(tr("Loading launch profiles…"));
                return;
            }
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("launch_profile_name")
                    .selected_text(self.original.as_deref().unwrap_or(tr("New profile")))
                    .show_ui(ui, |ui| {
                        for (index, profile) in self.profiles.iter().enumerate() {
                            if ui
                                .selectable_label(
                                    self.original.as_deref() == Some(profile.name.as_str()),
                                    &profile.name,
                                )
                                .clicked()
                            {
                                select = Some(index);
                            }
                        }
                    });
                if ui
                    .add_enabled(!self.saving, egui::Button::new(tr("Add profile")))
                    .clicked()
                {
                    add = true;
                }
                if ui
                    .add_enabled(
                        !self.saving && self.original.is_some(),
                        egui::Button::new(tr("Delete profile")),
                    )
                    .clicked()
                {
                    delete = true;
                }
            });
            ui.add_enabled_ui(!self.saving, |ui| {
                egui::Grid::new("launch_profile_fields")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        ui.label(tr("Profile name"));
                        ui.add(TextEdit::singleline(&mut self.draft.name).desired_width(380.0));
                        ui.end_row();
                        ui.label(tr("Command"));
                        ui.add(
                            TextEdit::singleline(&mut self.draft.command_name).desired_width(380.0),
                        );
                        ui.end_row();
                        ui.label(tr("Application URLs"));
                        ui.add(
                            TextEdit::singleline(
                                self.draft.application_url.get_or_insert_with(String::new),
                            )
                            .hint_text("http://localhost:5000;https://localhost:7000")
                            .desired_width(380.0),
                        );
                        ui.end_row();
                        ui.label(tr("Program arguments"));
                        ui.add(
                            TextEdit::singleline(
                                self.draft.command_line_args.get_or_insert_with(String::new),
                            )
                            .desired_width(380.0),
                        );
                        ui.end_row();
                        ui.label(tr("Launch URL"));
                        ui.add(
                            TextEdit::singleline(
                                self.draft.launch_url.get_or_insert_with(String::new),
                            )
                            .desired_width(380.0),
                        );
                        ui.end_row();
                        ui.label(tr("Working directory"));
                        ui.add(
                            TextEdit::singleline(
                                self.draft.working_directory.get_or_insert_with(String::new),
                            )
                            .desired_width(380.0),
                        );
                        ui.end_row();
                        ui.label(tr("Executable path"));
                        ui.add(
                            TextEdit::singleline(
                                self.draft.executable_path.get_or_insert_with(String::new),
                            )
                            .desired_width(380.0),
                        );
                        ui.end_row();
                        ui.label(tr("Environment variables"));
                        ui.add(
                            TextEdit::multiline(&mut self.environment)
                                .desired_rows(4)
                                .desired_width(380.0)
                                .font(egui::TextStyle::Monospace),
                        );
                        ui.end_row();
                    });
                ui.checkbox(&mut self.draft.launch_browser, tr("Launch browser"));
            });
            if let Some(error) = &self.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!self.saving, egui::Button::new(tr("Save")))
                    .clicked()
                {
                    save = true;
                }
                if ui
                    .add_enabled(!self.saving, egui::Button::new(tr("Cancel")))
                    .clicked()
                {
                    close = true;
                }
                if self.saving {
                    ui.spinner();
                }
            });
        });
        if let Some(index) = select {
            self.select(index);
        }
        if add {
            self.create();
        }
        if delete {
            self.submit(true);
        }
        if save {
            self.submit(false);
        }
        (close || (response.should_close() && !self.saving)).then_some(false)
    }
}
