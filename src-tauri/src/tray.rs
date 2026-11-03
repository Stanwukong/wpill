//! Tray icon: version label, launch-at-startup toggle, Claude Code, Codex and Google connections, pause routines, quit.

use std::sync::OnceLock;

use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Wry};

use crate::{agent, autostart, google, sessions};

const TRAY_ID: &str = "main";
const ID_AUTOSTART: &str = "autostart";
const ID_CLAUDE: &str = "claude";
const ID_CODEX: &str = "codex";
const ID_GOOGLE: &str = "google";
const ID_QUIT: &str = "quit";
const ID_PAUSE_ROUTINES: &str = "pause-routines";

/// The "Pause routines" item, so a pause from the command window shows here too.
static PAUSE_ROUTINES: OnceLock<CheckMenuItem<Wry>> = OnceLock::new();
/// The Google item, relabelled when a sign-in starts, finishes or is revoked.
static GOOGLE: OnceLock<MenuItem<Wry>> = OnceLock::new();

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let version = MenuItem::with_id(
        app,
        "version",
        format!("Cay {}", app.package_info().version),
        false,
        None::<&str>,
    )?;
    let separator = PredefinedMenuItem::separator(app)?;
    // The registry is the source of truth (see autostart.rs), so read it rather than caching.
    let autostart = CheckMenuItem::with_id(
        app,
        ID_AUTOSTART,
        "Launch at startup",
        true,
        autostart::is_enabled(),
        None::<&str>,
    )?;
    let claude = MenuItem::with_id(app, ID_CLAUDE, sessions::tray_label(), true, None::<&str>)?;
    let codex = MenuItem::with_id(
        app,
        ID_CODEX,
        sessions::codex_tray_label(),
        true,
        None::<&str>,
    )?;
    sessions::codex_keep_tray_item(codex.clone());
    let google = MenuItem::with_id(app, ID_GOOGLE, google::tray_label(app), true, None::<&str>)?;
    let _ = GOOGLE.set(google.clone());
    let pause = CheckMenuItem::with_id(
        app,
        ID_PAUSE_ROUTINES,
        "Pause routines",
        true,
        agent::routines_paused(app),
        None::<&str>,
    )?;
    let _ = PAUSE_ROUTINES.set(pause.clone());
    let quit = MenuItem::with_id(app, ID_QUIT, "Quit Cay", true, None::<&str>)?;
    let items: [&dyn IsMenuItem<Wry>; 8] = [
        &version, &separator, &autostart, &claude, &codex, &google, &pause, &quit,
    ];
    let menu = Menu::with_items(app, &items)?;

    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Cay")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            ID_AUTOSTART => toggle_autostart(&autostart),
            ID_CLAUDE => sessions::toggle_from_tray(claude.clone()),
            ID_CODEX => sessions::codex_toggle_from_tray(codex.clone()),
            ID_GOOGLE => google::toggle_from_tray(app),
            ID_PAUSE_ROUTINES => agent::set_routines_paused(app, !agent::routines_paused(app)),
            ID_QUIT => app.exit(0),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

fn toggle_autostart(item: &CheckMenuItem<Wry>) {
    let enable = !autostart::is_enabled();
    match autostart::set_enabled(enable) {
        Ok(()) => log::info!("launch at startup: {enable}"),
        Err(e) => log::warn!("changing launch at startup failed: {e}"),
    }
    // The click already flipped the checkmark; make it show what the registry actually says.
    if let Err(e) = item.set_checked(autostart::is_enabled()) {
        log::warn!("updating tray checkmark failed: {e}");
    }
}

/// Shows whether routines are paused (the click already flipped it; a change elsewhere didn't).
pub fn show_routines_paused(paused: bool) {
    if let Some(item) = PAUSE_ROUTINES.get() {
        if item.is_checked().ok() != Some(paused) {
            if let Err(e) = item.set_checked(paused) {
                log::warn!("updating tray checkmark failed: {e}");
            }
        }
    }
}

/// Shows the Google item's label for the current sign-in state.
pub fn show_google(label: &str) {
    if let Some(item) = GOOGLE.get() {
        if item.text().ok().as_deref() != Some(label) {
            if let Err(e) = item.set_text(label) {
                log::warn!("updating tray label failed: {e}");
            }
        }
    }
}
