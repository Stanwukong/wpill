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
const VALUE_NAME: &str = "Cay";
/// The entry's name before the rename (D32).
const OLD_VALUE_NAME: &str = "TopBar";

/// Registered in the Run key and not switched off in Task Manager / Settings.
pub fn is_enabled() -> bool {
    let registered = CURRENT_USER
        .open(RUN_KEY)
        .and_then(|key| key.get_string(VALUE_NAME))
        .is_ok();
    registered && !disabled_by_user(VALUE_NAME)
}

pub fn set_enabled(enable: bool) -> io::Result<()> {
    if enable {
        let command = command_for(&std::env::current_exe()?);
        CURRENT_USER
            .create(RUN_KEY)?
            .set_string(VALUE_NAME, command)?;
    } else {
        remove_value(RUN_KEY, VALUE_NAME)?;
    }
    // Drop any Task Manager override so our choice is what takes effect.
    remove_value(APPROVED_KEY, VALUE_NAME)
}

/// Moves a `TopBar` entry from before the rename to `Cay`, once: if it was on (and not switched off in Task
/// Manager), this exe is registered instead; the old value and its switch are removed either way. Release builds
/// only: a dev build would register `target\debug` (the same rule as toggling, CLAUDE.md).
pub fn migrate_from_topbar() {
    let Ok(run) = CURRENT_USER.open(RUN_KEY) else {
        return;
    };
    if run.get_string(OLD_VALUE_NAME).is_err() {
        return;
    }
    if cfg!(debug_assertions) {
        log::info!("a TopBar startup entry exists; left for the installed build to move");
        return;
    }
    let was_on = !disabled_by_user(OLD_VALUE_NAME);
    let registered = run.get_string(VALUE_NAME).is_ok();
    if was_on && !registered {
        if let Err(e) = set_enabled(true) {
            log::warn!("moving the startup entry to Cay failed: {e}");
            return;
        }
    }
    for path in [RUN_KEY, APPROVED_KEY] {
        if let Err(e) = remove_value(path, OLD_VALUE_NAME) {
            log::warn!("removing the old TopBar startup entry failed: {e}");
        }
    }
    log::info!("moved the TopBar startup entry to Cay (was on: {was_on})");
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

fn disabled_by_user(name: &str) -> bool {
    CURRENT_USER
        .open(APPROVED_KEY)
        .and_then(|key| key.get_value(name))
        .map(|value| value.first().is_some_and(|flag| flag & 1 == 1))
        .unwrap_or(false)
}

fn remove_value(path: &str, name: &str) -> io::Result<()> {
    let result = CURRENT_USER
        .options()
        .read()
        .write()
        .open(path)
        .and_then(|key| key.remove_value(name));
    match result {
        Err(e) if e.code() != ERROR_FILE_NOT_FOUND.to_hresult() => Err(e.into()),
        _ => Ok(()),
    }
}
