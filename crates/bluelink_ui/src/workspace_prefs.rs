//! Persisted desktop workspace chrome (side docks only; note tabs stay session-local).

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const PREFS_VERSION: u8 = 1;
pub const DOCK_WIDTH_DEFAULT: f32 = crate::theme::SIDEBAR_WIDTH;
pub const DOCK_WIDTH_MIN: f32 = 200.0;
pub const DOCK_WIDTH_MAX: f32 = 420.0;
pub const RIGHT_DOCK_BREAKPOINT: f32 = 1000.0;
pub const LEFT_DOCK_BREAKPOINT: f32 = 900.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum LeftDockView {
    #[default]
    Files,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum RightDockView {
    #[default]
    Outline,
    Backlinks,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspacePrefs {
    pub version: u8,
    pub left_open: bool,
    pub right_open: bool,
    pub left_width: f32,
    pub right_width: f32,
    pub left_view: LeftDockView,
    pub right_view: RightDockView,
}

impl Default for WorkspacePrefs {
    fn default() -> Self {
        Self {
            version: PREFS_VERSION,
            left_open: true,
            right_open: true,
            left_width: DOCK_WIDTH_DEFAULT,
            right_width: DOCK_WIDTH_DEFAULT,
            left_view: LeftDockView::Files,
            right_view: RightDockView::Outline,
        }
    }
}

impl WorkspacePrefs {
    pub fn load() -> Self {
        Self::load_from(&config_path())
    }

    fn load_from(path: &Path) -> Self {
        let Ok(raw) = fs::read_to_string(path) else {
            return Self::default();
        };
        Self::from_json(&raw).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut normalized = self.clone();
        normalized.normalize();
        let json = serde_json::to_string_pretty(&normalized).map_err(|e| e.to_string())?;
        fs::write(path, json).map_err(|e| e.to_string())
    }

    fn from_json(raw: &str) -> Option<Self> {
        let mut prefs: Self = serde_json::from_str(raw).ok()?;
        prefs.normalize();
        Some(prefs)
    }

    pub fn normalize(&mut self) {
        if self.version != PREFS_VERSION {
            *self = Self::default();
            return;
        }
        if !self.left_width.is_finite() {
            self.left_width = DOCK_WIDTH_DEFAULT;
        }
        if !self.right_width.is_finite() {
            self.right_width = DOCK_WIDTH_DEFAULT;
        }
        self.left_width = self.left_width.clamp(DOCK_WIDTH_MIN, DOCK_WIDTH_MAX);
        self.right_width = self.right_width.clamp(DOCK_WIDTH_MIN, DOCK_WIDTH_MAX);
    }
}

pub fn config_path() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("bluelink").join("workspace.json");
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home)
        .join(".config")
        .join("bluelink")
        .join("workspace.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn defaults_match_desktop_workspace() {
        let prefs = WorkspacePrefs::default();
        assert_eq!(prefs.version, PREFS_VERSION);
        assert!(prefs.left_open);
        assert!(prefs.right_open);
        assert_eq!(prefs.left_width, 280.0);
        assert_eq!(prefs.right_width, 280.0);
        assert_eq!(prefs.left_view, LeftDockView::Files);
        assert_eq!(prefs.right_view, RightDockView::Outline);
        assert!(RIGHT_DOCK_BREAKPOINT > LEFT_DOCK_BREAKPOINT);
    }

    #[test]
    fn json_round_trip_preserves_dock_state() {
        let prefs = WorkspacePrefs {
            left_open: false,
            right_width: 344.0,
            right_view: RightDockView::Backlinks,
            ..WorkspacePrefs::default()
        };
        let raw = serde_json::to_string(&prefs).unwrap();
        assert_eq!(WorkspacePrefs::from_json(&raw), Some(prefs));
    }

    #[test]
    fn malformed_and_unknown_versions_fall_back() {
        assert_eq!(WorkspacePrefs::from_json("not json"), None);
        let raw = r#"{"version":99,"left_open":false}"#;
        assert_eq!(
            WorkspacePrefs::from_json(raw),
            Some(WorkspacePrefs::default())
        );
    }

    #[test]
    fn missing_and_corrupted_files_load_safe_defaults() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("workspace.json");
        assert_eq!(WorkspacePrefs::load_from(&path), WorkspacePrefs::default());

        fs::write(&path, "{ definitely not json").unwrap();
        assert_eq!(WorkspacePrefs::load_from(&path), WorkspacePrefs::default());
    }

    #[test]
    fn widths_are_finite_and_clamped() {
        let mut prefs = WorkspacePrefs {
            left_width: -20.0,
            right_width: f32::NAN,
            ..WorkspacePrefs::default()
        };
        prefs.normalize();
        assert_eq!(prefs.left_width, DOCK_WIDTH_MIN);
        assert_eq!(prefs.right_width, DOCK_WIDTH_DEFAULT);

        prefs.left_width = 900.0;
        prefs.right_width = 50.0;
        prefs.normalize();
        assert_eq!(prefs.left_width, DOCK_WIDTH_MAX);
        assert_eq!(prefs.right_width, DOCK_WIDTH_MIN);
    }
}
