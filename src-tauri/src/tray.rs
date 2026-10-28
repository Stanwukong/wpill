//! Tray icon: version label, launch-at-startup toggle, quit.

use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Wry};
use tauri_plugin_autostart::ManagerExt;

const TRAY_ID: &str = "main";
const ID_AUTOSTART: &str = "autostart";
const ID_QUIT: &str = "quit";

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let version = MenuItem::with_id(
        app,
        "version",
        format!("TopBar {}", app.package_info().version),
        false,
        None::<&str>,
    )?;
    let separator = PredefinedMenuItem::separator(app)?;
    // The autostart plugin keeps its state in the per-user Run registry key, so that's the source of truth.
    let autostart = CheckMenuItem::with_id(
        app,
        ID_AUTOSTART,
        "Launch at startup",
        true,
        autostart_enabled(app),
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, ID_QUIT, "Quit TopBar", true, None::<&str>)?;
    let items: [&dyn IsMenuItem<Wry>; 4] = [&version, &separator, &autostart, &quit];
    let menu = Menu::with_items(app, &items)?;

    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("TopBar")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            ID_AUTOSTART => toggle_autostart(app, &autostart),
            ID_QUIT => app.exit(0),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

fn autostart_enabled(app: &AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or_else(|e| {
        log::warn!("reading autostart state failed: {e}");
        false
    })
}

fn toggle_autostart(app: &AppHandle, item: &CheckMenuItem<Wry>) {
    let manager = app.autolaunch();
    let enable = !autostart_enabled(app);
    let result = if enable {
        manager.enable()
    } else {
        manager.disable()
    };
    match result {
        Ok(()) => log::info!("launch at startup: {enable}"),
        Err(e) => log::warn!("changing launch at startup failed: {e}"),
    }
    // The click already flipped the checkmark; make it show what the registry actually says.
    if let Err(e) = item.set_checked(autostart_enabled(app)) {
        log::warn!("updating tray checkmark failed: {e}");
    }
}
