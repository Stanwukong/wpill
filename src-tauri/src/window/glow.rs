//! The screen-edge glow that shows while an agent run looks at the desktop (Phase 2.7c): a soft band of Claude's
//! accent along the four edges of the monitor being looked at, so you always know when Claude is looking.
//!
//! A native window, not a webview: layered (per-pixel alpha), click-through, never activated, topmost just below
//! the island, and excluded from every capture (`WDA_EXCLUDEFROMCAPTURE`), so it never shows up in a screenshot.
//! It's painted once per monitor with `UpdateLayeredWindow` and faded by stepping its constant alpha on a timer
//! that only runs during fades. Its thread owns the window and pumps its messages; `look_started` /
//! `look_ended` can be called from any thread. Spec from the motion-designer agent (2026-10-30).

use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, MonitorFromPoint,
    ReleaseDC, SelectObject, AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    BLENDFUNCTION, DIB_RGB_COLORS, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, KillTimer,
    PostMessageW, PostQuitMessage, RegisterClassExW, SetTimer, SetWindowDisplayAffinity,
    SetWindowPos, ShowWindow, SystemParametersInfoW, TranslateMessage, UpdateLayeredWindow,
    HWND_TOPMOST, MSG, SPI_GETCLIENTAREAANIMATION, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_HIDE,
    SW_SHOWNOACTIVATE, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, ULW_ALPHA, WDA_EXCLUDEFROMCAPTURE,
    WINDOW_EX_STYLE, WM_APP, WM_CLOSE, WM_DESTROY, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::activity::Watcher;
use crate::desktop::logic::{GlowState, Rect};
use crate::error::AppResult;
use crate::win32;

/// Claude's accent, the `--claude` token in `src/styles/global.css` (Phase 4a changes both together).
const COLOR: (f32, f32, f32) = (0xd7 as f32, 0x77 as f32, 0x57 as f32);
/// Band width in logical px at 100 % (× the monitor's DPI scale, not Text size).
const BAND: f32 = 32.0;
/// Alpha at the very edge.
const PEAK: f32 = 0.60;
const FADE_IN: Duration = Duration::from_millis(220);
const FADE_OUT: Duration = Duration::from_millis(400);
const FRAME_MS: u32 = 16;

const WM_GLOW_CHANGED: u32 = WM_APP + 1;
const TIMER_FADE: usize = 1;
const TIMER_HOLD: usize = 2;

/// One edge's contribution at `distance` px into a `band`-px band: `PEAK · (1 − t)²`.
fn edge(distance: f32, band: f32) -> f32 {
    let t = distance / band;
    if t >= 1.0 {
        0.0
    } else {
        PEAK * (1.0 - t) * (1.0 - t)
    }
}

/// The four edges combined like light: `1 − Π(1 − eᵢ)`.
fn intensity(x: f32, y: f32, width: f32, height: f32, band: f32) -> f32 {
    let edges = [x, y, width - 1.0 - x, height - 1.0 - y];
    1.0 - edges.iter().map(|&d| 1.0 - edge(d, band)).product::<f32>()
}

fn ease_out_cubic(p: f32) -> f32 {
    1.0 - (1.0 - p).powi(3)
}

fn ease_in_quad(p: f32) -> f32 {
    p * p
}

struct Fade {
    from: f32,
    to: f32,
    started: Instant,
    duration: Duration,
}

#[derive(Default)]
struct Shared {
    hwnd: isize,
    /// Kept below this window (the island), so the island draws over the top band.
    island: isize,
    state: GlowState,
    /// The monitor of the latest look, and the one the bitmap was painted for.
    monitor: Rect,
    painted: Rect,
    alpha: f32,
    fade: Option<Fade>,
}

fn shared() -> MutexGuard<'static, Shared> {
    static SHARED: OnceLock<Mutex<Shared>> = OnceLock::new();
    SHARED
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// A look at `monitor` starts: the glow comes up there (or stays up).
pub fn look_started(monitor: Rect) {
    let hwnd = {
        let mut s = shared();
        s.state.start();
        if !monitor.is_empty() {
            s.monitor = monitor;
        }
        s.hwnd
    };
    poke(hwnd);
}

/// A look ended: the glow holds a moment, then fades unless another look starts.
pub fn look_ended() {
    let hwnd = {
        let mut s = shared();
        s.state.end(Instant::now());
        s.hwnd
    };
    poke(hwnd);
}

fn poke(hwnd: isize) {
    if hwnd != 0 {
        // SAFETY: posting to our own window; fails harmlessly if it's gone.
        let _ = unsafe {
            PostMessageW(
                Some(win32::from_raw(hwnd)),
                WM_GLOW_CHANGED,
                WPARAM(0),
                LPARAM(0),
            )
        };
    }
}

/// Starts the glow's thread and window. `island` is kept above it.
pub fn spawn(island: HWND) -> AppResult<Watcher> {
    let island = win32::raw(island);
    Watcher::spawn(
        "agent-glow",
        move || {
            if let Err(e) = run(island) {
                log::warn!("glow unavailable: {e}");
            }
        },
        || {
            let hwnd = std::mem::take(&mut shared().hwnd);
            if hwnd != 0 {
                // SAFETY: asks our own window to close; its thread then leaves the message loop.
                let _ = unsafe {
                    PostMessageW(Some(win32::from_raw(hwnd)), WM_CLOSE, WPARAM(0), LPARAM(0))
                };
            }
        },
    )
}

fn run(island: isize) -> windows::core::Result<()> {
    // SAFETY: registers our class and creates a window owned by this thread, whose messages it pumps until
    // WM_DESTROY posts quit.
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: w!("TopBarGlow"),
            ..Default::default()
        };
        RegisterClassExW(&class);
        let ex =
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST;
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(ex.0),
            w!("TopBarGlow"),
            w!("TopBar glow"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        if let Err(e) = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) {
            log::warn!("glow can't be excluded from capture: {e}");
        }
        {
            let mut s = shared();
            s.hwnd = win32::raw(hwnd);
            s.island = island;
        }
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_GLOW_CHANGED | WM_TIMER => {
            update(hwnd);
            LRESULT(0)
        }
        WM_CLOSE => {
            // SAFETY: our own window, on its own thread.
            let _ = unsafe { DestroyWindow(hwnd) };
            LRESULT(0)
        }
        WM_DESTROY => {
            // SAFETY: ends this thread's message loop.
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        // SAFETY: default handling for everything else.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Moves the glow one step towards what `GlowState` says, and schedules the next step.
fn update(hwnd: HWND) {
    let now = Instant::now();
    let mut s = shared();
    let want = if s.state.visible(now) { 1.0 } else { 0.0 };
    let animate = animations_on();

    if want > 0.0 && s.monitor != s.painted && !s.monitor.is_empty() {
        let monitor = s.monitor;
        if paint(hwnd, monitor, s.alpha) {
            s.painted = monitor;
        }
    }
    // Retarget the fade from wherever it is now, so a new look during a fade-out never dips and re-flashes.
    let target = s.fade.as_ref().map_or(s.alpha, |f| f.to);
    if (target - want).abs() > f32::EPSILON {
        let duration = if want > 0.0 { FADE_IN } else { FADE_OUT };
        s.fade = Some(Fade {
            from: s.alpha,
            to: want,
            started: now,
            duration,
        });
    }
    if let Some((from, to, started, duration)) = s
        .fade
        .as_ref()
        .map(|f| (f.from, f.to, f.started, f.duration))
    {
        let p = if animate {
            (now - started).as_secs_f32() / duration.as_secs_f32()
        } else {
            1.0
        };
        let p = p.clamp(0.0, 1.0);
        let eased = if to > from {
            ease_out_cubic(p)
        } else {
            ease_in_quad(p)
        };
        s.alpha = from + (to - from) * eased;
        if p >= 1.0 {
            s.alpha = to;
            s.fade = None;
        }
    }
    let (alpha, fading, island) = (s.alpha, s.fade.is_some(), s.island);
    let hold = s.state.next_change(now);
    drop(s);

    set_alpha(hwnd, alpha, island);
    // SAFETY: timers on our own window, on its thread.
    unsafe {
        if fading {
            SetTimer(Some(hwnd), TIMER_FADE, FRAME_MS, None);
        } else {
            let _ = KillTimer(Some(hwnd), TIMER_FADE);
        }
        match hold {
            Some(left) => {
                SetTimer(Some(hwnd), TIMER_HOLD, left.as_millis() as u32 + 1, None);
            }
            None => {
                let _ = KillTimer(Some(hwnd), TIMER_HOLD);
            }
        }
    }
}

/// Shows (above everything but the island) or hides the window at `alpha`.
fn set_alpha(hwnd: HWND, alpha: f32, island: isize) {
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: (alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    // SAFETY: our own layered window; with no source DC, UpdateLayeredWindow only changes the blend.
    unsafe {
        if alpha <= 0.0 {
            let _ = ShowWindow(hwnd, SW_HIDE);
            return;
        }
        let _ = UpdateLayeredWindow(
            hwnd,
            None,
            None,
            None,
            None,
            None,
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        );
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, flags);
        if island != 0 {
            // The island goes back on top of the glow.
            let _ = SetWindowPos(
                win32::from_raw(island),
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                flags,
            );
        }
    }
}

/// Paints the band for `monitor` into the layered window (and moves it there).
fn paint(hwnd: HWND, monitor: Rect, alpha: f32) -> bool {
    let (w, h) = (monitor.width, monitor.height);
    let band = BAND * monitor_scale(monitor);
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    // SAFETY: the DCs and the DIB are freed before returning; `bits` holds w*h*4 bytes while the DIB exists.
    unsafe {
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let mut bits = std::ptr::null_mut();
        let Ok(dib) = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) else {
            let _ = DeleteDC(dc);
            ReleaseDC(None, screen);
            return false;
        };
        let pixels = std::slice::from_raw_parts_mut(bits as *mut u8, w as usize * h as usize * 4);
        fill(pixels, w as usize, h as usize, band);
        let old = SelectObject(dc, dib.into());
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: (alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let ok = UpdateLayeredWindow(
            hwnd,
            Some(screen),
            Some(&POINT {
                x: monitor.x,
                y: monitor.y,
            }),
            Some(&SIZE { cx: w, cy: h }),
            Some(dc),
            Some(&POINT { x: 0, y: 0 }),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        )
        .is_ok();
        SelectObject(dc, old);
        let _ = DeleteObject(dib.into());
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
        ok
    }
}

/// Premultiplied BGRA: one color, alpha from `intensity`. Only the band is touched; the middle stays clear.
fn fill(pixels: &mut [u8], width: usize, height: usize, band: f32) {
    let (r, g, b) = COLOR;
    let reach = band.ceil() as usize;
    for y in 0..height {
        let near_y = y < reach || y + reach >= height;
        for x in 0..width {
            if !near_y && x >= reach && x + reach < width {
                continue;
            }
            let a =
                intensity(x as f32, y as f32, width as f32, height as f32, band).clamp(0.0, 1.0);
            let i = (y * width + x) * 4;
            pixels[i] = (b * a).round() as u8;
            pixels[i + 1] = (g * a).round() as u8;
            pixels[i + 2] = (r * a).round() as u8;
            pixels[i + 3] = (255.0 * a).round() as u8;
        }
    }
}

/// The monitor's DPI scale (1.0 at 96 DPI).
fn monitor_scale(monitor: Rect) -> f32 {
    let (mut x, mut y) = (96u32, 96u32);
    // SAFETY: MonitorFromPoint accepts any point; the out-params outlive the call.
    unsafe {
        let m = MonitorFromPoint(
            POINT {
                x: monitor.x + monitor.width / 2,
                y: monitor.y + monitor.height / 2,
            },
            MONITOR_DEFAULTTONEAREST,
        );
        let _ = GetDpiForMonitor(m, MDT_EFFECTIVE_DPI, &mut x, &mut y);
    }
    x.max(96) as f32 / 96.0
}

/// Windows' "Animation effects" switch (the reduced-motion setting).
fn animations_on() -> bool {
    let mut on = windows::core::BOOL(1);
    // SAFETY: `on` is the BOOL this action writes.
    let _ = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(&mut on as *mut _ as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    on.as_bool()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_falloff_and_corners() {
        assert!((edge(0.0, 32.0) - PEAK).abs() < 1e-6);
        assert!((edge(16.0, 32.0) - PEAK * 0.25).abs() < 1e-6);
        assert_eq!(edge(32.0, 32.0), 0.0);
        assert_eq!(edge(100.0, 32.0), 0.0);
        let mid = intensity(960.0, 540.0, 1920.0, 1080.0, 32.0);
        assert_eq!(mid, 0.0);
        let side = intensity(0.0, 540.0, 1920.0, 1080.0, 32.0);
        let corner = intensity(0.0, 0.0, 1920.0, 1080.0, 32.0);
        assert!((side - PEAK).abs() < 1e-6);
        assert!(corner > side && corner < 1.0);
    }

    #[test]
    fn fill_touches_only_the_band() {
        let (w, h) = (200, 100);
        let mut px = vec![0u8; w * h * 4];
        fill(&mut px, w, h, 10.0);
        let at = |x: usize, y: usize| px[(y * w + x) * 4 + 3];
        assert_eq!(at(100, 50), 0);
        assert_eq!(at(0, 50), (255.0 * PEAK).round() as u8);
        assert!(at(5, 50) > 0 && at(5, 50) < at(0, 50));
        // Premultiplied: no channel above alpha.
        let i = (50 * w) * 4;
        assert!(px[i] <= px[i + 3] && px[i + 1] <= px[i + 3] && px[i + 2] <= px[i + 3]);
    }

    #[test]
    fn easing_ends() {
        assert_eq!(ease_out_cubic(0.0), 0.0);
        assert_eq!(ease_out_cubic(1.0), 1.0);
        assert_eq!(ease_in_quad(1.0), 1.0);
        assert!(ease_out_cubic(0.5) > 0.5);
    }
}
