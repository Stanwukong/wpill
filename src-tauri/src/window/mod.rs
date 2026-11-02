//! Creates and positions the island window and owns its Win32 styles.

pub mod command;
pub mod glow;

use tauri::{
    AppHandle, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, GetWindowRect, IsWindowVisible, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    GWL_EXSTYLE, HWND_TOPMOST, SWP_ASYNCWINDOWPOS, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_SHOWNOACTIVATE, WS_EX_APPWINDOW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
};

use crate::error::{AppError, AppResult};

pub const LABEL: &str = "island";

/// Window size in logical px: the largest panel plus room for its shadow. Taller than the panel usually is (600, was
/// 440 until 6b): the window is transparent and click-through outside the reported panel rect, so the room is free.
pub const WINDOW_WIDTH: f64 = 680.0;
pub const WINDOW_HEIGHT: f64 = 600.0;
/// Gap between the monitor's top edge and the island. Mirrors `PANEL_TOP_PX` in `src/island/motion.ts`.
pub const PANEL_TOP: f64 = 6.0;

/// The island's monitor (the primary one) in physical px, as the hit tests see it. Re-read by the poller every
/// 250 ms (Phase 4a-2), so docking, resolution and scale changes move the island without a restart.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometry {
    pub monitor_top: i32,
    pub center_x: i32,
    pub scale: f64,
}

/// A monitor's rect in physical px.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Area {
    pub left: i32,
    pub top: i32,
    pub width: i32,
}

/// Where the island window goes, in physical px.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Top centre of `area`, sized to the largest panel (D2) at `scale`. Pure.
pub fn place(area: Area, scale: f64) -> (Geometry, Placement) {
    let width = (WINDOW_WIDTH * scale).round() as i32;
    let height = (WINDOW_HEIGHT * scale).round() as i32;
    let center_x = area.left + area.width / 2;
    (
        Geometry {
            monitor_top: area.top,
            center_x,
            scale,
        },
        Placement {
            x: center_x - width / 2,
            y: area.top,
            width,
            height,
        },
    )
}

/// The primary monitor and its DPI scale (1.0 at 96 DPI; Text size isn't part of it, see D7).
fn primary_area() -> Option<(Area, f64)> {
    // SAFETY: MonitorFromPoint accepts any point; `info` is sized for GetMonitorInfoW; the DPI out-params outlive
    // the call.
    unsafe {
        let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return None;
        }
        let (mut dpi_x, mut dpi_y) = (0, 0);
        GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y).ok()?;
        let r = info.rcMonitor;
        let area = Area {
            left: r.left,
            top: r.top,
            width: r.right - r.left,
        };
        (area.width > 0 && dpi_x > 0).then_some((area, dpi_x as f64 / 96.0))
    }
}

/// Where the island belongs right now.
pub fn current() -> Option<(Geometry, Placement)> {
    primary_area().map(|(area, scale)| place(area, scale))
}

/// Moves the island to `to` unless it's already there. Never activates it or changes its z-order, and doesn't
/// wait for the main thread (`SWP_ASYNCWINDOWPOS`), since the poller calls it. Returns whether it moved.
pub fn keep_at(hwnd: HWND, to: Placement) -> bool {
    let mut rect = RECT::default();
    // SAFETY: hwnd is our own window; `rect` outlives the call.
    unsafe {
        if GetWindowRect(hwnd, &mut rect).is_ok()
            && rect.left == to.x
            && rect.top == to.y
            && rect.right - rect.left == to.width
            && rect.bottom - rect.top == to.height
        {
            return false;
        }
        let _ = SetWindowPos(
            hwnd,
            None,
            to.x,
            to.y,
            to.width,
            to.height,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
        );
    }
    true
}

pub fn create(app: &AppHandle) -> AppResult<(WebviewWindow, Geometry)> {
    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::default())
        .title("Cay")
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

    let (geometry, at) = current().ok_or(AppError::NoMonitor)?;
    window.set_size(PhysicalSize::new(at.width as u32, at.height as u32))?;
    window.set_position(PhysicalPosition::new(at.x, at.y))?;

    let hwnd = window.hwnd()?;
    set_click_through(hwnd, true);
    show_no_activate(hwnd);
    log::info!(
        "island window {}x{} at x={} (scale {})",
        at.width,
        at.height,
        at.x,
        geometry.scale
    );

    Ok((window, geometry))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_top_centre_at_100_percent() {
        let area = Area {
            left: 0,
            top: 0,
            width: 1920,
        };
        let (geo, at) = place(area, 1.0);
        assert_eq!(
            geo,
            Geometry {
                monitor_top: 0,
                center_x: 960,
                scale: 1.0
            }
        );
        assert_eq!(
            at,
            Placement {
                x: 620,
                y: 0,
                width: 680,
                height: 600
            }
        );
    }

    #[test]
    fn scales_and_follows_an_offset_monitor() {
        // A primary monitor that isn't at the origin of the virtual screen (or a future non-primary choice).
        let area = Area {
            left: -2560,
            top: -200,
            width: 2560,
        };
        let (geo, at) = place(area, 1.25);
        assert_eq!(geo.center_x, -1280);
        assert_eq!(geo.monitor_top, -200);
        assert_eq!(
            at,
            Placement {
                x: -1280 - 425,
                y: -200,
                width: 850,
                height: 750
            }
        );
    }

    #[test]
    fn a_scale_change_alone_changes_the_geometry() {
        let area = Area {
            left: 0,
            top: 0,
            width: 1920,
        };
        assert_ne!(place(area, 1.0).0, place(area, 1.5).0);
        assert_eq!(place(area, 1.5).1.width, 1020);
    }
}
