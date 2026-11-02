//! The `google` block in `%APPDATA%\io.github.stanwukong.cay\settings.json`, next to `activities` and `agent`.
//! Edited by hand until Phase 4b; read at launch, and missing keys are written back with their defaults. Never holds a
//! token: the sign-in lives in Credential Manager (`vault.rs`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GoogleSettings {
    /// Calendar ids to read; `null` reads the ones shown in Google Calendar (and the primary one).
    pub calendars: Option<Vec<String>>,
    /// Open tasks due today or overdue.
    pub tasks: bool,
    /// How often to look again while nothing else asks for it.
    pub refresh_minutes: u32,
    /// An event this close to its start is "soon": the pill's calendar icon shows.
    pub soon_minutes: u32,
    /// When the "starting soon" activity shows (Phase 6b).
    pub lead_minutes: u32,
    /// A Google OAuth client of your own (a Desktop app client), instead of the one this build carries.
    pub client_id: Option<String>,
    /// Its secret, if the client has one. Desktop secrets aren't confidential; it's still not a token.
    pub client_secret: Option<String>,
}

impl Default for GoogleSettings {
    fn default() -> Self {
        Self {
            calendars: None,
            tasks: true,
            refresh_minutes: 5,
            soon_minutes: 15,
            lead_minutes: 5,
            client_id: None,
            client_secret: None,
        }
    }
}

impl GoogleSettings {
    /// Clamped to sane ranges, whatever the file says.
    fn clamped(mut self) -> Self {
        self.refresh_minutes = self.refresh_minutes.clamp(1, 60);
        self.soon_minutes = self.soon_minutes.clamp(1, 120);
        self.lead_minutes = self.lead_minutes.clamp(1, 60);
        self
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct File {
    google: GoogleSettings,
    #[serde(flatten)]
    other: serde_json::Map<String, serde_json::Value>,
}

fn path(app: &AppHandle) -> Option<PathBuf> {
    Some(app.path().app_config_dir().ok()?.join("settings.json"))
}

pub fn load(app: &AppHandle) -> GoogleSettings {
    let Some(path) = path(app) else {
        return GoogleSettings::default();
    };
    let file = read(&path).unwrap_or_default();
    let settings = file.google.clone().clamped();
    if let Err(e) = write(&path, &file) {
        log::debug!("writing {} failed: {e}", path.display());
    }
    log::info!(
        "google settings: calendars {:?}, tasks {}, every {} min, soon {} min, own client {}",
        settings.calendars,
        settings.tasks,
        settings.refresh_minutes,
        settings.soon_minutes,
        settings.client_id.is_some()
    );
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
    fn defaults_fill_in_and_values_are_clamped() {
        let file: File = serde_json::from_str(
            r#"{ "google": { "refreshMinutes": 0, "calendars": ["a"] }, "agent": { "maxTurns": 3 } }"#,
        )
        .unwrap();
        let s = file.google.clone().clamped();
        assert_eq!(s.refresh_minutes, 1);
        assert_eq!(s.soon_minutes, 15);
        assert_eq!(s.calendars.as_deref(), Some(&["a".to_string()][..]));
        let out = serde_json::to_value(&file).unwrap();
        assert_eq!(out["agent"]["maxTurns"], 3);
        assert_eq!(out["google"]["clientId"], serde_json::Value::Null);
    }
}
