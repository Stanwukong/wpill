//! Launch at startup through the per-user Run registry key.
//!
//! We write the entry ourselves instead of using `tauri-plugin-autostart`: the plugin stores the
//! exe path unquoted (plus a trailing space), which breaks for paths with spaces such as
//! `C:\Users\DELL 7300\…`. Task Manager doesn't list such entries, and an unquoted path is a
//! known hijack risk.

use std::io;
use std::path::Path;

use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use windows_registry::CURRENT_USER;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// Task Manager and Settings store their enable/disable switch here; an odd first byte means disabled.
const APPROVED_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const VALUE_NAME: &str = "TopBar";

/// Registered in the Run key and not switched off in Task Manager / Settings.
pub fn is_enabled() -> bool {
    let registered = CURRENT_USER
        .open(RUN_KEY)
        .and_then(|key| key.get_string(VALUE_NAME))
        .is_ok();
    registered && !disabled_by_user()
}

pub fn set_enabled(enable: bool) -> io::Result<()> {
    if enable {
        let command = command_for(&std::env::current_exe()?);
        CURRENT_USER
            .create(RUN_KEY)?
            .set_string(VALUE_NAME, command)?;
    } else {
        remove_value(RUN_KEY)?;
    }
    // Drop any Task Manager override so our choice is what takes effect.
    remove_value(APPROVED_KEY)
}

/// Rewrites an existing entry that points at this exe in a stale form (e.g. the unquoted path the
/// autostart plugin used to write). Entries for a different exe, such as the installed app seen
/// from a dev build, are left alone.
pub fn repair() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Ok(key) = CURRENT_USER.options().read().write().open(RUN_KEY) else {
        return;
    };
    let Ok(current) = key.get_string(VALUE_NAME) else {
        return;
    };
    let expected = command_for(&exe);
    let target = current.trim().trim_matches('"');
    if current != expected && target.eq_ignore_ascii_case(&exe.display().to_string()) {
        match key.set_string(VALUE_NAME, &expected) {
            Ok(()) => log::info!("repaired startup entry: {expected}"),
            Err(e) => log::warn!("repairing startup entry failed: {e}"),
        }
    }
}

fn command_for(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

fn disabled_by_user() -> bool {
    CURRENT_USER
        .open(APPROVED_KEY)
        .and_then(|key| key.get_value(VALUE_NAME))
        .map(|value| value.first().is_some_and(|flag| flag & 1 == 1))
        .unwrap_or(false)
}

fn remove_value(path: &str) -> io::Result<()> {
    let result = CURRENT_USER
        .options()
        .read()
        .write()
        .open(path)
        .and_then(|key| key.remove_value(VALUE_NAME));
    match result {
        Err(e) if e.code() != ERROR_FILE_NOT_FOUND.to_hresult() => Err(e.into()),
        _ => Ok(()),
    }
}
