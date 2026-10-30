//! Dragging a file out of the island. Runs on the main thread (OLE-initialized STA, owner of the island
//! window) while the mouse button is still down from the webview's pointer event.
//!
//! The island never activates (`WS_EX_NOACTIVATE`); drop targets don't care whether the source is active.
//! `SHDoDragDrop` supplies the shell's default drop source (and drag image), and the data object comes from
//! the file's shell item, so Explorer, mail and chat apps all accept it like a drag from Explorer.

use std::path::Path;

use windows::core::{Result, HSTRING};
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::System::Com::IDataObject;
use windows::Win32::System::Ole::{IDropSource, DROPEFFECT_COPY, DROPEFFECT_NONE};
use windows::Win32::UI::Shell::{
    BHID_DataObject, IShellItem, SHCreateItemFromParsingName, SHDoDragDrop,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetCursorPos, WindowFromPoint, GA_ROOT,
};

/// Blocks until the drop or cancel. Returns whether the file was dropped somewhere other than the island.
pub fn drag_file(hwnd: HWND, path: &Path) -> Result<bool> {
    // SAFETY: called on the window's (STA) thread; the COM objects are released on drop.
    let effect = unsafe {
        let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None)?;
        let data: IDataObject = item.BindToHandler(None, &BHID_DataObject)?;
        SHDoDragDrop(Some(hwnd), &data, None::<&IDropSource>, DROPEFFECT_COPY)?
    };
    // Tauri's own file-drop handler accepts drops on the island; that isn't sharing the file.
    Ok(effect != DROPEFFECT_NONE && !cursor_over(hwnd))
}

fn cursor_over(hwnd: HWND) -> bool {
    let mut cursor = POINT::default();
    // SAFETY: read-only window queries; cursor is a valid out-pointer.
    unsafe {
        GetCursorPos(&mut cursor).is_ok() && GetAncestor(WindowFromPoint(cursor), GA_ROOT) == hwnd
    }
}
