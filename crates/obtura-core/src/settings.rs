//! Settings, kept as JSON in `~/.config/obtura/settings.json`.

use crate::{naming::DEFAULT_TEMPLATE, paths};
use obtura_scene::{Frame, Style};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Where "Save" puts pictures. `None` means `Pictures/Screenshots`.
    pub save_dir: Option<PathBuf>,
    /// File name template, see [`crate::naming`].
    pub name_template: String,
    /// Copy every new capture to the clipboard at once.
    pub copy_after_capture: bool,
    /// Stay running in the background after the first capture, so the next one is instant.
    pub keep_running: bool,
    /// Take the picture behind the region overlay through screen sharing (no GNOME flash; GNOME
    /// shows its sharing icon for a few seconds) rather than GNOME's screenshot service.
    pub quiet_capture: bool,
    /// The style each tool starts with, by tool name ("Set as default" in Formatting).
    pub tool_styles: BTreeMap<String, Style>,
    /// "My frame" in Beautify.
    pub frame: Option<Frame>,
    /// Put "my frame" around every new capture.
    pub auto_frame: bool,
    /// Languages for text reading, as tesseract's `-l` ("eng+fra"); empty: the installed ones.
    pub ocr_languages: String,
    /// Keep every capture in the library (history).
    pub history: bool,
    /// Read the text of each capture in the library, so it can be found by its words.
    pub history_text: bool,
    /// Remove library captures older than this many days (pinned ones stay); 0 keeps them all.
    pub history_days: u32,
    /// Screen recording.
    pub record: RecordSettings,
    /// How the colour picker writes the colour it copies.
    pub color_format: crate::color::ColorFormat,
    /// What the editor's New button takes ("region", "scrolling", "screen-5"...): the kind of
    /// the last capture that opened an editor, or what was chosen beside the button.
    pub new_capture: String,
    /// Windows: Print Screen and the other keys work in every app (on Linux these keys are in
    /// GNOME's own keyboard shortcuts, not here).
    pub keys_everywhere: bool,
    /// What Full Screen takes when there are several screens: [`FULL_SCREEN_ASK`] (Obtura asks,
    /// and may remember the answer), [`FULL_SCREEN_ALL`] (every screen in one picture), or the
    /// name of the screen chosen (as the app finds it again: "DP-7", `\\.\DISPLAY1`...).
    pub full_screen: String,
}

/// [`Settings::full_screen`]: ask which screen.
pub const FULL_SCREEN_ASK: &str = "";
/// [`Settings::full_screen`]: every screen.
pub const FULL_SCREEN_ALL: &str = "all";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecordFormat {
    Mp4,
    WebM,
    Gif,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecordSettings {
    pub format: RecordFormat,
    /// Record the microphone.
    pub microphone: bool,
    /// Record what the computer plays.
    pub computer_sound: bool,
    /// Show the mouse pointer in the recording.
    pub pointer: bool,
    /// Seconds counted down on the screen before recording starts (0: none).
    pub countdown: u32,
    /// GIF: frames per second, and size in percent of the area recorded.
    pub gif_fps: u32,
    pub gif_scale: u32,
    /// Where recordings are saved. `None` means `Videos/Screencasts`.
    pub dir: Option<PathBuf>,
    /// File name template, see [`crate::naming`].
    pub template: String,
}

pub const DEFAULT_RECORD_TEMPLATE: &str = "Recording %Y-%m-%d %H.%M.%S";

impl Default for RecordSettings {
    fn default() -> Self {
        Self { format: RecordFormat::Mp4, microphone: false, computer_sound: false, pointer: true, countdown: 3, gif_fps: 10, gif_scale: 100, dir: None, template: DEFAULT_RECORD_TEMPLATE.into() }
    }
}

impl RecordSettings {
    /// The folder recordings are saved in.
    pub fn dir(&self) -> PathBuf {
        self.dir.clone().unwrap_or_else(|| paths::videos_dir().join("Screencasts"))
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            save_dir: None,
            name_template: DEFAULT_TEMPLATE.into(),
            copy_after_capture: true,
            keep_running: true,
            quiet_capture: true,
            tool_styles: BTreeMap::new(),
            frame: None,
            auto_frame: false,
            ocr_languages: String::new(),
            history: true,
            history_text: true,
            history_days: 0,
            record: RecordSettings::default(),
            color_format: crate::color::ColorFormat::Hex,
            new_capture: "region".into(),
            keys_everywhere: true,
            full_screen: FULL_SCREEN_ASK.into(),
        }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        paths::config_dir().join("settings.json")
    }

    /// The folder pictures are saved in.
    pub fn save_dir(&self) -> PathBuf {
        self.save_dir.clone().unwrap_or_else(|| paths::pictures_dir().join("Screenshots"))
    }

    /// Reads the settings. A missing file gives the defaults. A file that cannot be read
    /// is copied to `settings.json.broken` (never deleted) and the defaults are used.
    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else { return Self::default() };
        serde_json::from_str(&text).unwrap_or_else(|_| {
            let _ = std::fs::copy(path, path.with_extension("json.broken"));
            Self::default()
        })
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&Self::path())
    }

    /// Writes through a temporary file, so a crash never leaves half a settings file.
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?)?;
        std::fs::rename(tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("obtura-settings-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn round_trip() {
        let dir = scratch("rt");
        let path = dir.join("sub/settings.json");
        let mut s = Settings { save_dir: Some("/pics".into()), name_template: "x %Y".into(), copy_after_capture: false, keep_running: false, frame: Some(Frame::default()), auto_frame: true, ..Settings::default() };
        s.tool_styles.insert("Line".into(), Style { width: 9.0, ..Style::default() });
        s.save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path), s);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_file_and_missing_keys_give_defaults() {
        let dir = scratch("def");
        assert_eq!(Settings::load_from(&dir.join("none.json")), Settings::default());
        let path = dir.join("partial.json");
        std::fs::write(&path, r#"{"copy_after_capture": false}"#).unwrap();
        let s = Settings::load_from(&path);
        assert!(!s.copy_after_capture && s.keep_running && s.name_template == DEFAULT_TEMPLATE);
        assert_eq!(s.new_capture, "region", "New takes a region until something else was used");
        assert_eq!(s.record, RecordSettings::default());
        // Settings written before recording existed, or with only part of it, still load.
        std::fs::write(&path, r#"{"record": {"format": "gif", "countdown": 0}}"#).unwrap();
        let s = Settings::load_from(&path);
        assert_eq!((s.record.format, s.record.countdown, s.record.gif_fps, s.record.pointer), (RecordFormat::Gif, 0, 10, true));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_broken_file_is_kept_aside_not_lost() {
        let dir = scratch("bad");
        let path = dir.join("settings.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(Settings::load_from(&path), Settings::default());
        assert_eq!(std::fs::read_to_string(dir.join("settings.json.broken")).unwrap(), "{ not json");
        let _ = std::fs::remove_dir_all(dir);
    }
}
