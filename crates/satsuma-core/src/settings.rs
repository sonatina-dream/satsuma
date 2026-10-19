//! User settings, stored as JSON (SPEC §9).

use crate::gesture::WheelCombos;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Strength {
    /// Smaller files with less quality loss.
    Balanced,
    /// Smaller files with more quality loss.
    Strong,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TerminalApp {
    Terminal,
    ITerm2,
}

impl TerminalApp {
    pub fn bundle_id(self) -> &'static str {
        match self {
            TerminalApp::Terminal => "com.apple.Terminal",
            TerminalApp::ITerm2 => "com.googlecode.iterm2",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            TerminalApp::Terminal => "Terminal",
            TerminalApp::ITerm2 => "iTerm2",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FloatBehavior {
    AddToExisting,
    CreateNew,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub wheels: WheelCombos,
    /// Click sound and haptic tick when a petal is highlighted.
    pub feedback: bool,
    pub image_compression: Strength,
    /// Longest edge in pixels when compressing images; `None` keeps the original dimensions.
    pub image_max_edge: Option<u32>,
    pub video_compression: Strength,
    pub audio_compression: Strength,
    /// Number of equal parts for Split Video.
    pub split_video_parts: u32,
    pub terminal: TerminalApp,
    pub float_behavior: FloatBehavior,
    pub has_completed_welcome: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            wheels: WheelCombos::default(),
            feedback: true,
            image_compression: Strength::Balanced,
            image_max_edge: None,
            video_compression: Strength::Balanced,
            audio_compression: Strength::Balanced,
            split_video_parts: 2,
            terminal: TerminalApp::Terminal,
            float_behavior: FloatBehavior::AddToExisting,
            has_completed_welcome: false,
        }
    }
}

impl Settings {
    /// `~/Library/Application Support/Satsuma/settings.json`
    pub fn default_path() -> PathBuf {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| "/tmp".into());
        home.join("Library/Application Support/Satsuma/settings.json")
    }

    /// Loads settings, falling back to defaults for a missing or unreadable file.
    pub fn load(path: &Path) -> Settings {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gesture::Mods;

    #[test]
    fn round_trip_through_file() {
        let dir = std::env::temp_dir().join(format!("satsuma-settings-{}", std::process::id()));
        let path = dir.join("settings.json");
        let mut s = Settings::default();
        s.wheels.tools = Mods::OPTION.union(Mods::SHIFT);
        s.image_max_edge = Some(2048);
        s.terminal = TerminalApp::ITerm2;
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path), s);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_or_partial_files_use_defaults() {
        assert_eq!(Settings::load(Path::new("/nonexistent/x.json")), Settings::default());
        let partial: Settings = serde_json::from_str(r#"{"feedback": false}"#).unwrap();
        assert!(!partial.feedback);
        assert_eq!(partial.split_video_parts, 2);
        assert_eq!(partial.wheels, WheelCombos::default());
    }
}
