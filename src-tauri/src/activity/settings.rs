//! Per-activity on/off switches, persisted in `settings.json` in the app config dir.
//!
//! Plain JSON in the shape `tauri-plugin-store` reads, so Phase 4b can adopt the plugin (and add the UI)
//! without a migration. Until then the file is edited by hand; it's re-read at launch.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::ipc::Activity;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Switches {
    pub volume: bool,
    pub brightness: bool,
    pub bluetooth: bool,
    pub power: bool,
    pub caps_lock: bool,
    pub screenshot: bool,
    /// Claude Code sessions (activities and the panel section).
    pub claude: bool,
}

impl Default for Switches {
    fn default() -> Self {
        Self {
            volume: true,
            brightness: true,
            bluetooth: true,
            power: true,
            caps_lock: true,
            screenshot: true,
            claude: true,
        }
    }
}

impl Switches {
    pub fn allows(&self, activity: &Activity) -> bool {
        match activity {
            Activity::Volume(_) => self.volume,
            Activity::Brightness(_) => self.brightness,
            Activity::Bluetooth(_) => self.bluetooth,
            Activity::Power(_) => self.power,
            Activity::CapsLock(_) => self.caps_lock,
            Activity::Screenshot(_) => self.screenshot,
            Activity::ClaudeDone(_)
            | Activity::ClaudeNeedsInput(_)
            | Activity::AgentFinished(_) => self.claude,
            // A run waits on its approval, so it can't be switched off (it would only time out).
            Activity::AgentApproval(_) => true,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct File {
    activities: Switches,
    /// Keys owned by later phases survive a rewrite.
    #[serde(flatten)]
    other: serde_json::Map<String, serde_json::Value>,
}

fn path(app: &AppHandle) -> Option<PathBuf> {
    Some(app.path().app_config_dir().ok()?.join("settings.json"))
}

/// Reads the switches, writing the file with defaults (and any missing keys) so it's discoverable.
pub fn load(app: &AppHandle) -> Switches {
    let Some(path) = path(app) else {
        return Switches::default();
    };
    let file = read(&path).unwrap_or_default();
    let switches = file.activities;
    if let Err(e) = write(&path, &file) {
        log::debug!("writing {} failed: {e}", path.display());
    }
    log::info!("activity switches: {switches:?} ({})", path.display());
    switches
}

fn read(path: &Path) -> Option<File> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&text) {
        Ok(file) => Some(file),
        Err(e) => {
            log::warn!("ignoring unreadable {}: {e}", path.display());
            None
        }
    }
}

fn write(path: &Path, file: &File) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(file).map_err(std::io::Error::other)?;
    if std::fs::read_to_string(path).ok().as_deref() == Some(text.as_str()) {
        return Ok(());
    }
    std::fs::write(path, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_keys_default_on_and_unknown_keys_survive() {
        let file: File =
            serde_json::from_str(r#"{ "activities": { "volume": false }, "hotZone": 200 }"#)
                .unwrap();
        assert!(!file.activities.volume);
        assert!(file.activities.caps_lock);
        let out = serde_json::to_value(&file).unwrap();
        assert_eq!(out["hotZone"], 200);
        assert_eq!(out["activities"]["capsLock"], true);
    }
}
