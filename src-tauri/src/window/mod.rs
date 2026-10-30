//! Creates and positions the island window and owns its Win32 styles.

pub mod command;

use tauri::{
    AppHandle, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, IsWindowVisible, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWL_EXSTYLE,
    HWND_TOPMOST, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE,
    SW_SHOWNOACTIVATE, WS_EX_APPWINDOW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TRANSPARENT,
};

use crate::error::{AppError, AppResult};

pub const LABEL: &str = "island";

/// Window size in logical px: the largest panel plus room for its shadow.
pub const WINDOW_WIDTH: f64 = 680.0;
pub const WINDOW_HEIGHT: f64 = 440.0;
/// Gap between the monitor's top edge and the island. Mirrors `PANEL_TOP_PX` in `src/island/motion.ts`.
pub const PANEL_TOP: f64 = 6.0;

/// Primary-monitor geometry in physical px, captured at startup.
/// Phase 4 (multi-monitor/DPI) will refresh it on display changes.
#[derive(Debug, Clone, Copy)]
pub struct Geometry {
    pub monitor_top: i32,
    pub center_x: i32,
    pub scale: f64,
}

pub fn create(app: &AppHandle) -> AppResult<(WebviewWindow, Geometry)> {
    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::default())
        .title("TopBar")
        .inner_size(WINDOW_WIDTH, WINDOW_HEIGHT)
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focusable(false)
        .focused(false)
        .visible(false)
        .build()?;

    let monitor = window.primary_monitor()?.ok_or(AppError::NoMonitor)?;
    let scale = monitor.scale_factor();
    let origin = *monitor.position();
    let size = *monitor.size();
    let width = (WINDOW_WIDTH * scale).round() as i32;
    let height = (WINDOW_HEIGHT * scale).round() as i32;
    let center_x = origin.x + size.width as i32 / 2;
    window.set_size(PhysicalSize::new(width as u32, height as u32))?;
    window.set_position(PhysicalPosition::new(center_x - width / 2, origin.y))?;

    let hwnd = window.hwnd()?;
    set_click_through(hwnd, true);
    show_no_activate(hwnd);
    log::info!(
        "island window {width}x{height} at x={} (scale {scale})",
        center_x - width / 2
    );

    Ok((
        window,
        Geometry {
            monitor_top: origin.y,
            center_x,
            scale,
        },
    ))
}

/// Sets the island's extended style: never activate, never in the taskbar or Alt+Tab, always
/// layered, and click-through (`WS_EX_TRANSPARENT`) per `click_through`. Idempotent and cheap.
///
/// We don't use Tauri's `set_ignore_cursor_events`: it toggles `WS_EX_LAYERED` too, and DWM
/// drops the window's surface for ~150 ms each time that changes, so the island flashes.
/// Keeping it layered permanently makes click-through a single bit flip with no repaint.
pub fn set_click_through(hwnd: HWND, click_through: bool) {
    let always = (WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0 | WS_EX_LAYERED.0) as isize;
    let transparent = WS_EX_TRANSPARENT.0 as isize;
    // SAFETY: hwnd is our own window; these calls have no other preconditions.
    unsafe {
        let current = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let mut next = (current | always) & !(WS_EX_APPWINDOW.0 as isize);
        if click_through {
            next |= transparent;
        } else {
            next &= !transparent;
        }
        if next == current {
            return;
        }
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, next);
        // Only the tool-window bit needs a frame refresh; forcing one on every toggle repaints.
        if (next ^ current) & WS_EX_TOOLWINDOW.0 as isize != 0 {
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
    }
}

/// Shows the window without activating it and re-asserts topmost.
/// (Tauri's `show()` uses `SW_SHOW`, which would steal focus.)
pub fn show_no_activate(hwnd: HWND) {
    // SAFETY: hwnd is our own window.
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

pub fn is_visible(hwnd: HWND) -> bool {
    // SAFETY: read-only query.
    unsafe { IsWindowVisible(hwnd).as_bool() }
}

pub fn hide(hwnd: HWND) {
    // SAFETY: hwnd is our own window.
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}
