//! Looking at the desktop for agent runs (Phase 2.7c, CASESTUDY D25): window lists, the focused window,
//! screenshots and UI Automation trees. Read-only: nothing here focuses, clicks, types, moves or resizes.
//!
//! - `windows.rs`: the Alt+Tab list (from `apps/`) and the checks every target goes through.
//! - `capture.rs` + `encode.rs`: pixels (PrintWindow / BitBlt) → PNG or JPEG within budget (WIC).
//! - `uia.rs` → `tree.rs`: the UI Automation walk → a capped outline. `logic.rs`: sizes, errors, glow timing.
//!
//! TopBar's own windows are never listed, captured or read. Every look runs on its own MTA thread with a time
//! limit, so a hung app can't hold up the agent's connection.

pub mod logic;
pub mod tree;

mod capture;
mod encode;
mod uia;
mod windows;

use std::sync::mpsc;
use std::time::Duration;

use ::windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

pub use logic::{LookError, Rect};
pub use windows::{Target, WindowEntry};

use crate::win32;
use tree::{Caps, Cut};

/// A look that takes longer than this is abandoned (its thread finishes on its own).
const LOOK_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, PartialEq)]
pub enum Look {
    ListWindows,
    FocusedWindow,
    /// `None`: the primary monitor.
    Screenshot {
        hwnd: Option<i64>,
    },
    UiTree {
        hwnd: i64,
        caps: Caps,
    },
}

impl Look {
    pub fn hwnd(&self) -> Option<i64> {
        match self {
            Look::Screenshot { hwnd } => *hwnd,
            Look::UiTree { hwnd, .. } => Some(*hwnd),
            _ => None,
        }
    }
}

pub struct Image {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    pub width: u32,
    pub height: u32,
    /// The captured area's top-left, in physical screen px.
    pub origin: Rect,
    /// Screen px per image px.
    pub scale: f64,
}

pub enum Outcome {
    Windows(Vec<WindowEntry>),
    /// `None` with `topbar` true: TopBar's own window is in front (never described).
    Focused {
        window: Option<WindowEntry>,
        topbar: bool,
    },
    Image {
        image: Image,
        target: Option<Target>,
    },
    Tree {
        outline: String,
        nodes: usize,
        cut: Option<Cut>,
        target: Target,
    },
}

/// The window a look names, for its approval summary. `None` if it's gone or one of ours.
pub fn identify(hwnd: i64) -> Option<Target> {
    windows::identify(hwnd)
}

/// The monitor a look is about (for the glow): the window's, or the primary one.
pub fn monitor_for(look: &Look) -> Rect {
    windows::monitor_rect(look.hwnd().map(|h| h as isize))
}

/// Runs `look`. `expect_pid` is the target's process when it was approved (a reused HWND is refused).
pub fn run(look: Look, expect_pid: Option<u32>) -> Result<Outcome, LookError> {
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("desktop-look".into())
        .spawn(move || {
            // SAFETY: COM for this thread only (WIC, UIA); balanced below.
            let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
            let result = look_now(look, expect_pid);
            if com {
                // SAFETY: balances the CoInitializeEx above; every COM object is dropped by now.
                unsafe { CoUninitialize() };
            }
            let _ = tx.send(result);
        });
    if let Err(e) = spawned {
        return Err(LookError::Failed(e.to_string()));
    }
    rx.recv_timeout(LOOK_TIMEOUT)
        .unwrap_or(Err(LookError::NotResponding))
}

fn look_now(look: Look, expect_pid: Option<u32>) -> Result<Outcome, LookError> {
    if windows::screen_locked() {
        return Err(LookError::ScreenLocked);
    }
    match look {
        Look::ListWindows => Ok(Outcome::Windows(windows::list())),
        Look::FocusedWindow => Ok(match windows::focused() {
            Ok(window) => Outcome::Focused {
                window,
                topbar: false,
            },
            Err(()) => Outcome::Focused {
                window: None,
                topbar: true,
            },
        }),
        Look::Screenshot { hwnd: None } => {
            let area = windows::monitor_rect(None);
            let bitmap = capture::screen(area).map_err(LookError::Failed)?;
            Ok(Outcome::Image {
                image: image(&bitmap, area)?,
                target: None,
            })
        }
        Look::Screenshot { hwnd: Some(hwnd) } => {
            let target = windows::check(hwnd, expect_pid)?;
            if windows::is_minimized(target.hwnd) {
                return Err(LookError::Minimized);
            }
            let h = win32::from_raw(target.hwnd);
            let frame = windows::frame_bounds(h);
            let bitmap =
                capture::window(h, windows::window_rect(h), frame).map_err(LookError::Failed)?;
            Ok(Outcome::Image {
                image: image(&bitmap, frame)?,
                target: Some(target),
            })
        }
        Look::UiTree { hwnd, caps } => {
            let target = windows::check(hwnd, expect_pid)?;
            let (nodes, cut) =
                uia::walk(target.hwnd, &caps).map_err(|e| LookError::Failed(e.message()))?;
            Ok(Outcome::Tree {
                outline: tree::render(&nodes, cut, &caps),
                nodes: nodes.len(),
                cut,
                target,
            })
        }
    }
}

fn image(bitmap: &capture::Bitmap, area: Rect) -> Result<Image, LookError> {
    let encoded = encode::encode(bitmap).map_err(|e| LookError::Failed(e.message()))?;
    if encoded.bytes.len() > logic::HARD_MAX_IMAGE_BYTES {
        return Err(LookError::Failed("the image came out too large".into()));
    }
    Ok(Image {
        scale: f64::from(bitmap.width) / f64::from(encoded.width.max(1)),
        bytes: encoded.bytes,
        mime: encoded.mime,
        width: encoded.width,
        height: encoded.height,
        origin: area,
    })
}
