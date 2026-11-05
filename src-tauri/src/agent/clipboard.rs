//! Attachments from the clipboard and from files dropped on the command window (2.10b.3, D52). Only paths cross
//! IPC, never bytes: copied files (Explorer's Copy, `CF_HDROP`) are their own paths, and a copied image (`PNG`, else
//! `CF_DIBV5`/`CF_DIB`) is saved as a file in Cay's `agent\pasted` folder first (pruned after a day). The message's
//! own checks and copies happen later, when it's sent (`attach::prepare`).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use windows::core::w;
use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW,
};
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

use super::attachments;
use crate::ipc::AttachmentPick;

/// The folder (under the agent's data folder) that holds pasted images.
pub const PASTED: &str = "pasted";
/// Pasted images older than this are deleted at the next paste.
const KEEP: Duration = Duration::from_secs(24 * 3600);
/// The longest side a pasted bitmap may have.
const MAX_SIDE: u32 = 16_384;
/// At most this many copied files are taken from one paste.
const MAX_DROPPED: u32 = 50;

const CF_DIB: u32 = 8;
const CF_HDROP: u32 = 15;
const CF_DIBV5: u32 = 17;

/// What the clipboard holds that can be attached, best first.
#[derive(Debug, PartialEq)]
pub enum Clip {
    Files(Vec<String>),
    Png(Vec<u8>),
    Dib(Vec<u8>),
    Nothing,
}

/// A bitmap from the clipboard: 32bpp BGR, top-down, as `desktop::encode_pixels` takes it.
#[derive(Debug, PartialEq)]
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// A PNG file starts with this.
pub fn is_png(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n")
}

fn u16_at(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(o..o + 2)?.try_into().ok()?))
}

fn u32_at(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(o..o + 4)?.try_into().ok()?))
}

/// A packed DIB (`CF_DIB` or `CF_DIBV5`: the header, masks or palette, then the rows) → top-down pixels. Takes
/// 24- and 32-bit RGB, and 32-bit bitfields in the usual BGR order; anything else is refused.
pub fn parse_dib(d: &[u8]) -> Result<Pixels, &'static str> {
    const BI_RGB: u32 = 0;
    const BI_BITFIELDS: u32 = 3;
    let bad = "the copied image isn't in a format Cay reads";
    let header = u32_at(d, 0).ok_or(bad)? as usize;
    let width = u32_at(d, 4).ok_or(bad)? as i32;
    let height = u32_at(d, 8).ok_or(bad)? as i32;
    let bits = u16_at(d, 14).ok_or(bad)?;
    let compression = u32_at(d, 16).ok_or(bad)?;
    let colors = u32_at(d, 32).ok_or(bad)? as usize;
    if header < 40 || header > d.len() || width <= 0 || height == 0 {
        return Err(bad);
    }
    let (w, h, top_down) = (width as u32, height.unsigned_abs(), height < 0);
    if w > MAX_SIDE || h > MAX_SIDE {
        return Err("the copied image is too large");
    }
    let mut offset = header;
    match (bits, compression) {
        (24 | 32, BI_RGB) => {}
        (32, BI_BITFIELDS) => {
            // The masks follow a plain header, and are part of a V4/V5 one; both at offset 40.
            let masks = (u32_at(d, 40), u32_at(d, 44), u32_at(d, 48));
            if masks != (Some(0x00FF_0000), Some(0x0000_FF00), Some(0x0000_00FF)) {
                return Err(bad);
            }
            if header == 40 {
                offset += 12;
            }
        }
        _ => return Err(bad),
    }
    offset += colors * 4;
    let bpp = bits as usize / 8;
    let stride = (w as usize * bits as usize).div_ceil(32) * 4;
    let end = offset
        .checked_add(stride.checked_mul(h as usize).ok_or(bad)?)
        .ok_or(bad)?;
    if end > d.len() {
        return Err(bad);
    }
    let mut data = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h as usize {
        let row = if top_down { y } else { h as usize - 1 - y };
        let start = offset + row * stride;
        for px in d[start..start + w as usize * bpp].chunks_exact(bpp) {
            data.extend_from_slice(&[px[0], px[1], px[2], 0xFF]);
        }
    }
    Ok(Pixels {
        width: w,
        height: h,
        data,
    })
}

/// A pasted image's file name: `pasted-20261105-141500-7f3a.png`.
pub fn pasted_name(at_ms: i64, mime: &str) -> String {
    let ext = if mime == "image/jpeg" { "jpg" } else { "png" };
    format!("pasted-{}.{ext}", super::audit::run_id(at_ms))
}

/// What a chip shows for each of `paths`: name, kind and size. A folder or a missing file is an error sentence.
pub fn describe(paths: &[String]) -> Result<Vec<AttachmentPick>, String> {
    paths
        .iter()
        .map(|raw| {
            let path = PathBuf::from(raw.trim());
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| raw.clone());
            let meta =
                std::fs::metadata(&path).map_err(|_| format!("{name} isn't there anymore."))?;
            if meta.is_dir() {
                return Err(format!(
                    "{name} is a folder. Attach the files in it instead."
                ));
            }
            Ok(AttachmentPick {
                path: path.to_string_lossy().into_owned(),
                kind: attachments::kind_of(&name),
                name,
                bytes: meta.len(),
            })
        })
        .collect()
}

/// Ctrl+V in the command window: what the clipboard holds as attachments, empty when it holds none (text).
pub fn paste(folder: &Path) -> Result<Vec<AttachmentPick>, String> {
    match read()? {
        Clip::Nothing => Ok(Vec::new()),
        Clip::Files(paths) => describe(&paths),
        Clip::Png(bytes) => save(folder, &bytes, "image/png"),
        Clip::Dib(bytes) => {
            let pixels = parse_dib(&bytes).map_err(|e| format!("Couldn't paste: {e}."))?;
            let encoded = crate::desktop::encode_pixels(pixels.width, pixels.height, pixels.data)
                .map_err(|e| format!("Couldn't paste the image: {e}."))?;
            save(folder, &encoded.bytes, encoded.mime)
        }
    }
}

fn save(folder: &Path, bytes: &[u8], mime: &str) -> Result<Vec<AttachmentPick>, String> {
    prune(folder);
    std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let path = folder.join(pasted_name(super::audit::now_ms(), mime));
    std::fs::write(&path, bytes).map_err(|e| format!("Couldn't save the pasted image: {e}"))?;
    describe(&[path.to_string_lossy().into_owned()])
}

/// Deletes pasted images older than a day.
fn prune(folder: &Path) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age > KEEP);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Reads the clipboard once. Another app may hold it for a moment, so opening is retried briefly.
fn read() -> Result<Clip, String> {
    let mut opened = false;
    for _ in 0..10 {
        // SAFETY: no owner window; closed below on every path once opened.
        if unsafe { OpenClipboard(None) }.is_ok() {
            opened = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    if !opened {
        return Err("Another app is using the clipboard. Try again.".into());
    }
    let clip = read_open();
    // SAFETY: balances the OpenClipboard above.
    let _ = unsafe { CloseClipboard() };
    Ok(clip)
}

/// The clipboard's attachable content; it must be open.
fn read_open() -> Clip {
    // SAFETY: the format name is a static wide string.
    let png = unsafe { RegisterClipboardFormatW(w!("PNG")) };
    // SAFETY: plain queries on the open clipboard.
    let has = |format: u32| format != 0 && unsafe { IsClipboardFormatAvailable(format) }.is_ok();
    if has(CF_HDROP) {
        // SAFETY: the clipboard is open; the handle stays the clipboard's (never freed here).
        if let Ok(handle) = unsafe { GetClipboardData(CF_HDROP) } {
            let files = dropped_files(HDROP(handle.0));
            if !files.is_empty() {
                return Clip::Files(files);
            }
        }
    }
    if has(png) {
        if let Some(bytes) = global_bytes(png).filter(|b| is_png(b)) {
            return Clip::Png(bytes);
        }
    }
    for format in [CF_DIBV5, CF_DIB] {
        if has(format) {
            if let Some(bytes) = global_bytes(format) {
                return Clip::Dib(bytes);
            }
        }
    }
    Clip::Nothing
}

/// The paths in a `CF_HDROP` handle.
fn dropped_files(drop: HDROP) -> Vec<String> {
    // SAFETY: `drop` is the open clipboard's CF_HDROP; 0xFFFFFFFF asks for the count.
    let count = unsafe { DragQueryFileW(drop, u32::MAX, None) }.min(MAX_DROPPED);
    (0..count)
        .filter_map(|i| {
            // SAFETY: as above; a None buffer returns the length without the NUL.
            let len = unsafe { DragQueryFileW(drop, i, None) } as usize;
            let mut buf = vec![0u16; len + 1];
            // SAFETY: `buf` has room for the path and its NUL.
            let got = unsafe { DragQueryFileW(drop, i, Some(&mut buf)) } as usize;
            (got > 0).then(|| String::from_utf16_lossy(&buf[..got]))
        })
        .collect()
}

/// A copy of the bytes of a clipboard format held in global memory.
fn global_bytes(format: u32) -> Option<Vec<u8>> {
    // SAFETY: the clipboard is open; the handle stays the clipboard's (never freed here).
    let handle: HANDLE = unsafe { GetClipboardData(format) }.ok()?;
    let global = HGLOBAL(handle.0);
    // SAFETY: `global` is a valid HGLOBAL owned by the clipboard; locked and unlocked here.
    unsafe {
        let size = GlobalSize(global);
        let ptr = GlobalLock(global) as *const u8;
        if ptr.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(ptr, size).to_vec();
        let _ = GlobalUnlock(global);
        Some(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::AttachmentKind;

    /// A plain 40-byte header.
    fn header(width: i32, height: i32, bits: u16, compression: u32) -> Vec<u8> {
        let mut h = Vec::new();
        h.extend_from_slice(&40u32.to_le_bytes());
        h.extend_from_slice(&width.to_le_bytes());
        h.extend_from_slice(&height.to_le_bytes());
        h.extend_from_slice(&1u16.to_le_bytes());
        h.extend_from_slice(&bits.to_le_bytes());
        h.extend_from_slice(&compression.to_le_bytes());
        h.extend_from_slice(&[0; 20]);
        h
    }

    #[test]
    fn bottom_up_24_bit() {
        // 2×2, rows padded to 8 bytes; the first row in the data is the bottom one.
        let mut d = header(2, 2, 24, 0);
        d.extend_from_slice(&[1, 2, 3, 4, 5, 6, 0, 0]); // bottom
        d.extend_from_slice(&[7, 8, 9, 10, 11, 12, 0, 0]); // top
        let p = parse_dib(&d).unwrap();
        assert_eq!((p.width, p.height), (2, 2));
        assert_eq!(
            p.data,
            vec![7, 8, 9, 255, 10, 11, 12, 255, 1, 2, 3, 255, 4, 5, 6, 255]
        );
    }

    #[test]
    fn top_down_32_bit_bitfields() {
        let mut d = header(1, -2, 32, 3);
        for mask in [0x00FF_0000u32, 0x0000_FF00, 0x0000_00FF] {
            d.extend_from_slice(&mask.to_le_bytes());
        }
        d.extend_from_slice(&[1, 2, 3, 0, 4, 5, 6, 0]);
        let p = parse_dib(&d).unwrap();
        assert_eq!((p.width, p.height), (1, 2));
        assert_eq!(p.data, vec![1, 2, 3, 255, 4, 5, 6, 255]);

        // Other channel orders are refused rather than guessed.
        let mut odd = header(1, 1, 32, 3);
        for mask in [0x0000_00FFu32, 0x0000_FF00, 0x00FF_0000] {
            odd.extend_from_slice(&mask.to_le_bytes());
        }
        odd.extend_from_slice(&[0; 4]);
        assert!(parse_dib(&odd).is_err());
    }

    #[test]
    fn short_odd_or_huge_dibs_are_refused() {
        assert!(parse_dib(&[]).is_err());
        let mut short = header(4, 4, 32, 0);
        short.extend_from_slice(&[0; 10]);
        assert!(parse_dib(&short).is_err());
        assert!(
            parse_dib(&header(2, 2, 8, 0)).is_err(),
            "palettes aren't read"
        );
        assert!(parse_dib(&header(0, 2, 24, 0)).is_err());
        assert!(parse_dib(&header(20_000, 1, 24, 0)).is_err());
        let mut rle = header(1, 1, 24, 1);
        rle.extend_from_slice(&[0; 4]);
        assert!(parse_dib(&rle).is_err());
    }

    #[test]
    fn png_and_names() {
        assert!(is_png(b"\x89PNG\r\n\x1a\nrest"));
        assert!(!is_png(b"GIF89a"));
        let name = pasted_name(1_793_000_000_000, "image/jpeg");
        assert!(name.starts_with("pasted-2026"), "{name}");
        assert!(name.ends_with(".jpg"));
        assert!(pasted_name(0, "image/png").ends_with(".png"));
    }

    #[test]
    fn describe_names_kinds_and_sizes() {
        let dir = std::env::temp_dir().join(format!("cay-clip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("notes.txt");
        std::fs::write(&file, "hello").unwrap();
        let image = dir.join("Shot.PNG");
        std::fs::write(&image, "x").unwrap();
        let picks =
            describe(&[file.display().to_string(), format!(" {} ", image.display())]).unwrap();
        assert_eq!(picks[0].name, "notes.txt");
        assert_eq!(picks[0].kind, AttachmentKind::File);
        assert_eq!(picks[0].bytes, 5);
        assert_eq!(picks[1].kind, AttachmentKind::Image);
        assert_eq!(picks[1].path, image.display().to_string());

        assert!(describe(&[dir.display().to_string()])
            .unwrap_err()
            .contains("folder"));
        assert!(describe(&[dir.join("gone.txt").display().to_string()])
            .unwrap_err()
            .contains("gone.txt"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pasted_bitmap_is_saved_as_an_image_file() {
        let dir = std::env::temp_dir().join(format!("cay-paste-{}", std::process::id()));
        let mut d = header(3, 2, 24, 0);
        d.extend_from_slice(&[0, 0, 255, 0, 255, 0, 255, 0, 0, 0, 0, 0]);
        d.extend_from_slice(&[9, 9, 9, 9, 9, 9, 9, 9, 9, 0, 0, 0]);
        let p = parse_dib(&d).unwrap();
        let encoded = crate::desktop::encode_pixels(p.width, p.height, p.data).unwrap();
        let picks = save(&dir, &encoded.bytes, encoded.mime).unwrap();
        assert_eq!(picks[0].kind, AttachmentKind::Image);
        assert!(picks[0].name.starts_with("pasted-"));
        let written = std::fs::read(&picks[0].path).unwrap();
        assert!(is_png(&written));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
