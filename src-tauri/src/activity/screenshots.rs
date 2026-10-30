//! The `screenshot-watcher` thread: new images in the Screenshots folder (Win+PrtScn, and Snipping Tool's
//! auto-save) become a screenshot activity with a thumbnail.
//!
//! One overlapped `ReadDirectoryChangesW` at a time, waited on together with a stop event, so shutdown
//! cancels the pending read instead of leaving it outstanding.

use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine;
use windows::core::{Result, HSTRING};
use windows::Win32::Foundation::{SIZE, WAIT_OBJECT_0};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadDirectoryChangesW, FILE_ACTION_ADDED, FILE_ACTION_RENAMED_NEW_NAME,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED, FILE_LIST_DIRECTORY,
    FILE_NOTIFY_CHANGE_FILE_NAME, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};
use windows::Win32::System::Com::{
    CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_MULTITHREADED,
};
use windows::Win32::System::Threading::{SetEvent, WaitForMultipleObjects, INFINITE};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::UI::Shell::{
    FOLDERID_Screenshots, IShellItemImageFactory, SHCreateItemFromParsingName,
    SHGetKnownFolderPath, KF_FLAG_CREATE, SIIGBF_RESIZETOFIT,
};

use super::{Signal, Watcher};
use crate::error::AppResult;
use crate::ipc::{Activity, ScreenshotActivity};
use crate::win32::Handle;

/// Long side of the thumbnail in px (shown at ~128 CSS px, so crisp up to 200 %).
const THUMB_PX: i32 = 256;
/// The writer may still have the file open when it appears; wait this long at most.
const WRITE_WAIT: Duration = Duration::from_secs(3);
/// A file that's added and then renamed (Snipping Tool) reports twice.
const DUPLICATE_WINDOW: Duration = Duration::from_secs(3);
const EXTENSIONS: [&str; 3] = ["png", "jpg", "jpeg"];

pub fn start(out: Sender<Signal>) -> AppResult<Watcher> {
    let stop = Arc::new(Handle::event()?);
    let signal = stop.clone();
    Watcher::spawn(
        "screenshot-watcher",
        move || run(&out, &stop),
        move || {
            // SAFETY: signals the event both sides hold through the Arc.
            let _ = unsafe { SetEvent(signal.0) };
        },
    )
}

fn run(out: &Sender<Signal>, stop: &Handle) {
    // SAFETY: called once at the top of this thread; balanced by CoUninitialize below.
    let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    match watch(out, stop) {
        Ok(()) => log::info!("screenshot watcher stopped"),
        Err(e) => log::warn!("screenshot activities unavailable: {e}"),
    }
    if com.is_ok() {
        // SAFETY: matches the successful CoInitializeEx; every COM object is dropped by now.
        unsafe { CoUninitialize() };
    }
}

/// The Screenshots folder, wherever it's been moved (OneDrive, another drive). Created if missing.
fn folder() -> Result<PathBuf> {
    // SAFETY: the returned string is copied and then freed with CoTaskMemFree.
    unsafe {
        let path = SHGetKnownFolderPath(&FOLDERID_Screenshots, KF_FLAG_CREATE, None)?;
        let out = PathBuf::from(String::from_utf16_lossy(path.as_wide()));
        CoTaskMemFree(Some(path.0 as *const _));
        Ok(out)
    }
}

fn watch(out: &Sender<Signal>, stop: &Handle) -> Result<()> {
    let folder = folder()?;
    // SAFETY: opens the directory for change notifications; closed by the guard.
    let dir = Handle(unsafe {
        CreateFileW(
            &HSTRING::from(folder.as_os_str()),
            FILE_LIST_DIRECTORY.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
            None,
        )?
    });
    let io = Handle::event()?;
    log::debug!("watching screenshots in {}", folder.display());

    // u32s, because the records must be DWORD-aligned.
    let mut buffer = vec![0u32; 16 * 1024];
    let mut recent: Vec<(PathBuf, Instant)> = Vec::new();
    loop {
        let mut overlapped = OVERLAPPED {
            hEvent: io.0,
            ..Default::default()
        };
        let mut bytes = 0u32;
        // SAFETY: buffer and overlapped stay alive and unmoved until the read completes or is cancelled
        // below; every path out of this block waits for that.
        unsafe {
            ReadDirectoryChangesW(
                dir.0,
                buffer.as_mut_ptr().cast(),
                (buffer.len() * 4) as u32,
                false,
                FILE_NOTIFY_CHANGE_FILE_NAME,
                None,
                Some(&mut overlapped),
                None,
            )?;
            let waited = WaitForMultipleObjects(&[io.0, stop.0], false, INFINITE);
            if waited != WAIT_OBJECT_0 {
                let _ = CancelIoEx(dir.0, Some(&overlapped));
                let _ = GetOverlappedResult(dir.0, &overlapped, &mut bytes, true);
                return Ok(());
            }
            GetOverlappedResult(dir.0, &overlapped, &mut bytes, false)?;
        }
        // Zero bytes means the buffer overflowed: changes were lost, nothing to parse.
        // SAFETY: reinterpreting the u32 buffer as bytes; `bytes` never exceeds its length.
        let raw = unsafe {
            std::slice::from_raw_parts(
                buffer.as_ptr().cast::<u8>(),
                (bytes as usize).min(buffer.len() * 4),
            )
        };
        let now = Instant::now();
        recent.retain(|(_, at)| now.duration_since(*at) < DUPLICATE_WINDOW);
        for name in added_names(raw) {
            let path = folder.join(&name);
            if !is_image(&path) || recent.iter().any(|(p, _)| *p == path) {
                continue;
            }
            recent.push((path.clone(), now));
            match screenshot(&path) {
                Ok(activity) => {
                    log::debug!("screenshot saved: {name}");
                    let _ = out.send(Activity::Screenshot(activity).into());
                }
                Err(e) => log::debug!("screenshot {name} skipped: {e}"),
            }
        }
    }
}

/// Names from a `FILE_NOTIFY_INFORMATION` chain that were added or renamed into the folder.
fn added_names(raw: &[u8]) -> Vec<String> {
    let u32_at = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(raw.get(at..at + 4)?.try_into().ok()?))
    };
    let mut names = Vec::new();
    let mut offset = 0usize;
    while let (Some(next), Some(action), Some(len)) =
        (u32_at(offset), u32_at(offset + 4), u32_at(offset + 8))
    {
        let start = offset + 12;
        let Some(name) = raw.get(start..start + len as usize) else {
            break;
        };
        if action == FILE_ACTION_ADDED.0 || action == FILE_ACTION_RENAMED_NEW_NAME.0 {
            let units: Vec<u16> = name
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_le_bytes(*c))
                .collect();
            names.push(String::from_utf16_lossy(&units));
        }
        if next == 0 {
            break;
        }
        offset += next as usize;
    }
    names
}

fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

fn screenshot(path: &Path) -> std::result::Result<ScreenshotActivity, String> {
    wait_until_written(path)?;
    let (thumbnail, width, height) = thumbnail(path).map_err(|e| e.to_string())?;
    Ok(ScreenshotActivity {
        file_name: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        thumbnail,
        width,
        height,
        path: path.to_path_buf(),
    })
}

/// An exclusive open only succeeds once the writer has closed the file.
fn wait_until_written(path: &Path) -> std::result::Result<(), String> {
    let deadline = Instant::now() + WRITE_WAIT;
    loop {
        match std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(path)
        {
            Ok(file) if file.metadata().is_ok_and(|m| m.len() > 0) => return Ok(()),
            Ok(_) | Err(_) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(100))
            }
            Ok(_) => return Err("file stayed empty".into()),
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// The shell's thumbnail (cached by Explorer, any image type) as a PNG data URL, with its size.
fn thumbnail(path: &Path) -> Result<(String, u32, u32)> {
    // SAFETY: the bitmap returned by GetImage is ours and is deleted after conversion.
    unsafe {
        let factory: IShellItemImageFactory =
            SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None)?;
        let bitmap = factory.GetImage(
            SIZE {
                cx: THUMB_PX,
                cy: THUMB_PX,
            },
            SIIGBF_RESIZETOFIT,
        )?;
        let result = to_png(bitmap);
        let _ = DeleteObject(bitmap.into());
        result.ok_or_else(windows::core::Error::from_thread)
    }
}

fn to_png(bitmap: HBITMAP) -> Option<(String, u32, u32)> {
    // SAFETY: GetObjectW fills a BITMAP of the given size; GetDIBits writes at most width × height × 4
    // bytes into `pixels`, which is that large. The DC is deleted before returning.
    unsafe {
        let mut info = BITMAP::default();
        let size = size_of::<BITMAP>() as i32;
        if GetObjectW(bitmap.into(), size, Some((&mut info as *mut BITMAP).cast())) != size {
            return None;
        }
        let (width, height) = (info.bmWidth, info.bmHeight.abs());
        if width <= 0 || height <= 0 {
            return None;
        }
        let mut header = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        let dc = CreateCompatibleDC(None);
        let lines = GetDIBits(
            dc,
            bitmap,
            0,
            height as u32,
            Some(pixels.as_mut_ptr().cast()),
            &mut header,
            DIB_RGB_COLORS,
        );
        let _ = DeleteDC(dc);
        if lines != height {
            return None;
        }
        // BGRA → RGBA. Screenshots are opaque, and shell bitmaps often leave alpha at 0.
        for px in pixels.as_chunks_mut::<4>().0 {
            px.swap(0, 2);
            px[3] = 255;
        }
        let mut png_bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png_bytes, width as u32, height as u32);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().ok()?;
            writer.write_image_data(&pixels).ok()?;
        }
        let b64 = base64::engine::general_purpose::STANDARD.encode(&png_bytes);
        Some((
            format!("data:image/png;base64,{b64}"),
            width as u32,
            height as u32,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One `FILE_NOTIFY_INFORMATION` record, padded to a DWORD boundary. `last` ends the chain.
    fn record(action: u32, name: &str, last: bool) -> Vec<u8> {
        let units: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let len = (12 + units.len()).next_multiple_of(4);
        let next = if last { 0 } else { len as u32 };
        let mut out = Vec::new();
        out.extend(next.to_le_bytes());
        out.extend(action.to_le_bytes());
        out.extend((units.len() as u32).to_le_bytes());
        out.extend(units);
        out.resize(len, 0);
        out
    }

    #[test]
    fn parses_added_and_renamed_names_and_skips_others() {
        let mut raw = record(FILE_ACTION_ADDED.0, "Screenshot (1).png", false);
        raw.extend(record(2 /* removed */, "old.png", false));
        raw.extend(record(FILE_ACTION_RENAMED_NEW_NAME.0, "Snip.PNG", true));
        assert_eq!(added_names(&raw), vec!["Screenshot (1).png", "Snip.PNG"]);
        assert!(added_names(&raw[..10]).is_empty());
    }

    #[test]
    fn only_images_count() {
        assert!(is_image(Path::new("a/Snip.PNG")));
        assert!(is_image(Path::new("b.jpeg")));
        assert!(!is_image(Path::new("c.png.tmp")));
        assert!(!is_image(Path::new("desktop.ini")));
    }
}
