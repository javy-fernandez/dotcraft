//! Explicit SDK pinning. Background work is scoped to the chooser's workspace snapshot.

use egui::Ui;
use rc_core::{Inbox, Waker};
use rc_dotnet::{OpHandle, SdkInfo, global_json::GlobalJson};
use rc_kit::i18n::{tr, trf};
use std::path::PathBuf;

pub struct SdkSelection {
    root: PathBuf,
    load: Inbox<Result<(GlobalJson, Vec<SdkInfo>), String>>,
    saved: Inbox<Result<(), String>>,
    _scan: OpHandle,
    file: Option<GlobalJson>,
    versions: Vec<SdkInfo>,
    selected: String,
    loading: bool,
    saving: bool,
    error: Option<String>,
}

impl SdkSelection {
    pub fn new(root: PathBuf, dotnet: PathBuf, waker: Waker) -> Self {
        let load = Inbox::new(waker.clone());
        let scan = rc_dotnet::global_json::start_load(root.clone(), dotnet, load.sender());
        Self {
            root,
            load,
            saved: Inbox::new(waker),
            _scan: scan,
            file: None,
            versions: Vec::new(),
            selected: String::new(),
            loading: true,
            saving: false,
            error: None,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        let width = (ctx.content_rect().width() - 48.0).clamp(120.0, 520.0);
        let response = egui::Modal::new(egui::Id::new("sdk_selection")).show(ctx, |ui| {
            ui.set_width(width);
            ui.label(egui::RichText::new(tr("Project SDK")).font(rc_kit::theme::bold(15.0)));
            ui.add_space(10.0);
            self.contents(ui)
        });
        let (close, can_close) = response.inner;
        close || (response.should_close() && can_close)
    }

    /// Returns true once the accepted write has completed; closing never blocks on SDK discovery.
    pub fn contents(&mut self, ui: &mut Ui) -> (bool, bool) {
        for result in self.load.drain() {
            self.loading = false;
            match result {
                Ok((file, versions)) => {
                    self.selected = file.version().unwrap_or_default().to_owned();
                    self.file = Some(file);
                    self.versions = versions;
                }
                Err(error) => self.error = Some(error),
            }
        }
        for result in self.saved.drain() {
            self.saving = false;
            match result {
                Ok(()) => return (true, true),
                Err(error) => self.error = Some(error),
            }
        }
        // Bound the body against the viewport, not the previous modal's available height.
        // Save and Close remain outside the scroll area even with long paths or errors.
        let body_height = (ui.ctx().content_rect().height() - 172.0).clamp(36.0, 400.0);
        egui::ScrollArea::vertical().id_salt("sdk_selection_body").max_height(body_height).auto_shrink([false, true]).show(ui, |ui| {
            ui.label(tr("Writes global.json in the project root. Ancestor files are never changed."));
            let path = self.root.join("global.json").display().to_string();
            ui.add(egui::Label::new(&path).truncate()).on_hover_text(&path);
            if self.loading || self.saving {
                ui.spinner();
            }
            if let Some(file) = &self.file {
                ui.label(trf("Current SDK pin: {version}", &[("version", &file.version().unwrap_or(tr("None")))]));
            }
            ui.add_enabled_ui(!self.loading && !self.saving, |ui| {
                ui.label(tr("Installed SDK"));
                egui::ComboBox::from_id_salt("workspace_sdk_version")
                    .width(ui.available_width().min(320.0))
                    .selected_text(if self.selected.is_empty() { tr("Select an SDK") } else { &self.selected })
                    .show_ui(ui, |ui| {
                        for sdk in &self.versions {
                            ui.selectable_value(&mut self.selected, sdk.version.clone(), &sdk.version);
                        }
                    });
                if !self.loading && self.versions.is_empty() {
                    ui.label(tr("No installed SDKs were found."));
                }
            });
            if let Some(error) = &self.error {
                ui.colored_label(rc_kit::pal(ui).error, trf("Could not update SDK selection: {error}", &[("error", error)]));
            }
        });
        ui.add_space(8.0);
        let can_save = !self.loading && !self.saving && self.versions.iter().any(|sdk| sdk.version == self.selected) && self.file.is_some();
        let (save, close) = ui
            .horizontal_wrapped(|ui| {
                let save = ui.add_enabled(can_save, egui::Button::new(tr("Save SDK selection"))).clicked();
                let close = ui.add_enabled(!self.saving, egui::Button::new(tr("Close"))).clicked();
                (save, close)
            })
            .inner;
        if save && let Some(file) = self.file.clone() {
            let version = self.selected.clone();
            let out = self.saved.sender();
            self.saving = true;
            self.error = None;
            std::thread::spawn(move || {
                out.send(file.save_version(&version).map_err(|e| format!("{e:#}")));
            });
        }
        (close, !self.saving)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

    #[test]
    fn sdk_chooser_only_writes_after_explicit_save() {
        rc_kit::i18n::set_lang(rc_kit::i18n::Lang::En);
        let root = tempfile::tempdir().unwrap();
        let host = root.path().join("dotnet");
        std::fs::write(&host, "#!/bin/sh\nprintf '10.0.100 [/sdk]\\n'\n").unwrap();
        std::fs::set_permissions(&host, std::fs::Permissions::from_mode(0o755)).unwrap();
        let state = SdkSelection::new(root.path().to_path_buf(), host, Waker::noop());
        let mut first = true;
        let mut h = Harness::builder().with_size(egui::vec2(600.0, 400.0)).build_ui_state(
            move |ui, state: &mut SdkSelection| {
                if std::mem::take(&mut first) {
                    rc_kit::theme::install_fonts(ui.ctx());
                    return;
                }
                state.show(ui.ctx());
            },
            state,
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        while h.state().loading && Instant::now() < deadline {
            h.run_steps(1);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!h.state().loading);
        assert!(h.state().error.is_none(), "{:?}", h.state().error);
        assert!(!root.path().join("global.json").exists());
        h.state_mut().selected = "10.0.100".into();
        h.run_steps(2);
        h.get_by_label("Save SDK selection").click();
        h.run_steps(2);
        while h.state().saving && Instant::now() < deadline {
            h.run_steps(1);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!h.state().saving);
        assert_eq!(GlobalJson::load(root.path()).unwrap().version(), Some("10.0.100"));
    }

    #[test]
    fn compact_sdk_chooser_keeps_save_and_close_reachable_with_long_paths() {
        rc_kit::i18n::set_lang(rc_kit::i18n::Lang::En);
        for size in [egui::vec2(480.0, 320.0), egui::vec2(800.0, 300.0)] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("long-workspace-folder-name".repeat(6)).join("nested-workspace-folder".repeat(6));
            std::fs::create_dir_all(&root).unwrap();
            let host = temp.path().join("dotnet");
            std::fs::write(&host, "#!/bin/sh\nprintf '10.0.100 [/sdk]\\n'\n").unwrap();
            std::fs::set_permissions(&host, std::fs::Permissions::from_mode(0o755)).unwrap();
            let state = SdkSelection::new(root.clone(), host, Waker::noop());
            let mut first = true;
            let mut h = Harness::builder().with_size(size).build_ui_state(
                move |ui, state: &mut SdkSelection| {
                    if std::mem::take(&mut first) {
                        rc_kit::theme::install_fonts(ui.ctx());
                        return;
                    }
                    state.show(ui.ctx());
                },
                state,
            );
            let deadline = Instant::now() + Duration::from_secs(10);
            while h.state().loading && Instant::now() < deadline {
                h.run_steps(1);
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(!h.state().loading);
            h.state_mut().selected = "10.0.100".into();
            h.state_mut().error = Some("A long diagnostic message with nested paths. ".repeat(30));
            h.run_steps(5);
            let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
            let modal = h.ctx.memory(|m| m.area_rect(egui::Id::new("sdk_selection"))).unwrap();
            assert!(viewport.contains_rect(modal), "modal outside {size:?}: {modal:?}");
            for label in ["Save SDK selection", "Close"] {
                let button = h.get_by_label(label).rect();
                assert!(viewport.contains_rect(button), "{label} outside {size:?}: {button:?}");
            }
            assert!(!root.join("global.json").exists());
            h.get_by_label("Save SDK selection").click();
            h.run_steps(2);
            while h.state().saving && Instant::now() < deadline {
                h.run_steps(1);
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(GlobalJson::load(&root).unwrap().version(), Some("10.0.100"));
            h.run_steps(2);
            h.get_by_label("Close").click();
            h.run_steps(1);
        }
    }
}
