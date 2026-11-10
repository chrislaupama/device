//! App preferences (Settings › Audio). Device setup is an app preference, not
//! project state (docs/adr/0002-recording.md), so it lives in a small JSON file
//! in the user's app-config directory, never in a project:
//!
//! - macOS: `~/Library/Application Support/Saine Studio/preferences.json`
//! - Linux: `$XDG_CONFIG_HOME/saine-studio/preferences.json` (or `~/.config/...`)
//! - Windows: `%APPDATA%\Saine Studio\preferences.json`
//!
//! `SAINE_PREFS_PATH` overrides the location. `--self-test` and the
//! `--state`/`--screenshot` runs never read or write this file.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioPrefs {
    /// Output device id (Core Audio UID on macOS); None = system default.
    pub output: Option<String>,
    pub output_name: Option<String>,
    /// Input device id; None = no input device. Only ever set by the user
    /// picking an input in Settings › Audio (opening an input can show the
    /// macOS microphone-permission prompt).
    pub input: Option<String>,
    pub input_name: Option<String>,
    /// 0 = device default.
    pub sample_rate: f64,
    /// 0 = device default.
    pub buffer_size: u32,
    pub input_muted: bool,
    /// Plugin delay compensation switched off (stored inverted so files
    /// written before this field default to compensation ON).
    #[serde(default)]
    pub pdc_off: bool,
}

impl AudioPrefs {
    /// True when the preferences ask for anything beyond the system defaults.
    pub fn is_custom(&self) -> bool {
        self.output.is_some() || self.input.is_some() || self.sample_rate > 0.0 || self.buffer_size > 0
    }

    pub fn device_config(&self) -> saine_engine::DeviceConfig {
        saine_engine::DeviceConfig { output: self.output.clone(), input: self.input.clone(), sample_rate: self.sample_rate, buffer_size: self.buffer_size }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub audio: AudioPrefs,
    /// Interface zoom in percent (View › Zoom In / Out / Reset); 0 = default (100).
    #[serde(default)]
    pub interface_zoom: u32,
    /// Browser › Files › Folders: folders the user added, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub browser_folders: Vec<String>,
}

/// The app's support directory (see the module docs), shared by the preferences
/// file and the plugin scan cache.
pub fn support_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    Some(if cfg!(target_os = "macos") {
        home?.join("Library/Application Support/Saine Studio")
    } else if cfg!(windows) {
        PathBuf::from(std::env::var_os("APPDATA")?).join("Saine Studio")
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| home.map(|h| h.join(".config")))?.join("saine-studio")
    })
}

/// Default preferences file location (see the module docs).
pub fn default_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("SAINE_PREFS_PATH") {
        return Some(PathBuf::from(p));
    }
    Some(support_dir()?.join("preferences.json"))
}

/// Plugin scan cache (`saine_engine::scan_plugins`), next to the preferences:
/// `~/Library/Application Support/Saine Studio/plugin-cache.json` on macOS.
/// `SAINE_PLUGIN_CACHE` overrides the location.
pub fn plugin_cache_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("SAINE_PLUGIN_CACHE") {
        return Some(PathBuf::from(p));
    }
    Some(support_dir()?.join("plugin-cache.json"))
}

/// Missing file = defaults; a corrupt file is reported and ignored.
pub fn load(path: &Path) -> Result<Option<Preferences>, String> {
    match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).map(Some).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

pub fn save(path: &Path, p: &Preferences) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(p).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_and_defaults() {
        let dir = std::env::temp_dir().join(format!("saine-prefs-test-{}", std::process::id()));
        let path = dir.join("preferences.json");
        assert_eq!(load(&path).unwrap(), None);
        let p = Preferences { audio: AudioPrefs { output: Some("out-uid".into()), buffer_size: 256, input_muted: true, ..Default::default() }, interface_zoom: 110, browser_folders: vec!["/tmp/kit".into()] };
        save(&path, &p).unwrap();
        assert_eq!(load(&path).unwrap(), Some(p.clone()));
        assert!(p.audio.is_custom());
        assert!(!AudioPrefs::default().is_custom());
        // Unknown/missing fields are tolerated.
        std::fs::write(&path, r#"{"audio":{"buffer_size":512},"future":1}"#).unwrap();
        assert_eq!(load(&path).unwrap().unwrap().audio.buffer_size, 512);
        let _ = std::fs::remove_dir_all(dir);
    }
}
