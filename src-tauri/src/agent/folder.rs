//! The command window's "Browse…": the shell's folder picker, owned by the command window. Called from a sync
//! command, so it runs on the main thread (COM STA, like the screenshot drag).

use windows::core::{Result, HSTRING};
use windows::Win32::Foundation::HWND;
use windows::Win32::Storage::FileSystem::GetLongPathNameW;
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::Shell::{
    FileOpenDialog, IFileOpenDialog, FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS,
    SIGDN_FILESYSPATH,
};

/// The chosen folder, or `None` if the dialog was cancelled.
pub fn pick(owner: HWND) -> Result<Option<String>> {
    // SAFETY: COM is initialized on the main thread (Tao/OLE); the dialog and item are released on drop, and
    // the path string is freed with CoTaskMemFree.
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
        let options = dialog.GetOptions()?;
        dialog.SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST)?;
        if dialog.Show(Some(owner)).is_err() {
            return Ok(None);
        }
        let item = dialog.GetResult()?;
        let path = item.GetDisplayName(SIGDN_FILESYSPATH)?;
        let text = path.to_string().ok();
        CoTaskMemFree(Some(path.0 as *const _));
        Ok(text)
    }
}

/// `C:\Users\DELL73~1\…` → `C:\Users\DELL 7300\…`. Claude Code compares paths with its working folder as text,
/// so a short (8.3) folder made it ask before reading files inside it. Unchanged if it can't be resolved.
pub fn long_path(path: &str) -> String {
    let wide = HSTRING::from(path);
    let mut buf = vec![0u16; 1024];
    // SAFETY: `buf` is writable for its length; the call returns the length written (0 on failure).
    let n = unsafe { GetLongPathNameW(&wide, Some(&mut buf)) } as usize;
    if n == 0 || n > buf.len() {
        return path.to_string();
    }
    String::from_utf16_lossy(&buf[..n])
}
