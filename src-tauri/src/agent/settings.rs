//! The `agent` block in `%APPDATA%\dev.wpill.topbar\settings.json`, next to the activity switches. Edited by
//! hand until Phase 4b; read at launch, and missing keys are written back with their defaults.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentSettings {
    /// Opens the command window.
    pub hotkey: String,
    /// Stops every running agent.
    pub stop_hotkey: String,
    /// `claude.exe` to run; `null` picks the newest one found.
    pub claude_path: Option<String>,
    /// `null` leaves the model to Claude Code (`--model` isn't passed).
    pub model: Option<String>,
    pub max_turns: u32,
    /// `null` for no budget cap.
    pub max_budget_usd: Option<f64>,
    /// Only TopBar's MCP server in agent runs (`--strict-mcp-config`); off loads your own servers too.
    pub strict_mcp_config: bool,
    /// Load your user settings (allow rules, hooks, plugins). Off passes `--setting-sources project,local`.
    pub user_settings: bool,
    /// An approval nobody answers is denied after this long.
    pub approval_timeout_secs: u64,
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            hotkey: "Win+Alt+A".into(),
            stop_hotkey: "Win+Alt+Escape".into(),
            claude_path: None,
            model: None,
            max_turns: 25,
            max_budget_usd: Some(1.0),
            strict_mcp_config: true,
            user_settings: true,
            approval_timeout_secs: 600,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct File {
    agent: AgentSettings,
    #[serde(flatten)]
    other: serde_json::Map<String, serde_json::Value>,
}

fn path(app: &AppHandle) -> Option<PathBuf> {
    Some(app.path().app_config_dir().ok()?.join("settings.json"))
}

pub fn load(app: &AppHandle) -> AgentSettings {
    let Some(path) = path(app) else {
        return AgentSettings::default();
    };
    let file = read(&path).unwrap_or_default();
    let settings = file.agent.clone();
    if let Err(e) = write(&path, &file) {
        log::debug!("writing {} failed: {e}", path.display());
    }
    log::info!("agent settings: {settings:?}");
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

/// `Win+Alt+A` → modifiers (with `MOD_NOREPEAT`) and a virtual-key code. Case and spaces don't matter.
pub fn parse_hotkey(text: &str) -> Option<(HOT_KEY_MODIFIERS, u32)> {
    let mut mods = MOD_NOREPEAT;
    let mut key = None;
    for part in text.split('+').map(|p| p.trim().to_ascii_lowercase()) {
        match part.as_str() {
            "win" | "windows" | "super" => mods |= MOD_WIN,
            "alt" => mods |= MOD_ALT,
            "ctrl" | "control" => mods |= MOD_CONTROL,
            "shift" => mods |= MOD_SHIFT,
            other if key.is_none() => key = Some(vk(other)?),
            _ => return None,
        }
    }
    // A hotkey needs a modifier (a bare letter would swallow typing everywhere).
    (mods != MOD_NOREPEAT).then_some(())?;
    Some((mods, key?))
}

fn vk(name: &str) -> Option<u32> {
    let bytes = name.as_bytes();
    if bytes.len() == 1 && (bytes[0].is_ascii_alphanumeric()) {
        return Some(u32::from(bytes[0].to_ascii_uppercase()));
    }
    if let Some(n) = name.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()) {
        if (1..=24).contains(&n) {
            return Some(0x70 + n - 1);
        }
    }
    Some(match name {
        "space" => 0x20,
        "escape" | "esc" => 0x1B,
        "enter" | "return" => 0x0D,
        "tab" => 0x09,
        "backspace" => 0x08,
        "pause" => 0x13,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkeys() {
        assert_eq!(
            parse_hotkey("Win+Alt+A"),
            Some((MOD_NOREPEAT | MOD_WIN | MOD_ALT, 0x41))
        );
        assert_eq!(
            parse_hotkey(" ctrl + shift + space "),
            Some((MOD_NOREPEAT | MOD_CONTROL | MOD_SHIFT, 0x20))
        );
        assert_eq!(
            parse_hotkey("Win+Alt+Escape"),
            Some((MOD_NOREPEAT | MOD_WIN | MOD_ALT, 0x1B))
        );
        assert_eq!(
            parse_hotkey("Alt+F12"),
            Some((MOD_NOREPEAT | MOD_ALT, 0x7B))
        );
        assert_eq!(parse_hotkey("A"), None);
        assert_eq!(parse_hotkey("Win+Alt"), None);
        assert_eq!(parse_hotkey("Win+A+B"), None);
        assert_eq!(parse_hotkey("Win+Hyper"), None);
    }

    #[test]
    fn defaults_fill_missing_keys_and_others_survive() {
        let file: File = serde_json::from_str(
            r#"{ "agent": { "maxTurns": 5 }, "activities": { "volume": false } }"#,
        )
        .unwrap();
        assert_eq!(file.agent.max_turns, 5);
        assert!(file.agent.strict_mcp_config);
        assert_eq!(file.agent.max_budget_usd, Some(1.0));
        let out = serde_json::to_value(&file).unwrap();
        assert_eq!(out["activities"]["volume"], false);
        assert_eq!(out["agent"]["hotkey"], "Win+Alt+A");
    }
}
