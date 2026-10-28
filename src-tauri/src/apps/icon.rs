//! Window/exe icons rendered to PNG data URLs, with a small cache.

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Mutex;

use base64::Engine;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GdiFlush, SelectObject,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
};
use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
use windows::Win32::UI::Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON};
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, DrawIconEx, GetClassLongPtrW, SendMessageTimeoutW, DI_MASK, DI_NORMAL, GCLP_HICON,
    GCLP_HICONSM, HICON, ICON_BIG, ICON_SMALL, ICON_SMALL2, SMTO_ABORTIFHUNG, WM_GETICON,
};

/// Rendered size in px; the UI shows icons at 24 logical px, so this stays crisp up to 200% scaling.
const ICON_PX: i32 = 48;
const CACHE_LIMIT: usize = 512;
const GETICON_TIMEOUT_MS: u32 = 50;

/// Keyed by HICON value (window icons) or exe path (fallback icons).
#[derive(Default)]
pub struct IconCache(Mutex<HashMap<String, Option<String>>>);

impl IconCache {
    pub fn icon_for(&self, hwnd: HWND, exe_path: Option<&str>) -> Option<String> {
        if let Some(icon) = window_icon(hwnd) {
            return self.get_or_insert(format!("h{}", icon.0 as isize), || to_data_url(icon));
        }
        let path = exe_path?;
        self.get_or_insert(format!("p{path}"), || exe_icon(path))
    }

    fn get_or_insert(&self, key: String, make: impl FnOnce() -> Option<String>) -> Option<String> {
        if let Some(hit) = self.0.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
            return hit.clone();
        }
        let value = make();
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() >= CACHE_LIMIT {
            map.clear();
        }
        map.insert(key, value.clone());
        value
    }
}

/// Icons the window owns; the caller must not destroy them.
fn window_icon(hwnd: HWND) -> Option<HICON> {
    for kind in [ICON_BIG, ICON_SMALL2, ICON_SMALL] {
        let mut result = 0usize;
        // SAFETY: result is a valid out-pointer; the timeout guards against hung windows.
        let ok = unsafe {
            SendMessageTimeoutW(
                hwnd,
                WM_GETICON,
                WPARAM(kind as usize),
                LPARAM(0),
                SMTO_ABORTIFHUNG,
                GETICON_TIMEOUT_MS,
                Some(&mut result),
            )
        };
        if ok.0 != 0 && result != 0 {
            return Some(HICON(result as *mut c_void));
        }
    }
    for index in [GCLP_HICON, GCLP_HICONSM] {
        // SAFETY: read-only class query.
        let value = unsafe { GetClassLongPtrW(hwnd, index) };
        if value != 0 {
            return Some(HICON(value as *mut c_void));
        }
    }
    None
}

fn exe_icon(path: &str) -> Option<String> {
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let mut info = SHFILEINFOW::default();
    // SAFETY: wide is NUL-terminated; info/size describe a valid struct. We own and destroy the returned icon.
    unsafe {
        let ok = SHGetFileInfoW(
            PCWSTR(wide.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut info),
            size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        );
        if ok == 0 || info.hIcon.is_invalid() {
            return None;
        }
        let url = to_data_url(info.hIcon);
        let _ = DestroyIcon(info.hIcon);
        url
    }
}

fn to_data_url(icon: HICON) -> Option<String> {
    let rgba = render_rgba(icon)?;
    let mut png_bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_bytes, ICON_PX as u32, ICON_PX as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(&rgba).ok()?;
    }
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png_bytes);
    Some(format!("data:image/png;base64,{b64}"))
}

/// Draws the icon into a 32-bit DIB and returns straight-alpha RGBA.
fn render_rgba(icon: HICON) -> Option<Vec<u8>> {
    let len = (ICON_PX * ICON_PX * 4) as usize;
    let header = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: ICON_PX,
        biHeight: -ICON_PX, // top-down
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        ..Default::default()
    };
    let info = BITMAPINFO {
        bmiHeader: header,
        ..Default::default()
    };

    // SAFETY: the DIB's pixel buffer (`bits`, `len` bytes) is valid until DeleteObject; every
    // GDI object created here is released before returning.
    unsafe {
        let dc = CreateCompatibleDC(None);
        if dc.is_invalid() {
            return None;
        }
        let mut bits: *mut c_void = std::ptr::null_mut();
        let Ok(bitmap) = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
        else {
            let _ = DeleteDC(dc);
            return None;
        };
        let previous = SelectObject(dc, bitmap.into());
        let pixels = std::slice::from_raw_parts_mut(bits as *mut u8, len);

        pixels.fill(0);
        let drawn = DrawIconEx(dc, 0, 0, icon, ICON_PX, ICON_PX, 0, None, DI_NORMAL).is_ok();
        let _ = GdiFlush();
        let mut out = pixels.to_vec();

        // Legacy icons without an alpha channel: derive alpha from the mask (black = opaque).
        let has_alpha = out.as_chunks::<4>().0.iter().any(|p| p[3] != 0);
        if drawn && !has_alpha {
            pixels.fill(0xFF);
            let _ = DrawIconEx(dc, 0, 0, icon, ICON_PX, ICON_PX, 0, None, DI_MASK);
            let _ = GdiFlush();
            let masks = pixels.as_chunks::<4>().0;
            for (px, mask) in out.as_chunks_mut::<4>().0.iter_mut().zip(masks) {
                px[3] = 255 - mask[0];
            }
        }

        SelectObject(dc, previous);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(dc);

        if !drawn {
            return None;
        }
        // BGRA → RGBA; alpha icons come back premultiplied.
        for px in out.as_chunks_mut::<4>().0 {
            let a = px[3] as u32;
            let (b, g, r) = (px[0] as u32, px[1] as u32, px[2] as u32);
            let unpremultiply = |c: u32| {
                if has_alpha && a > 0 && a < 255 {
                    (c * 255 / a).min(255) as u8
                } else {
                    c as u8
                }
            };
            px[0] = unpremultiply(r);
            px[1] = unpremultiply(g);
            px[2] = unpremultiply(b);
        }
        Some(out)
    }
}
