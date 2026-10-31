//! Tray icon: version label, launch-at-startup toggle, Claude Code connection, pause routines, quit.

use std::sync::OnceLock;

use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Wry};

use crate::{agent, autostart, claude};

const TRAY_ID: &str = "main";
const ID_AUTOSTART: &str = "autostart";
const ID_CLAUDE: &str = "claude";
const ID_QUIT: &str = "quit";
const ID_PAUSE_ROUTINES: &str = "pause-routines";

/// The "Pause routines" item, so a pause from the command window shows here too.
static PAUSE_ROUTINES: OnceLock<CheckMenuItem<Wry>> = OnceLock::new();

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let version = MenuItem::with_id(
        app,
        "version",
        format!("TopBar {}", app.package_info().version),
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
    let claude = MenuItem::with_id(app, ID_CLAUDE, claude::tray_label(), true, None::<&str>)?;
    let pause = CheckMenuItem::with_id(
        app,
        ID_PAUSE_ROUTINES,
        "Pause routines",
        true,
        agent::routines_paused(app),
        None::<&str>,
    )?;
    let _ = PAUSE_ROUTINES.set(pause.clone());
    let quit = MenuItem::with_id(app, ID_QUIT, "Quit TopBar", true, None::<&str>)?;
    let items: [&dyn IsMenuItem<Wry>; 6] =
        [&version, &separator, &autostart, &claude, &pause, &quit];
    let menu = Menu::with_items(app, &items)?;

    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("TopBar")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            ID_AUTOSTART => toggle_autostart(&autostart),
            ID_CLAUDE => claude::toggle_from_tray(claude.clone()),
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
