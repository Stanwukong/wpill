//! OpenCode sessions (2.9a, D47): a plugin file Cay writes into OpenCode's `plugins` folder (`plugin.rs`) forwards
//! its events to the same `cay.exe hook`, and `tools.rs` renames OpenCode's tools for the rest of `sessions/`. This
//! module owns the tray's Connect/Disconnect OpenCode.

mod plugin;
mod tools;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::menu::MenuItem;
use tauri::Wry;
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONINFORMATION, MB_ICONWARNING, MB_OK};

use super::{confirm, message, this_exe, version};
use plugin::State;
pub use tools::claude_shaped;

static DIALOG_OPEN: AtomicBool = AtomicBool::new(false);

/// `$OPENCODE_CONFIG_DIR`, else `%USERPROFILE%\.config\opencode` (where OpenCode keeps its config on Windows too).
fn config_dir() -> Option<PathBuf> {
    std::env::var_os("OPENCODE_CONFIG_DIR")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .map(|home| Path::new(&home).join(".config").join("opencode"))
        })
}

fn plugin_path() -> Result<PathBuf, String> {
    config_dir()
        .map(|d| d.join("plugins").join(plugin::FILE_NAME))
        .ok_or_else(|| "couldn't find your user folder".to_string())
}

fn read(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("couldn't read {}: {e}", path.display())),
    }
}

fn state() -> State {
    let (Ok(path), Ok(exe)) = (plugin_path(), this_exe()) else {
        return State::NotConnected;
    };
    match read(&path) {
        Ok(text) => plugin::state(text.as_deref(), &exe),
        Err(_) => State::NotConnected,
    }
}

pub fn tray_label() -> &'static str {
    match state() {
        State::Connected => "Disconnect OpenCode…",
        State::OutOfDate => "Update OpenCode connection…",
        State::NotConnected | State::Taken => "Connect OpenCode…",
    }
}

/// Tray "Connect/Disconnect OpenCode…": a native dialog on its own thread, as for the other agents.
pub fn toggle_from_tray(item: MenuItem<Wry>) {
    if DIALOG_OPEN.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        if let Err(e) = run_dialog() {
            message(
                &format!("Cay couldn't change OpenCode's plugins:\n\n{e}"),
                MB_OK | MB_ICONWARNING,
            );
        }
        if let Err(e) = item.set_text(tray_label()) {
            log::warn!("updating tray label failed: {e}");
        }
        DIALOG_OPEN.store(false, Ordering::SeqCst);
    });
}

const ASK_NOTE: &str = "OpenCode allows edits and shell commands without asking unless your opencode.json says \
                        otherwise; Cay never changes that. To answer them in Cay, add \"permission\": { \"edit\": \
                        \"ask\", \"bash\": \"ask\" } to it yourself. Sessions that are already open pick the plugin up \
                        after a restart.";

fn run_dialog() -> Result<(), String> {
    let path = plugin_path()?;
    let exe = this_exe()?;
    match state() {
        State::Taken => {
            message(
                &format!(
                    "There's already a {} in\n{}\nthat isn't Cay's, so Cay won't replace it. Rename it to connect.",
                    plugin::FILE_NAME,
                    path.parent().map_or(path.clone(), Path::to_path_buf).display()
                ),
                MB_OK | MB_ICONINFORMATION,
            );
            return Ok(());
        }
        State::Connected => {
            let text = format!(
                "Remove Cay's plugin\n{}?\n\nOnly this file is deleted; your OpenCode settings stay as they are.",
                path.display()
            );
            if confirm(&text) {
                std::fs::remove_file(&path)
                    .map_err(|e| format!("couldn't delete {}: {e}", path.display()))?;
                log::info!("disconnected OpenCode ({})", path.display());
                message("OpenCode is disconnected.", MB_OK | MB_ICONINFORMATION);
            }
            return Ok(());
        }
        State::OutOfDate | State::NotConnected => {}
    }
    let found = version::installed("opencode", "node_modules\\opencode-ai");
    if let Some(v) = found.filter(|v| *v < plugin::MIN_VERSION) {
        message(
            &format!(
                "OpenCode {} is older than {}, the first version Cay works with. Update it (opencode upgrade), then \
                 connect again.",
                version::text(v),
                version::text(plugin::MIN_VERSION)
            ),
            MB_OK | MB_ICONINFORMATION,
        );
        return Ok(());
    }
    let checked = match found {
        Some(v) => format!("OpenCode {} found.", version::text(v)),
        None => format!(
            "Cay couldn't find OpenCode to check its version; it needs {} or later.",
            version::text(plugin::MIN_VERSION)
        ),
    };
    let home = match std::env::var_os("OPENCODE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        Some(_) => "\n\nOPENCODE_CONFIG_DIR is set, so this is that folder, not your usual one.",
        None => "",
    };
    let text = format!(
        "Cay will write a small plugin to\n{}\n\nOpenCode loads it at startup. It runs\n{} hook --agent opencode\nfor \
         each session event, and for a permission request it waits for your answer in Cay and passes it to OpenCode; \
         OpenCode's own prompt works as before. Nothing else in your OpenCode folder changes.\n\n{ASK_NOTE}\n\n\
         {checked}{home}",
        path.display(),
        exe.display()
    );
    if !confirm(&text) {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("couldn't create {}: {e}", dir.display()))?;
    }
    std::fs::write(&path, plugin::source(&exe))
        .map_err(|e| format!("couldn't write {}: {e}", path.display()))?;
    log::info!("connected OpenCode ({})", path.display());
    message("OpenCode is connected.", MB_OK | MB_ICONINFORMATION);
    Ok(())
}
