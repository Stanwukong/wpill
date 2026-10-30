//! The `command` window: where you type an agent instruction and read its transcript. Unlike the island it's a
//! normal, focusable window (CLAUDE.md rule 2), created on first use and hidden (not closed) afterwards.
//!
//! Call `show` from a background thread or an async command, never from a sync command or the main thread:
//! building a webview window there deadlocks on Windows. The hotkey thread's `WM_HOTKEY` gives TopBar the
//! right to take the foreground, which is what lets `set_focus` work.

use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindow};

use crate::error::{AppError, AppResult};
use crate::ipc::events;

pub const LABEL: &str = "command";
const WIDTH: f64 = 720.0;
const HEIGHT: f64 = 560.0;

pub fn show(app: &AppHandle, focus_run: Option<String>) -> AppResult<()> {
    let window = match app.get_webview_window(LABEL) {
        Some(window) => window,
        None => create(app)?,
    };
    window.unminimize()?;
    window.show()?;
    window.set_focus()?;
    if let Some(run_id) = focus_run {
        window.emit_to(LABEL, events::COMMAND_FOCUS_RUN, run_id)?;
    }
    Ok(())
}

pub fn hide(app: &AppHandle) -> AppResult<()> {
    if let Some(window) = app.get_webview_window(LABEL) {
        window.hide()?;
    }
    Ok(())
}

pub fn hwnd(app: &AppHandle) -> Option<windows::Win32::Foundation::HWND> {
    app.get_webview_window(LABEL)?.hwnd().ok()
}

/// Windows' Text size setting (Accessibility → Text size) as a factor, e.g. 1.2. WebView2 folds it into
/// `devicePixelRatio` but Tauri's logical sizes don't include it, so the window grows with it to keep the same
/// CSS room (CLAUDE.md, "DPI and Windows Text size").
fn text_scale() -> f64 {
    windows_registry::CURRENT_USER
        .open(r"Software\Microsoft\Accessibility")
        .and_then(|key| key.get_u32("TextScaleFactor"))
        .map_or(1.0, |percent| f64::from(percent.clamp(100, 225)) / 100.0)
}

fn create(app: &AppHandle) -> AppResult<WebviewWindow> {
    let text = text_scale();
    let window =
        tauri::WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("command.html".into()))
            .title("TopBar")
            .inner_size(WIDTH * text, HEIGHT * text)
            .min_inner_size(520.0 * text, 380.0 * text)
            .decorations(false)
            .shadow(true)
            .resizable(true)
            .visible(false)
            .build()?;
    // Centered below the island, a little down from the top of the primary monitor.
    let monitor = window.primary_monitor()?.ok_or(AppError::NoMonitor)?;
    let size = window.outer_size()?;
    let origin = monitor.position();
    let x = origin.x + (monitor.size().width as i32 - size.width as i32) / 2;
    let y = origin.y + (monitor.size().height as f64 * 0.12) as i32;
    window.set_position(PhysicalPosition::new(x, y))?;
    Ok(window)
}
