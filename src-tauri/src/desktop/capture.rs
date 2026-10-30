//! Pixels: a window with `PrintWindow(PW_RENDERFULLCONTENT)` (works when it's covered or partly off-screen, no
//! capture border), or a monitor with `BitBlt` from the screen. For the screen, TopBar's own windows are excluded
//! from capture (`WDA_EXCLUDEFROMCAPTURE`) only for the moment of the capture, so your own screenshots and screen
//! shares still see the island. DRM-protected content comes out black either way.

use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC,
    SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, HDC, SRCCOPY,
};
use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowDisplayAffinity, IsWindowVisible, SetWindowDisplayAffinity,
    WDA_EXCLUDEFROMCAPTURE, WDA_NONE,
};

use super::logic::Rect;
use crate::win32;

/// `PW_RENDERFULLCONTENT` (Windows 8.1+): DWM's copy of the window, including DirectComposition content.
const PW_RENDERFULLCONTENT: PRINT_WINDOW_FLAGS = PRINT_WINDOW_FLAGS(2);
/// Larger than any real monitor or window; a bogus rect shouldn't allocate gigabytes.
const MAX_SIDE: i32 = 16_384;

/// Top-down 32-bit BGRX pixels.
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// `hwnd`'s visible frame (`frame`, inside `window`, the full window rect PrintWindow draws).
pub fn window(hwnd: HWND, window: Rect, frame: Rect) -> Result<Bitmap, String> {
    let full = draw(window.width, window.height, |dc| {
        // SAFETY: `dc` is a memory DC with a DIB of the window's size selected.
        unsafe { PrintWindow(hwnd, dc, PW_RENDERFULLCONTENT).as_bool() }
    })?;
    // Crop off the invisible resize borders.
    let crop = frame.intersect(&window);
    if crop.is_empty() {
        return Ok(full);
    }
    Ok(cropped(
        &full,
        crop.x - window.x,
        crop.y - window.y,
        crop.width,
        crop.height,
    ))
}

/// The screen inside `area` (a monitor), without TopBar's windows.
pub fn screen(area: Rect) -> Result<Bitmap, String> {
    let hidden = exclude_own_windows();
    let result = draw(area.width, area.height, |dc| {
        // SAFETY: the screen DC is released before returning; `dc` holds a DIB of `area`'s size.
        unsafe {
            let screen = GetDC(None);
            let ok = BitBlt(
                dc,
                0,
                0,
                area.width,
                area.height,
                Some(screen),
                area.x,
                area.y,
                SRCCOPY | CAPTUREBLT,
            )
            .is_ok();
            ReleaseDC(None, screen);
            ok
        }
    });
    restore(&hidden);
    result
}

/// Draws into a fresh `width`×`height` DIB with `paint`, and copies the pixels out.
fn draw(width: i32, height: i32, paint: impl FnOnce(HDC) -> bool) -> Result<Bitmap, String> {
    if width <= 0 || height <= 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Err(format!("nothing to capture ({width}x{height})"));
    }
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            // Negative: top-down rows.
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    // SAFETY: the DC and bitmap are freed before returning; `bits` points at width*height*4 bytes owned by the
    // DIB section while it exists, and is copied out before it's deleted.
    unsafe {
        let dc = CreateCompatibleDC(None);
        if dc.is_invalid() {
            return Err("no memory DC".into());
        }
        let mut bits = std::ptr::null_mut();
        let bitmap = match CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(b) if !bits.is_null() => b,
            _ => {
                let _ = DeleteDC(dc);
                return Err("couldn't allocate the capture".into());
            }
        };
        let old = SelectObject(dc, bitmap.into());
        let ok = paint(dc);
        let len = width as usize * height as usize * 4;
        let pixels = std::slice::from_raw_parts(bits as *const u8, len).to_vec();
        SelectObject(dc, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(dc);
        if !ok {
            return Err("Windows couldn't capture it".into());
        }
        Ok(Bitmap {
            width: width as u32,
            height: height as u32,
            pixels,
        })
    }
}

fn cropped(b: &Bitmap, x: i32, y: i32, width: i32, height: i32) -> Bitmap {
    let (x, y) = (x.max(0) as usize, y.max(0) as usize);
    let width = (width as usize).min(b.width as usize - x.min(b.width as usize));
    let height = (height as usize).min(b.height as usize - y.min(b.height as usize));
    let stride = b.width as usize * 4;
    let mut pixels = Vec::with_capacity(width * height * 4);
    for row in y..y + height {
        let start = row * stride + x * 4;
        pixels.extend_from_slice(&b.pixels[start..start + width * 4]);
    }
    Bitmap {
        width: width as u32,
        height: height as u32,
        pixels,
    }
}

/// Marks every visible top-level window of ours as excluded from capture and waits for DWM to apply it.
/// Returns the ones changed (windows already excluded, like the glow, are left alone).
fn exclude_own_windows() -> Vec<isize> {
    let mut own: Vec<isize> = Vec::new();
    // SAFETY: the callback only pushes into `own`, which outlives the call.
    let _ = unsafe {
        EnumWindows(
            Some(collect_own),
            LPARAM(&mut own as *mut Vec<isize> as isize),
        )
    };
    let mut changed = Vec::new();
    for raw in own {
        let hwnd = win32::from_raw(raw);
        let mut current = 0u32;
        // SAFETY: our own windows; `current` outlives the call.
        unsafe {
            let _ = GetWindowDisplayAffinity(hwnd, &mut current);
            if current == WDA_EXCLUDEFROMCAPTURE.0 {
                continue;
            }
            match SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) {
                Ok(()) => changed.push(raw),
                Err(e) => log::warn!("couldn't exclude window {raw:#x} from capture: {e}"),
            }
        }
    }
    // Two compositions: the first may already be under way with the old affinity.
    // SAFETY: plain DWM calls.
    unsafe {
        let _ = DwmFlush();
        let _ = DwmFlush();
    }
    changed
}

fn restore(windows: &[isize]) {
    for &raw in windows {
        // SAFETY: our own window.
        let _ = unsafe { SetWindowDisplayAffinity(win32::from_raw(raw), WDA_NONE) };
    }
}

unsafe extern "system" fn collect_own(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: lparam is the `&mut Vec<isize>` passed by `exclude_own_windows`.
    let own = unsafe { &mut *(lparam.0 as *mut Vec<isize>) };
    // SAFETY: read-only query.
    if win32::window_pid(hwnd) == std::process::id() && unsafe { IsWindowVisible(hwnd).as_bool() } {
        own.push(win32::raw(hwnd));
    }
    true.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_rows() {
        // 3x2, pixel value = index.
        let b = Bitmap {
            width: 3,
            height: 2,
            pixels: (0..6u8).flat_map(|i| [i, i, i, 0]).collect(),
        };
        let c = cropped(&b, 1, 1, 5, 5);
        assert_eq!((c.width, c.height), (2, 1));
        assert_eq!(c.pixels, vec![4, 4, 4, 0, 5, 5, 5, 0]);
    }
}
