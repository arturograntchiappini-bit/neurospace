//! Remembered choices: last vault, theme, transparency and text size (`app.json`).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Smallest text size factor.
pub const MIN_ZOOM: f32 = 0.75;
/// Largest text size factor.
pub const MAX_ZOOM: f32 = 2.0;
/// One step of the text size keys and menu entries.
pub const ZOOM_STEP: f32 = 0.1;

/// Which theme the user asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    /// Follow the operating system.
    #[default]
    System,
    /// Always light.
    Light,
    /// Always dark.
    Dark,
}

/// The contents of `app.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The vault to reopen at start.
    pub last_vault: Option<PathBuf>,
    /// The chosen theme.
    pub theme: ThemeChoice,
    /// Draw every panel fully opaque.
    pub reduce_transparency: bool,
    /// Text size factor, 0.75 to 2.0.
    pub zoom: f32,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            last_vault: None,
            theme: ThemeChoice::System,
            reduce_transparency: false,
            zoom: 1.0,
        }
    }
}

impl Settings {
    /// Reads `path`; a missing, unreadable or invalid file gives the defaults.
    pub fn load(path: &Path) -> Settings {
        let parsed = fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<Settings>(&text).ok());
        let mut settings = parsed.unwrap_or_default();
        settings.zoom = clamp_zoom(settings.zoom);
        settings
    }

    /// Writes `path` through a temporary file, creating the parent folder.
    ///
    /// # Errors
    /// Any I/O error of creating the folder, writing or renaming.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, serde_json::to_vec_pretty(self)?)?;
        fs::rename(&temporary, path)
    }

    /// Changes the text size by `delta`, in tenths, within the allowed range.
    pub fn step_zoom(&mut self, delta: f32) {
        self.zoom = clamp_zoom(((self.zoom + delta) * 10.0).round() / 10.0);
    }
}

fn clamp_zoom(zoom: f32) -> f32 {
    if zoom.is_nan() {
        return 1.0;
    }
    zoom.clamp(MIN_ZOOM, MAX_ZOOM)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_return_defaults_when_file_missing_or_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.json");
        assert_eq!(Settings::load(&path), Settings::default());
        fs::write(&path, "not json").unwrap();
        assert_eq!(Settings::load(&path), Settings::default());
        fs::write(&path, r#"{"zoom": "x"}"#).unwrap();
        assert_eq!(Settings::load(&path), Settings::default());
    }

    #[test]
    fn should_clamp_zoom_when_out_of_range() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.json");
        fs::write(&path, r#"{"zoom": 9.0}"#).unwrap();
        assert_eq!(Settings::load(&path).zoom, 2.0);
        fs::write(&path, r#"{"zoom": 0.1}"#).unwrap();
        assert_eq!(Settings::load(&path).zoom, 0.75);
    }

    #[test]
    fn should_round_trip_when_saved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("app.json");
        let settings = Settings {
            last_vault: Some(PathBuf::from("some/vault")),
            theme: ThemeChoice::Dark,
            reduce_transparency: true,
            zoom: 1.5,
        };
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path), settings);
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn should_step_zoom_in_tenths_within_range() {
        let mut settings = Settings::default();
        settings.step_zoom(ZOOM_STEP);
        settings.step_zoom(ZOOM_STEP);
        assert_eq!(settings.zoom, 1.2);
        settings.zoom = 0.8;
        settings.step_zoom(-ZOOM_STEP);
        settings.step_zoom(-ZOOM_STEP);
        assert_eq!(settings.zoom, 0.75);
        settings.zoom = 2.0;
        settings.step_zoom(ZOOM_STEP);
        assert_eq!(settings.zoom, 2.0);
    }
}
