//! An exe's display name: its version resource's `FileDescription` ("Google Chrome"), as Task Manager shows it.

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
};

/// `None` when the file has no version resource or no description.
pub fn file_description(path: &str) -> Option<String> {
    let wide = HSTRING::from(path);
    // SAFETY: `wide` is a NUL-terminated path; the out-handle may be null.
    let size = unsafe { GetFileVersionInfoSizeW(PCWSTR(wide.as_ptr()), None) };
    if size == 0 || size > (1 << 20) {
        return None;
    }
    let mut data = vec![0u8; size as usize];
    // SAFETY: `data` is a writable buffer of `size` bytes.
    unsafe { GetFileVersionInfoW(PCWSTR(wide.as_ptr()), None, size, data.as_mut_ptr().cast()) }
        .ok()?;

    // The first translation (language, code page) names the string table to read.
    let (lang, codepage) = {
        let (ptr, len) = query(&data, r"\VarFileInfo\Translation")?;
        if len < 4 {
            return None;
        }
        // SAFETY: VerQueryValueW returned at least 4 bytes inside `data`.
        let words = unsafe { std::slice::from_raw_parts(ptr.cast::<u16>(), 2) };
        (words[0], words[1])
    };
    let key = format!(r"\StringFileInfo\{lang:04x}{codepage:04x}\FileDescription");
    let (ptr, len) = query(&data, &key)?;
    // SAFETY: for string values `len` counts UTF-16 units (including the NUL) inside `data`.
    let units = unsafe { std::slice::from_raw_parts(ptr.cast::<u16>(), len as usize) };
    let text = String::from_utf16_lossy(units);
    let text = text.trim_end_matches('\0').trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn query(data: &[u8], key: &str) -> Option<(*const core::ffi::c_void, u32)> {
    let sub = HSTRING::from(key);
    let mut ptr = std::ptr::null_mut();
    let mut len = 0u32;
    // SAFETY: `data` holds a version resource from GetFileVersionInfoW; ptr/len are valid out-pointers, and the
    // returned pointer stays inside `data`, which outlives every use.
    let ok = unsafe {
        VerQueryValueW(
            data.as_ptr().cast(),
            PCWSTR(sub.as_ptr()),
            &mut ptr,
            &mut len,
        )
    };
    (ok.as_bool() && !ptr.is_null()).then_some((ptr as *const _, len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notepad_has_a_description() {
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
        let name = file_description(&format!(r"{windir}\System32\notepad.exe"));
        assert!(name.is_some_and(|n| !n.is_empty()));
        assert_eq!(file_description(r"C:\definitely\not\here.exe"), None);
    }
}
