//! Which windows Claude may look at: the Alt+Tab list (from `apps/`), the focused window, and the checks every
//! target goes through (alive, not ours, on this desktop, responding, not elevated, screen not locked).

use serde::Serialize;
use windows::Win32::Foundation::{CloseHandle, HWND, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::StationsAndDesktops::{
    CloseDesktop, OpenInputDesktop, DESKTOP_CONTROL_FLAGS, DESKTOP_READOBJECTS,
};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetForegroundWindow, GetWindowRect, IsHungAppWindow, IsIconic, IsWindow, GA_ROOT,
};

use super::logic::{LookError, Rect};
use crate::{apps, win32};

/// A window as Claude sees it in `list_windows` / `focused_window`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowEntry {
    pub hwnd: i64,
    pub app: Option<String>,
    pub title: String,
    /// Physical screen px (the visible frame, without the invisible resize borders).
    pub bounds: Rect,
    pub minimized: bool,
    pub focused: bool,
}

/// A checked target: alive, someone else's, on this desktop.
#[derive(Debug, Clone)]
pub struct Target {
    pub hwnd: isize,
    pub pid: u32,
    pub app: Option<String>,
    pub title: String,
}

fn own_pid() -> u32 {
    std::process::id()
}

/// Alt+Tab windows in Z-order, without any of TopBar's own (island, command window, glow).
pub fn list() -> Vec<WindowEntry> {
    let front = foreground_root();
    apps::top_level_windows(HWND::default())
        .into_iter()
        .filter(|w| w.pid != own_pid())
        .map(|w| {
            let hwnd = win32::from_raw(w.hwnd);
            // SAFETY: read-only query on any HWND.
            let minimized = unsafe { IsIconic(hwnd).as_bool() };
            WindowEntry {
                hwnd: w.hwnd as i64,
                app: w.exe_path.as_deref().and_then(app_name),
                title: w.title,
                // A minimized window is parked at -32000; it has no place on screen.
                bounds: if minimized {
                    Rect::default()
                } else {
                    frame_bounds(hwnd)
                },
                minimized,
                focused: front == Some(w.hwnd),
            }
        })
        .collect()
}

/// The window in front, or `Err(())` when it's one of TopBar's own (never described), or `Ok(None)` when
/// nothing is (the desktop).
pub fn focused() -> Result<Option<WindowEntry>, ()> {
    let Some(front) = foreground_root() else {
        return Ok(None);
    };
    if win32::window_pid(win32::from_raw(front)) == own_pid() {
        return Err(());
    }
    if let Some(entry) = list().into_iter().find(|w| w.hwnd as isize == front) {
        return Ok(Some(entry));
    }
    // In front but not an Alt+Tab window (e.g. the taskbar or a popup): still say what it is.
    let hwnd = win32::from_raw(front);
    let (title, exe) = apps::window_identity(hwnd);
    Ok(Some(WindowEntry {
        hwnd: front as i64,
        app: exe.as_deref().and_then(app_name),
        title: title.unwrap_or_default(),
        bounds: frame_bounds(hwnd),
        minimized: false,
        focused: true,
    }))
}

/// A target's name for approvals: "Chrome · GitHub". `None` if it's gone or ours.
pub fn identify(hwnd: i64) -> Option<Target> {
    let h = win32::from_raw(hwnd as isize);
    // SAFETY: read-only query on any HWND.
    if !unsafe { IsWindow(Some(h)).as_bool() } {
        return None;
    }
    let pid = win32::window_pid(h);
    if pid == own_pid() {
        return None;
    }
    let (title, exe) = apps::window_identity(h);
    Some(Target {
        hwnd: hwnd as isize,
        pid,
        app: exe.as_deref().and_then(app_name),
        title: title.unwrap_or_default(),
    })
}

/// Every check before a look at `hwnd`. `expect_pid` is the process seen at approval time: an HWND reused by
/// another window in between is refused as gone.
pub fn check(hwnd: i64, expect_pid: Option<u32>) -> Result<Target, LookError> {
    if screen_locked() {
        return Err(LookError::ScreenLocked);
    }
    let h = win32::from_raw(hwnd as isize);
    // SAFETY: read-only queries on any HWND.
    unsafe {
        if hwnd == 0 || !IsWindow(Some(h)).as_bool() {
            return Err(LookError::WindowGone);
        }
        let root = GetAncestor(h, GA_ROOT);
        if !root.is_invalid() && root != h {
            // A child window: look at its top-level window instead.
            return check(win32::raw(root) as i64, expect_pid);
        }
    }
    let target = identify(hwnd).ok_or(LookError::TopbarWindow)?;
    if expect_pid.is_some_and(|pid| pid != target.pid) {
        return Err(LookError::WindowGone);
    }
    if apps::is_cloaked(h) {
        return Err(LookError::NotOnThisDesktop);
    }
    // SAFETY: read-only query.
    if unsafe { IsHungAppWindow(h).as_bool() } {
        return Err(LookError::NotResponding);
    }
    if is_elevated(target.pid) && !is_elevated_self() {
        return Err(LookError::Elevated);
    }
    Ok(target)
}

pub fn is_minimized(hwnd: isize) -> bool {
    // SAFETY: read-only query.
    unsafe { IsIconic(win32::from_raw(hwnd)).as_bool() }
}

/// The lock screen and UAC prompts run on the secure desktop, which a normal process can't open.
pub fn screen_locked() -> bool {
    // SAFETY: the desktop handle is closed at once.
    unsafe {
        match OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_READOBJECTS) {
            Ok(desk) => {
                let _ = CloseDesktop(desk);
                false
            }
            Err(_) => true,
        }
    }
}

/// How long since your last mouse or keyboard input (TopBar's own injected input counts too).
pub fn idle_ms() -> u64 {
    let mut info = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    // SAFETY: fills our struct; `cbSize` is set.
    if !unsafe { GetLastInputInfo(&mut info) }.as_bool() {
        return 0;
    }
    // SAFETY: plain query. `dwTime` is the low 32 bits of the tick count, so compare in u32.
    let now = unsafe { GetTickCount() };
    u64::from(now.wrapping_sub(info.dwTime))
}

/// The visible frame (DWM's extended frame bounds), falling back to the window rect.
pub fn frame_bounds(hwnd: HWND) -> Rect {
    let mut r = RECT::default();
    // SAFETY: the out-pointer and size describe `r`.
    let dwm = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut r as *mut RECT as *mut _,
            size_of::<RECT>() as u32,
        )
    };
    // SAFETY: as above.
    if dwm.is_err() && unsafe { GetWindowRect(hwnd, &mut r) }.is_err() {
        return Rect::default();
    }
    Rect::from_ltrb(r.left, r.top, r.right, r.bottom)
}

/// The window rect including invisible borders (what `PrintWindow` draws).
pub fn window_rect(hwnd: HWND) -> Rect {
    let mut r = RECT::default();
    // SAFETY: `r` outlives the call.
    let _ = unsafe { GetWindowRect(hwnd, &mut r) };
    Rect::from_ltrb(r.left, r.top, r.right, r.bottom)
}

/// The monitor a window is on (nearest), or the primary one for `None`, in physical px.
pub fn monitor_rect(hwnd: Option<isize>) -> Rect {
    // SAFETY: MonitorFromWindow accepts any HWND; `info` is sized for GetMonitorInfoW.
    unsafe {
        let monitor = match hwnd {
            Some(h) => MonitorFromWindow(win32::from_raw(h), MONITOR_DEFAULTTONEAREST),
            None => MonitorFromWindow(HWND::default(), MONITOR_DEFAULTTOPRIMARY),
        };
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return Rect::default();
        }
        let r = info.rcMonitor;
        Rect::from_ltrb(r.left, r.top, r.right, r.bottom)
    }
}

fn foreground_root() -> Option<isize> {
    // SAFETY: read-only queries.
    unsafe {
        let front = GetForegroundWindow();
        if front.is_invalid() {
            return None;
        }
        let root = GetAncestor(front, GA_ROOT);
        Some(win32::raw(if root.is_invalid() { front } else { root }))
    }
}

/// `C:\…\chrome.exe` → `chrome`.
fn app_name(path: &str) -> Option<String> {
    let file = apps::exe_file_name(path)?;
    Some(
        file.strip_suffix(".exe")
            .or_else(|| file.strip_suffix(".EXE"))
            .unwrap_or(&file)
            .to_string(),
    )
}

fn is_elevated_self() -> bool {
    // SAFETY: the pseudo-handle needs no closing.
    token_elevated(unsafe { GetCurrentProcess() }).unwrap_or(false)
}

/// Elevated, or a token we may not even read (which, from a normal process, means elevated or protected).
fn is_elevated(pid: u32) -> bool {
    // SAFETY: the process handle is closed before returning.
    unsafe {
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return true;
        };
        let elevated = token_elevated(process).unwrap_or(true);
        let _ = CloseHandle(process);
        elevated
    }
}

fn token_elevated(process: windows::Win32::Foundation::HANDLE) -> Option<bool> {
    // SAFETY: the token is closed by the guard; `elevation`/`len` describe a valid buffer.
    unsafe {
        let mut token = Default::default();
        OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;
        let token = win32::Handle(token);
        let mut elevation = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        GetTokenInformation(
            token.0,
            TokenElevation,
            Some(&mut elevation as *mut TOKEN_ELEVATION as *mut _),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
        .ok()?;
        Some(elevation.TokenIsElevated != 0)
    }
}
