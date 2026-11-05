//! The `processes` block in `settings.json` (Phase 3): the memory-heavy apps section, off by default. Edited by hand
//! until Phase 4b; read at launch, and missing keys are written back with their defaults.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProcessSettings {
    /// The panel's "Memory" section. Off by default: it lists other apps' processes.
    pub enabled: bool,
    /// How many apps the section lists.
    pub limit: u32,
}

impl Default for ProcessSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            limit: 6,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct File {
    processes: ProcessSettings,
    #[serde(flatten)]
    other: serde_json::Map<String, serde_json::Value>,
}

fn path(app: &AppHandle) -> Option<PathBuf> {
    Some(app.path().app_config_dir().ok()?.join("settings.json"))
}

pub fn load(app: &AppHandle) -> ProcessSettings {
    let Some(path) = path(app) else {
        return ProcessSettings::default();
    };
    let file = read(&path).unwrap_or_default();
    let mut settings = file.processes;
    settings.limit = settings.limit.clamp(1, 20);
    if let Err(e) = write(&path, &file) {
        log::debug!("writing {} failed: {e}", path.display());
    }
    log::info!("process settings: {settings:?}");
    settings
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
    fn off_by_default_and_other_blocks_survive() {
        let file: File = serde_json::from_str(
            r#"{ "activities": { "volume": false }, "processes": { "limit": 4 } }"#,
        )
        .unwrap();
        assert!(!file.processes.enabled);
        assert_eq!(file.processes.limit, 4);
        let out = serde_json::to_value(&file).unwrap();
        assert_eq!(out["activities"]["volume"], false);
        assert_eq!(out["processes"]["enabled"], false);
        let empty: File = serde_json::from_str("{}").unwrap();
        assert_eq!(empty.processes, ProcessSettings::default());
    }
}
