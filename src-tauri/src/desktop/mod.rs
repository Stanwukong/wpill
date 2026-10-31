//! The desktop for agent runs: looking (Phase 2.7c, CASESTUDY D25) and acting (Phase 2.7d, D27).
//!
//! - `windows.rs`: the Alt+Tab list (from `apps/`) and the checks every target goes through.
//! - `capture.rs` + `encode.rs`: pixels (PrintWindow / BitBlt) → PNG or JPEG within budget (WIC).
//! - `uia.rs` → `tree.rs`: the UI Automation walk → a capped outline with refs. `logic.rs`: sizes, errors, glow.
//! - Acting: `act.rs` (UIA patterns on an element found again from a ref), `input.rs` (`SendInput`), `guard.rs`
//!   (your input aborts), `launch.rs` (links and Start-menu apps). Pure: `plan.rs`, `refs.rs`.
//!
//! TopBar's own windows are never listed, captured, read or acted on. Every look and action runs on its own MTA
//! thread with a time limit, so a hung app can't hold up the agent's connection. One action runs at a time.

pub mod logic;
pub mod plan;
pub mod refs;
pub mod tree;

mod act;
mod capture;
mod encode;
mod guard;
mod input;
mod launch;
mod uia;
mod windows;

use std::sync::{mpsc, Mutex};
use std::time::Duration;

use ::windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

pub use guard::abort as abort_action;
pub use logic::{LookError, Rect};
pub use plan::{ActError, Action};
pub use windows::{idle_ms, screen_locked, Target, WindowEntry};

use crate::win32;
use tree::{Caps, Cut};

/// A look that takes longer than this is abandoned (its thread finishes on its own).
const LOOK_TIMEOUT: Duration = Duration::from_secs(8);
/// An action (including its waits for you to let go and for another action to finish) gets this long; then
/// it's aborted.
const ACT_TIMEOUT: Duration = Duration::from_secs(90);

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
        /// In ref order, for the action tools (2.7d).
        elements: Vec<plan::Element>,
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
                elements: tree::elements(&nodes),
            })
        }
    }
}

/// Where an action is: waiting for you to let go, or sending input / calling the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Waiting,
    Acting,
}

/// What an action did, for its reply.
pub struct Acted {
    /// "Invoked button "Save"", "Typed 42 characters".
    pub what: String,
    /// The app didn't confirm in time (it may have opened a dialog), or a set value didn't stick.
    pub note: Option<String>,
    pub target: Option<Target>,
    /// The window in front afterwards (`None` when it's the desktop or one of TopBar's).
    pub foreground: Option<WindowEntry>,
}

/// Runs an approved action on `hwnd` (none for `launch`). `expect_pid` is the target's process at approval time.
/// `focus` brings a window to the front (on the main thread); `phase` is told when the action stops waiting
/// for you and starts. Only one action runs at a time; others wait their turn.
pub fn act(
    hwnd: Option<i64>,
    expect_pid: Option<u32>,
    action: Action,
    focus: impl Fn(isize) -> Result<(), ActError> + Send + 'static,
    phase: impl Fn(Phase) + Send + 'static,
) -> Result<Acted, ActError> {
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("desktop-act".into())
        .spawn(move || {
            static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
            let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
            // SAFETY: COM for this thread only (UIA, the shell); balanced below.
            let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
            let result = act_now(hwnd, expect_pid, action, &focus, &phase);
            if com {
                // SAFETY: balances the CoInitializeEx above; every COM object is dropped by now.
                unsafe { CoUninitialize() };
            }
            let _ = tx.send(result);
        });
    if let Err(e) = spawned {
        return Err(ActError::Failed(e.to_string()));
    }
    rx.recv_timeout(ACT_TIMEOUT).unwrap_or_else(|_| {
        guard::abort();
        Err(ActError::Failed(
            "it took too long, so TopBar stopped it".into(),
        ))
    })
}

fn act_now(
    hwnd: Option<i64>,
    expect_pid: Option<u32>,
    action: Action,
    focus: &dyn Fn(isize) -> Result<(), ActError>,
    phase: &dyn Fn(Phase),
) -> Result<Acted, ActError> {
    if windows::screen_locked() {
        return Err(ActError::ScreenLocked);
    }
    let target = match hwnd {
        Some(h) => Some(windows::check(h, expect_pid)?),
        None => None,
    };
    let need = || target.clone().ok_or(ActError::WindowGone);

    let guard = guard::Guard::start()?;
    phase(Phase::Waiting);
    guard.wait_quiet()?;
    phase(Phase::Acting);
    if let Some(t) = &target {
        // Again after the wait: it may have closed (or stopped responding) meanwhile.
        windows::check(t.hwnd as i64, Some(t.pid))?;
    }
    let aborted = || {
        if guard.aborted() {
            Err(ActError::Aborted { done: 0, total: 1 })
        } else {
            Ok(())
        }
    };
    let front = |t: &Target| -> Result<(), ActError> {
        if !input::is_front(t.hwnd) {
            focus(t.hwnd)?;
            std::thread::sleep(Duration::from_millis(150));
            if !input::is_front(t.hwnd) {
                return Err(ActError::NotForeground);
            }
        }
        Ok(())
    };
    let automation = || uia::automation().map_err(|e| ActError::Failed(e.message().to_string()));

    let mut note = None;
    let what = match action {
        Action::Focus => {
            let t = need()?;
            front(&t)?;
            "Brought it to the front".to_string()
        }
        Action::Invoke { element, verb } => {
            let t = need()?;
            let uia = automation()?;
            let e = act::find(&uia, t.hwnd, t.pid, &element)?;
            aborted()?;
            let done = act::invoke(&e, &element, verb)?;
            if done.unconfirmed {
                note = Some(
                    "The app didn't confirm in time; it may have opened a dialog. Look again."
                        .into(),
                );
            }
            done.what
        }
        Action::SetValue { element, value } => {
            let t = need()?;
            plan::check_text(&value, true)?;
            let uia = automation()?;
            let e = act::find(&uia, t.hwnd, t.pid, &element)?;
            aborted()?;
            let done = act::set_value(&e, &element, &value)?;
            if done.unconfirmed {
                note = Some("The app didn't confirm in time. Look again to check.".into());
            } else if done.value_matches == Some(false) {
                note = Some(
                    "The field doesn't hold that value now (the app may have changed or refused it). \
                     If it must be typed, focus it and use type_text."
                        .into(),
                );
            }
            done.what
        }
        Action::Click { at, button } => {
            let t = need()?;
            let frame = || windows::frame_bounds(win32::from_raw(t.hwnd));
            let (x, y) = match &at {
                plan::ClickAt::Element(element) => {
                    let uia = automation()?;
                    let e = act::find(&uia, t.hwnd, t.pid, element)?;
                    front(&t)?;
                    act::center(&e)?
                }
                plan::ClickAt::Point { x, y } => {
                    front(&t)?;
                    (*x, *y)
                }
                plan::ClickAt::Screenshot { x, y, view } => {
                    front(&t)?;
                    if frame() != view.frame {
                        return Err(ActError::StaleView);
                    }
                    view.screen_point(*x, *y)
                }
            };
            if !plan::contains(&frame(), x, y) {
                return Err(ActError::Covered);
            }
            input::click(&guard, t.hwnd, x, y, button)?;
            let verb = match button {
                plan::Button::Left => "Clicked",
                plan::Button::Right => "Right-clicked",
                plan::Button::Double => "Double-clicked",
            };
            format!("{verb} at {x}, {y}")
        }
        Action::TypeText { text } => {
            let t = need()?;
            let units = plan::type_units(&text)?;
            front(&t)?;
            let uia = automation()?;
            let into = match act::focused(&uia) {
                Some((true, _)) => return Err(ActError::PasswordField),
                Some((false, label)) => format!(" into {label}"),
                None => String::new(),
            };
            input::type_units(&guard, t.hwnd, &units, || match act::focused(&uia) {
                Some((true, _)) => Err(ActError::PasswordField),
                _ => Ok(()),
            })?;
            format!("Typed {} characters{into}", text.chars().count())
        }
        Action::PressKeys { chords } => {
            let t = need()?;
            front(&t)?;
            input::press(&guard, t.hwnd, &chords)?;
            let keys: Vec<String> = chords.iter().map(plan::Chord::label).collect();
            format!("Pressed {}", keys.join(", then "))
        }
        Action::Launch(plan::LaunchTarget::Url(url)) => {
            let url = plan::launch_url(&url)?;
            aborted()?;
            launch::open_url(&url)?;
            format!("Opened {url}")
        }
        Action::Launch(plan::LaunchTarget::App(name)) => {
            let apps = launch::apps();
            let app = launch::find(&apps, &name)?;
            aborted()?;
            launch::open_app(app)?;
            format!("Started {}", app.name)
        }
    };
    drop(guard);
    Ok(Acted {
        what,
        note,
        target,
        foreground: windows::focused().ok().flatten(),
    })
}

/// The monitor an action is on (for the glow): the window's, or the primary one.
pub fn monitor_of(hwnd: Option<i64>) -> Rect {
    windows::monitor_rect(hwnd.map(|h| h as isize))
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
