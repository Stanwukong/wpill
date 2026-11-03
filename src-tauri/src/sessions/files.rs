//! Settings files Cay edits for Connect (Claude Code's `settings.json`, Codex's `hooks.json`) and Cay's own
//! `settings.json` blocks kept for them. Merges happen in each agent's `hooks.rs`; this only reads and writes.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

/// The parsed file, `Ok(None)` if it doesn't exist yet.
pub fn read(path: &Path) -> Result<Option<Value>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => Ok(Some(json!({}))),
        Ok(text) => serde_json::from_str(&text).map(Some).map_err(|e| {
            format!(
                "{} isn't valid JSON ({e}); it was left unchanged",
                path.display()
            )
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("couldn't read {}: {e}", path.display())),
    }
}

/// Backs up the current file (if any) next to it, then replaces it atomically. Returns the backup path.
pub fn write(path: &Path, settings: &Value) -> Result<Option<PathBuf>, String> {
    let fail = |what: &str, e: std::io::Error| format!("couldn't {what} {}: {e}", path.display());
    let backup = if path.exists() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let backup = path.with_extension(format!("json.cay-{stamp}.bak"));
        std::fs::copy(path, &backup).map_err(|e| fail("back up", e))?;
        Some(backup)
    } else {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| fail("create the folder for", e))?;
        }
        None
    };
    let mut text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    text.push('\n');
    let temp = path.with_extension("json.cay-tmp");
    std::fs::write(&temp, text).map_err(|e| fail("write", e))?;
    std::fs::rename(&temp, path).map_err(|e| fail("replace", e))?;
    Ok(backup)
}

/// Cay's own `settings.json` (`%APPDATA%\io.github.stanwukong.cay`), the file Tauri's config dir points at. The
/// forwarder runs without Tauri, so it's found from the environment.
pub fn cay_settings_path() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(
        PathBuf::from(appdata)
            .join("io.github.stanwukong.cay")
            .join("settings.json"),
    )
}

/// One top-level block of Cay's `settings.json`, if the file and the block exist.
pub fn cay_block(key: &str) -> Option<Value> {
    let text = std::fs::read_to_string(cay_settings_path()?).ok()?;
    let file: Value = serde_json::from_str(&text).ok()?;
    file.get(key).cloned()
}

/// Sets (or with `None`, removes) one top-level block of Cay's `settings.json`; other keys stay.
pub fn set_cay_block(key: &str, block: Option<Value>) -> Result<(), String> {
    let path = cay_settings_path().ok_or("couldn't find Cay's settings folder")?;
    let mut file: Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    let root = file
        .as_object_mut()
        .ok_or("Cay's settings.json isn't an object")?;
    match block {
        Some(block) => {
            root.insert(key.into(), block);
        }
        None => {
            root.remove(key);
        }
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("couldn't write {}: {e}", path.display()))
}
