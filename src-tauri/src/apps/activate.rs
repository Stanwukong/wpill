//! Brings another app's window to the foreground despite the foreground-lock rules.

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_MOUSE};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, IsIconic, IsWindow,
    SetForegroundWindow, ShowWindow, SW_RESTORE,
};

use crate::error::{AppError, AppResult};

pub fn bring_to_front(hwnd: HWND) -> AppResult<()> {
    // SAFETY: hwnd is validated with IsWindow first; the rest tolerate a window dying mid-way.
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() {
            return Err(AppError::WindowGone);
        }
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        if try_set_foreground(hwnd) {
            return Ok(());
        }

        // Only the process that received the last input event may change the foreground window.
        // An empty synthetic mouse event makes us that process (the same trick PowerToys uses).
        let input = INPUT {
            r#type: INPUT_MOUSE,
            ..Default::default()
        };
        SendInput(&[input], size_of::<INPUT>() as i32);
        if try_set_foreground(hwnd) {
            return Ok(());
        }

        // Last resort: share input state with the current foreground thread.
        let me = GetCurrentThreadId();
        let fg_thread = GetWindowThreadProcessId(GetForegroundWindow(), None);
        if fg_thread != 0 && fg_thread != me && AttachThreadInput(me, fg_thread, true).as_bool() {
            let _ = BringWindowToTop(hwnd);
            let ok = try_set_foreground(hwnd);
            let _ = AttachThreadInput(me, fg_thread, false);
            if ok {
                return Ok(());
            }
        }

        if !IsWindow(Some(hwnd)).as_bool() {
            return Err(AppError::WindowGone);
        }
        Err(AppError::ForegroundDenied)
    }
}

unsafe fn try_set_foreground(hwnd: HWND) -> bool {
    // SAFETY: both calls accept any HWND.
    unsafe { SetForegroundWindow(hwnd).as_bool() && GetForegroundWindow() == hwnd }
}
