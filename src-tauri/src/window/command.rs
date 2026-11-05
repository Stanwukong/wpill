//! The `command` window: where you type an agent instruction and read its transcript. Unlike the island it's a
//! normal, focusable window (CLAUDE.md rule 2), created on first use and hidden (not closed) afterwards.
//!
//! Call `show` from a background thread or an async command, never from a sync command or the main thread:
//! building a webview window there deadlocks on Windows. The hotkey thread's `WM_HOTKEY` gives Cay the
//! right to take the foreground, which is what lets `set_focus` work.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
    WindowEvent,
};

use crate::agent::settings::{self as agent_settings, WindowPlace};

use crate::error::{AppError, AppResult};
use crate::ipc::{events, CommandFocus};

/// What a window that was just created should select: its page isn't listening yet when `show` emits, so it takes
/// this once loaded (`command_take_focus`).
static PENDING: Mutex<Option<CommandFocus>> = Mutex::new(None);

/// Whether the window is maximized, as last emitted (`command:maximized`), so a resize only emits a change.
static MAXIMIZED: AtomicBool = AtomicBool::new(false);

/// The window's last restored (not maximized, not minimized) rect, kept from its moves and resizes so it can be
/// saved while it's maximized (2.10c.2).
static NORMAL: Mutex<Option<Rect>> = Mutex::new(None);

/// A just-created window was maximized when it was last hidden: maximize it once it's shown.
static MAXIMIZE_ON_SHOW: AtomicBool = AtomicBool::new(false);

pub const LABEL: &str = "command";
/// How much of the saved title bar must lie on a monitor's work area for the place to be used (physical px).
const MIN_VISIBLE_BAR: i64 = 120;
/// The title bar's height in CSS px (`.cmd__bar`), the strip that must stay reachable.
const BAR_HEIGHT: f64 = 44.0;
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
    if MAXIMIZE_ON_SHOW.swap(false, Ordering::Relaxed) {
        window.maximize()?;
    }
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
        remember(app, &window);
        window.hide()?;
    }
    Ok(())
}

/// Saves where the window is (`agent.window`) as it hides: its restored rect and whether it's maximized.
fn remember(app: &AppHandle, window: &WebviewWindow) {
    let Some(rect) = *NORMAL.lock().unwrap_or_else(|e| e.into_inner()) else {
        return;
    };
    agent_settings::save_window(
        app,
        WindowPlace {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
            maximized: window.is_maximized().unwrap_or(false),
        },
    );
}

/// Keeps `NORMAL` current while the window is neither maximized nor minimized.
fn follow_normal(window: &WebviewWindow) {
    let restored = !window.is_maximized().unwrap_or(true) && !window.is_minimized().unwrap_or(true);
    if !restored {
        return;
    }
    if let (Ok(pos), Ok(size)) = (window.outer_position(), window.inner_size()) {
        *NORMAL.lock().unwrap_or_else(|e| e.into_inner()) = Some(Rect {
            x: pos.x,
            y: pos.y,
            width: size.width,
            height: size.height,
        });
    }
}

/// A rect in physical px: a window's place, or a monitor's work area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Where a saved window may go (D31: never off-screen): onto the work area holding most of its title bar (the top
/// `bar` px), shrunk to fit and moved fully inside it. `None` when no current work area holds at least
/// `MIN_VISIBLE_BAR` px of the bar (the monitor is gone or moved), so the window opens centred instead.
pub fn fit(saved: Rect, bar: u32, areas: &[Rect]) -> Option<Rect> {
    let overlap = |area: &Rect| -> i64 {
        let left = i64::from(saved.x).max(i64::from(area.x));
        let right = (i64::from(saved.x) + i64::from(saved.width))
            .min(i64::from(area.x) + i64::from(area.width));
        let top = i64::from(saved.y).max(i64::from(area.y));
        let bottom = (i64::from(saved.y) + i64::from(bar.min(saved.height)))
            .min(i64::from(area.y) + i64::from(area.height));
        if right <= left || bottom <= top {
            0
        } else {
            right - left
        }
    };
    let (_, area) = areas
        .iter()
        .map(|area| (overlap(area), area))
        .filter(|(seen, _)| *seen >= MIN_VISIBLE_BAR)
        .max_by_key(|(seen, _)| *seen)?;
    let width = saved.width.min(area.width);
    let height = saved.height.min(area.height);
    let max_x = i64::from(area.x) + i64::from(area.width - width);
    let max_y = i64::from(area.y) + i64::from(area.height - height);
    Some(Rect {
        x: i64::from(saved.x).clamp(i64::from(area.x), max_x) as i32,
        y: i64::from(saved.y).clamp(i64::from(area.y), max_y) as i32,
        width,
        height,
    })
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
        Escape::Hide => hide(app)?,
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
    MAXIMIZED.store(false, Ordering::Relaxed);
    MAXIMIZE_ON_SHOW.store(false, Ordering::Relaxed);
    if !restore(app, &window) {
        // Centered below the island, a little down from the top of the primary monitor.
        let monitor = window.primary_monitor()?.ok_or(AppError::NoMonitor)?;
        let size = window.outer_size()?;
        let origin = monitor.position();
        let x = origin.x + (monitor.size().width as i32 - size.width as i32) / 2;
        let y = origin.y + (monitor.size().height as f64 * 0.12) as i32;
        window.set_position(PhysicalPosition::new(x, y))?;
    }
    follow_normal(&window);
    let watched = window.clone();
    window.on_window_event(move |event| match event {
        WindowEvent::Resized(_) => {
            follow_maximized(&watched);
            follow_normal(&watched);
        }
        WindowEvent::Moved(_) => follow_normal(&watched),
        _ => {}
    });
    Ok(window)
}

/// Puts a new window where it was last (`agent.window`) if that's still on a monitor; `false` leaves it to centre.
fn restore(app: &AppHandle, window: &WebviewWindow) -> bool {
    let Some(place) = agent_settings::load_window(app) else {
        return false;
    };
    let areas: Vec<Rect> = window
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|monitor| {
            let area = monitor.work_area();
            Rect {
                x: area.position.x,
                y: area.position.y,
                width: area.size.width,
                height: area.size.height,
            }
        })
        .collect();
    let saved = Rect {
        x: place.x,
        y: place.y,
        width: place.width,
        height: place.height,
    };
    let bar = (BAR_HEIGHT * window.scale_factor().unwrap_or(1.0) * text_scale()) as u32;
    let Some(rect) = fit(saved, bar, &areas) else {
        log::debug!("command window: its saved place is off every monitor; centring");
        return false;
    };
    let placed = window
        .set_size(PhysicalSize::new(rect.width, rect.height))
        .and_then(|()| window.set_position(PhysicalPosition::new(rect.x, rect.y)));
    if let Err(err) = placed {
        log::debug!("command window: restoring its place failed: {err}");
        return false;
    }
    MAXIMIZE_ON_SHOW.store(place.maximized, Ordering::Relaxed);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_restores_a_maximized_window_before_hiding_it() {
        assert_eq!(escape_action(true), Escape::Restore);
        assert_eq!(escape_action(false), Escape::Hide);
    }

    const PRIMARY: Rect = Rect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1032,
    };
    /// A second monitor to the left, its taskbar at the top.
    const LEFT: Rect = Rect {
        x: -1280,
        y: 40,
        width: 1280,
        height: 984,
    };

    fn rect(x: i32, y: i32, width: u32, height: u32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn a_place_on_a_monitor_is_kept() {
        let saved = rect(300, 120, 900, 700);
        assert_eq!(fit(saved, 44, &[PRIMARY, LEFT]), Some(saved));
        let left = rect(-1100, 200, 800, 600);
        assert_eq!(fit(left, 44, &[PRIMARY, LEFT]), Some(left));
    }

    #[test]
    fn a_place_on_a_gone_monitor_centres() {
        assert_eq!(fit(rect(-1100, 200, 800, 600), 44, &[PRIMARY]), None);
        assert_eq!(fit(rect(2500, 100, 800, 600), 44, &[PRIMARY]), None);
        // Only a sliver of the title bar would show: centre instead.
        assert_eq!(fit(rect(1850, 100, 800, 600), 44, &[PRIMARY]), None);
        // The bar above the work area (behind a top taskbar, or off the top).
        assert_eq!(fit(rect(-1000, -100, 800, 600), 44, &[LEFT]), None);
        assert_eq!(fit(rect(100, 100, 800, 600), 44, &[]), None);
    }

    #[test]
    fn a_place_half_off_is_moved_and_shrunk_inside() {
        // Hanging off the right and bottom: moved in.
        assert_eq!(
            fit(rect(1500, 700, 800, 600), 44, &[PRIMARY]),
            Some(rect(1120, 432, 800, 600))
        );
        // Bigger than the work area now (a smaller monitor): shrunk to it.
        assert_eq!(
            fit(rect(-1200, 60, 1600, 1200), 44, &[PRIMARY, LEFT]),
            Some(rect(-1280, 40, 1280, 984))
        );
    }

    #[test]
    fn a_place_across_two_monitors_goes_to_the_one_with_more_bar() {
        // 200 px of the bar on the left monitor, 600 on the primary.
        assert_eq!(
            fit(rect(-200, 100, 800, 600), 44, &[PRIMARY, LEFT]),
            Some(rect(0, 100, 800, 600))
        );
    }
}
