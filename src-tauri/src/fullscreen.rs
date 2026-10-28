//! Detects a fullscreen app in the foreground on the primary monitor (where the island lives).

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONULL,
};
use windows::Win32::UI::Shell::{
    SHQueryUserNotificationState, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetShellWindow, GetWindowRect, IsZoomed, MONITORINFOF_PRIMARY,
};

use crate::win32;

pub fn foreground_is_fullscreen(own: HWND) -> bool {
    // SAFETY: no preconditions; reports exclusive D3D fullscreen and presentation mode.
    if let Ok(state) = unsafe { SHQueryUserNotificationState() } {
        if state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_PRESENTATION_MODE {
            return true;
        }
    }

    // SAFETY: plain queries on whatever window is in the foreground; all tolerate stale handles.
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_invalid() || fg == own || fg == GetShellWindow() {
            return false;
        }
        if matches!(win32::class_name(fg).as_str(), "Progman" | "WorkerW") {
            return false;
        }
        // A maximized window with an auto-hidden taskbar can cover the monitor too; that isn't fullscreen.
        if IsZoomed(fg).as_bool() {
            return false;
        }

        let mut rect = RECT::default();
        if GetWindowRect(fg, &mut rect).is_err() {
            return false;
        }
        let monitor = MonitorFromWindow(fg, MONITOR_DEFAULTTONULL);
        if monitor.is_invalid() {
            return false;
        }
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool()
            || info.dwFlags & MONITORINFOF_PRIMARY == 0
        {
            return false;
        }

        let m = info.rcMonitor;
        rect.left <= m.left && rect.top <= m.top && rect.right >= m.right && rect.bottom >= m.bottom
    }
}
