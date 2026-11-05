//! The `command` window: where you type an agent instruction and read its transcript. Unlike the island it's a
//! normal, focusable window (CLAUDE.md rule 2), created on first use and hidden (not closed) afterwards.
//!
//! Call `show` from a background thread or an async command, never from a sync command or the main thread:
//! building a webview window there deadlocks on Windows. The hotkey thread's `WM_HOTKEY` gives Cay the
//! right to take the foreground, which is what lets `set_focus` work.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WindowEvent,
};

use crate::error::{AppError, AppResult};
use crate::ipc::{events, CommandFocus};

/// What a window that was just created should select: its page isn't listening yet when `show` emits, so it takes
/// this once loaded (`command_take_focus`).
static PENDING: Mutex<Option<CommandFocus>> = Mutex::new(None);

/// Whether the window is maximized, as last emitted (`command:maximized`), so a resize only emits a change.
static MAXIMIZED: AtomicBool = AtomicBool::new(false);

pub const LABEL: &str = "command";
const WIDTH: f64 = 720.0;
const HEIGHT: f64 = 560.0;

pub fn show(app: &AppHandle, focus_run: Option<String>) -> AppResult<()> {
    focus(app, focus_run.map(|run_id| CommandFocus::Run { run_id }))
}

/// `show`, then selects a terminal session's held request (2.8b).
pub fn show_ask(app: &AppHandle, request_id: u32) -> AppResult<()> {
    focus(app, Some(CommandFocus::Ask { request_id }))
}

fn focus(app: &AppHandle, target: Option<CommandFocus>) -> AppResult<()> {
    let window = match app.get_webview_window(LABEL) {
        Some(window) => window,
        None => {
            *PENDING.lock().unwrap_or_else(|e| e.into_inner()) = target.clone();
            create(app)?
        }
    };
    window.unminimize()?;
    window.show()?;
    window.set_focus()?;
    match target {
        Some(CommandFocus::Run { run_id }) => {
            window.emit_to(LABEL, events::COMMAND_FOCUS_RUN, run_id)?
        }
        Some(CommandFocus::Ask { request_id }) => {
            window.emit_to(LABEL, events::COMMAND_FOCUS_ASK, request_id)?
        }
        None => {}
    }
    Ok(())
}

/// `command_take_focus`: what the page should select on load, once.
pub fn take_focus() -> Option<CommandFocus> {
    PENDING.lock().unwrap_or_else(|e| e.into_inner()).take()
}

pub fn hide(app: &AppHandle) -> AppResult<()> {
    if let Some(window) = app.get_webview_window(LABEL) {
        window.hide()?;
    }
    Ok(())
}

/// What Esc does (2.10c): a maximized window is restored first; only then does Esc hide it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Escape {
    Restore,
    Hide,
}

pub fn escape_action(maximized: bool) -> Escape {
    if maximized {
        Escape::Restore
    } else {
        Escape::Hide
    }
}

/// Esc in the window: restore if maximized, else hide.
pub fn escape(app: &AppHandle) -> AppResult<()> {
    let Some(window) = app.get_webview_window(LABEL) else {
        return Ok(());
    };
    match escape_action(window.is_maximized()?) {
        Escape::Restore => window.unmaximize()?,
        Escape::Hide => window.hide()?,
    }
    Ok(())
}

/// The Fullscreen button and F11 (2.10c, choice 5): native maximize, which fills the work area and leaves the
/// taskbar showing. Returns the new state; `command:maximized` follows from the resize as well.
pub fn toggle_maximize(app: &AppHandle) -> AppResult<bool> {
    let Some(window) = app.get_webview_window(LABEL) else {
        return Ok(false);
    };
    if window.is_maximized()? {
        window.unmaximize()?;
    } else {
        window.maximize()?;
    }
    window.is_maximized().map_err(Into::into)
}

pub fn is_maximized(app: &AppHandle) -> bool {
    app.get_webview_window(LABEL)
        .and_then(|window| window.is_maximized().ok())
        .unwrap_or(false)
}

/// Emits `command:maximized` when the state changed: maximize, restore, Win+Up, Snap and the title bar's
/// double-click (Tauri's own drag-region toggle) all end in a resize.
fn follow_maximized(window: &WebviewWindow) {
    let Ok(now) = window.is_maximized() else {
        return;
    };
    if MAXIMIZED.swap(now, Ordering::Relaxed) != now {
        if let Err(err) = window.emit_to(LABEL, events::COMMAND_MAXIMIZED, now) {
            log::debug!("command:maximized: {err}");
        }
    }
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
            .title("Cay")
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
    MAXIMIZED.store(false, Ordering::Relaxed);
    let watched = window.clone();
    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::Resized(_)) {
            follow_maximized(&watched);
        }
    });
    Ok(window)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_restores_a_maximized_window_before_hiding_it() {
        assert_eq!(escape_action(true), Escape::Restore);
        assert_eq!(escape_action(false), Escape::Hide);
    }
}
