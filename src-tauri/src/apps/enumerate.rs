//! Lists top-level windows the way Alt+Tab does.

use windows::core::{BOOL, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumWindows, GetAncestor, GetLastActivePopup, GetWindowLongPtrW,
    GetWindowTextLengthW, GetWindowTextW, IsWindowVisible, GA_ROOTOWNER, GWL_EXSTYLE,
    WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};

use crate::win32;

pub struct Candidate {
    pub hwnd: isize,
    pub title: String,
    pub exe_path: Option<String>,
}

/// In Z-order (most recently active first), excluding `own`.
pub fn alt_tab_windows(own: HWND) -> Vec<Candidate> {
    let mut all: Vec<isize> = Vec::new();
    // SAFETY: the callback only pushes into `all`, which outlives the call.
    let _ = unsafe { EnumWindows(Some(collect), LPARAM(&mut all as *mut Vec<isize> as isize)) };

    all.into_iter()
        .map(win32::from_raw)
        .filter(|&hwnd| hwnd != own && is_alt_tab_window(hwnd))
        .filter_map(|hwnd| {
            Some(Candidate {
                hwnd: win32::raw(hwnd),
                title: window_title(hwnd)?,
                exe_path: exe_path(hwnd),
            })
        })
        .collect()
}

unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: lparam is the `&mut Vec<isize>` passed by `alt_tab_windows`.
    let all = unsafe { &mut *(lparam.0 as *mut Vec<isize>) };
    all.push(win32::raw(hwnd));
    true.into()
}

fn is_alt_tab_window(hwnd: HWND) -> bool {
    // SAFETY: read-only queries; a window that died meanwhile just fails the checks.
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() {
            return false;
        }
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let app_window = ex & WS_EX_APPWINDOW.0 != 0;
        if !app_window && ex & (WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0) != 0 {
            return false;
        }
        if is_cloaked(hwnd) {
            return false;
        }
        if matches!(win32::class_name(hwnd).as_str(), "Progman" | "WorkerW") {
            return false;
        }
        app_window || last_visible_active_popup(hwnd) == hwnd
    }
}

/// Raymond Chen's rule: walk to the root owner, then down its visible last-active-popup chain.
/// Only the window found at the end of that walk represents the group in Alt+Tab.
unsafe fn last_visible_active_popup(hwnd: HWND) -> HWND {
    // SAFETY: both calls accept any HWND.
    unsafe {
        let mut walk = GetAncestor(hwnd, GA_ROOTOWNER);
        for _ in 0..16 {
            let next = GetLastActivePopup(walk);
            if next == walk {
                break;
            }
            walk = next;
            if IsWindowVisible(next).as_bool() {
                break;
            }
        }
        walk
    }
}

/// Cloaked windows: other virtual desktops, suspended UWP frames, etc.
pub(super) fn is_cloaked(hwnd: HWND) -> bool {
    let mut cloaked = 0u32;
    // SAFETY: the out-pointer and size describe `cloaked`.
    let ok = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut _,
            size_of::<u32>() as u32,
        )
    };
    ok.is_ok() && cloaked != 0
}

pub(super) fn window_title(hwnd: HWND) -> Option<String> {
    // SAFETY: buf is sized from the reported length plus the terminator.
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return None;
        }
        let mut buf = vec![0u16; len as usize + 1];
        let copied = GetWindowTextW(hwnd, &mut buf);
        let title = String::from_utf16_lossy(&buf[..copied.max(0) as usize]);
        let title = title.trim();
        (!title.is_empty()).then(|| title.to_owned())
    }
}

/// UWP apps live in a child of an ApplicationFrameHost window; report the child's process instead.
pub(super) fn exe_path(hwnd: HWND) -> Option<String> {
    let pid = win32::window_pid(hwnd);
    let path = process_image_path(pid)?;
    if !path
        .to_ascii_lowercase()
        .ends_with("\\applicationframehost.exe")
    {
        return Some(path);
    }
    let mut search = ChildSearch {
        frame_pid: pid,
        found: None,
    };
    // SAFETY: the callback only writes to `search`, which outlives the call.
    unsafe {
        let _ = EnumChildWindows(
            Some(hwnd),
            Some(find_foreign_child),
            LPARAM(&mut search as *mut ChildSearch as isize),
        );
    }
    search.found.and_then(process_image_path).or(Some(path))
}

struct ChildSearch {
    frame_pid: u32,
    found: Option<u32>,
}

unsafe extern "system" fn find_foreign_child(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: lparam is the `&mut ChildSearch` passed by `exe_path`.
    let search = unsafe { &mut *(lparam.0 as *mut ChildSearch) };
    let pid = win32::window_pid(hwnd);
    if pid != 0 && pid != search.frame_pid {
        search.found = Some(pid);
        return false.into();
    }
    true.into()
}

fn process_image_path(pid: u32) -> Option<String> {
    // SAFETY: the handle is closed before returning; buf/len describe a valid buffer.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let result = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(handle);
        result.ok()?;
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }
}
