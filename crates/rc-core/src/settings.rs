//! User settings, stored as JSON in the data directory. Unknown or missing fields fall back to
//! their defaults, so files from older and newer versions both load.

use crate::paths;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeChoice {
    Light,
    #[default]
    Dark,
    /// Follow the operating system.
    System,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeymapChoice {
    /// Rider's default for the platform (macOS or Windows/Linux).
    #[default]
    Rider,
    VisualStudio,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeChoice,
    /// Interface language code (`es`, `fr`…); `None` follows the system.
    pub language: Option<String>,
    pub keymap: KeymapChoice,
    pub editor_font_size: f32,
    pub editor_line_spacing: f32,
    pub ui_zoom: f32,
    pub show_inlay_hints: bool,
    pub show_indent_guides: bool,
    pub right_margin: usize,
    /// Save files when the window loses focus or when switching tabs (IntelliJ autosave).
    pub autosave: bool,
    /// Open files from the explorer with a single click (otherwise a double click).
    pub single_click_open: bool,
    /// Explicit path to `dotnet` (otherwise found on PATH / the default install).
    pub dotnet_path: Option<PathBuf>,
    /// Explicit path to the Roslyn language server DLL or executable.
    pub roslyn_path: Option<PathBuf>,
    pub netcoredbg_path: Option<PathBuf>,
    /// Most recent first.
    pub recent_workspaces: Vec<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: ThemeChoice::Dark,
            language: None,
            keymap: KeymapChoice::Rider,
            editor_font_size: 13.0,
            editor_line_spacing: 1.2,
            ui_zoom: 1.0,
            show_inlay_hints: true,
            show_indent_guides: true,
            right_margin: 120,
            autosave: true,
            single_click_open: false,
            dotnet_path: None,
            roslyn_path: None,
            netcoredbg_path: None,
            recent_workspaces: Vec::new(),
        }
    }
}

impl Settings {
    pub fn file() -> PathBuf {
        paths::data_dir().join("settings.json")
    }

    /// Loads the settings; an unreadable file is kept aside as `settings.json.bak`.
    pub fn load() -> Settings {
        let file = Self::file();
        let Ok(text) = std::fs::read_to_string(&file) else { return Settings::default() };
        match serde_json::from_str(&text) {
            Ok(s) => s,
            Err(e) => {
                let _ = std::fs::copy(&file, file.with_extension("json.bak"));
                eprintln!("unreadable settings.json ({e}); using defaults");
                Settings::default()
            }
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        paths::write_atomic(&Self::file(), serde_json::to_string_pretty(self)?.as_bytes())?;
        Ok(())
    }

    /// Moves `path` to the top of the recent list (max 15 entries).
    pub fn add_recent(&mut self, path: PathBuf) {
        self.recent_workspaces.retain(|p| p != &path);
        self.recent_workspaces.insert(0, path);
        self.recent_workspaces.truncate(15);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_take_defaults() {
        let s: Settings = serde_json::from_str(r#"{"editor_font_size": 15.0, "unknown": 1}"#).unwrap();
        assert_eq!(s.editor_font_size, 15.0);
        assert_eq!(s.right_margin, 120);
    }

    #[test]
    fn recent_list_moves_to_front_without_duplicates() {
        let mut s = Settings::default();
        s.add_recent("/a".into());
        s.add_recent("/b".into());
        s.add_recent("/a".into());
        assert_eq!(s.recent_workspaces, vec![PathBuf::from("/a"), PathBuf::from("/b")]);
    }
}
