//! Running-apps list: Alt+Tab-style enumeration, icons, and switching.

mod activate;
mod enumerate;
mod icon;

pub use icon::IconCache;

use std::path::Path;

use tauri::{AppHandle, Manager, State};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

use crate::error::{AppError, AppResult};
use crate::hover::Island;
use crate::ipc::WindowInfo;
use crate::{win32, window};

#[tauri::command]
pub async fn list_windows(app: AppHandle) -> AppResult<Vec<WindowInfo>> {
    let island = app
        .get_webview_window(window::LABEL)
        .ok_or(AppError::NoIslandWindow)?;
    let own = win32::raw(island.hwnd()?);

    // Icon lookups can block on hung windows (bounded by SendMessageTimeout), so keep them off async workers.
    tauri::async_runtime::spawn_blocking(move || {
        // SAFETY: SHGetFileInfoW needs COM on this thread; repeat calls on a pooled thread are harmless.
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let cache = app.state::<IconCache>();
        enumerate::alt_tab_windows(win32::from_raw(own))
            .into_iter()
            .map(|w| WindowInfo {
                hwnd: w.hwnd as i64,
                icon: cache.icon_for(win32::from_raw(w.hwnd), w.exe_path.as_deref()),
                exe_name: w.exe_path.as_deref().and_then(file_name),
                title: w.title,
            })
            .collect()
    })
    .await
    .map_err(AppError::Tauri)
}

/// Runs on the main thread (sync command), which has the message queue `AttachThreadInput` needs.
#[tauri::command]
pub fn activate_window(island: State<'_, Island>, hwnd: i64) -> AppResult<()> {
    activate::bring_to_front(win32::from_raw(hwnd as isize))?;
    island.request_collapse();
    Ok(())
}

/// An Alt+Tab window, for other features that need to find one (e.g. a Claude Code session's terminal).
pub struct TopLevel {
    pub hwnd: isize,
    pub pid: u32,
    pub title: String,
}

/// Alt+Tab windows in Z-order (most recently active first), excluding `own`.
pub fn top_level_windows(own: HWND) -> Vec<TopLevel> {
    enumerate::alt_tab_windows(own)
        .into_iter()
        .map(|w| TopLevel {
            pid: win32::window_pid(win32::from_raw(w.hwnd)),
            hwnd: w.hwnd,
            title: w.title,
        })
        .collect()
}

/// The foreground fallback chain from `activate.rs`. Call on the main thread (sync command).
pub fn bring_to_front(hwnd: HWND) -> AppResult<()> {
    activate::bring_to_front(hwnd)
}

fn file_name(path: &str) -> Option<String> {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
}
