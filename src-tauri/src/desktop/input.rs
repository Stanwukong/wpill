//! Synthetic input (Phase 2.7d), the fallback when UI Automation can't do it: clicks, typed text and key chords
//! through `SendInput`. Input goes to whatever has focus, so before every batch the target must still be the
//! foreground window, the screen unlocked and you not interrupting; keys held down are always released, even
//! when stopped part-way.

use std::time::Duration;

use ::windows::Win32::Foundation::POINT;
use ::windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, GetKeyboardLayout, MapVirtualKeyExW, SendInput, VkKeyScanExW, HKL, INPUT, INPUT_0,
    INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MAPVK_VK_TO_VSC, MOUSEEVENTF_ABSOLUTE,
    MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN,
    MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT, MOUSE_EVENT_FLAGS, VIRTUAL_KEY,
    VK_CAPITAL,
};
use ::windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetForegroundWindow, GetSystemMetrics, GetWindowThreadProcessId, WindowFromPoint,
    GA_ROOT, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

use super::guard::Guard;
use super::logic::Rect;
use super::plan::{self, ActError, Button, Chord, Key, Unit};
use super::windows;
use crate::win32;

/// Between batches, so the app keeps up and your input gets a chance to abort.
const PAUSE: Duration = Duration::from_millis(12);
/// Between key events (Shift down, key down, key up, Shift up): XAML apps (Windows 11 Notepad) drop keys, lose
/// Shift and reorder events that arrive close together (C11). About 90 ms a character, ~125 ms with Shift, so the
/// 500 `type_text` allows take about a minute.
const KEY_GAP: Duration = Duration::from_millis(25);
/// After a modifier goes down, longer: XAML apps read it late, and an early key comes out unshifted.
const MODIFIER_SETTLE: Duration = Duration::from_millis(60);
/// After a character sent as `VK_PACKET`, longer: see `type_units`.
const PACKET_PAUSE: Duration = Duration::from_millis(40);

/// Sends `events` one at a time, `KEY_GAP` apart. If one fails, every key that went down is released.
fn send_paced(events: &[INPUT]) -> Result<(), ActError> {
    for (i, event) in events.iter().enumerate() {
        if let Err(e) = send(std::slice::from_ref(event)) {
            release(&events[..i]);
            return Err(e);
        }
        std::thread::sleep(if is_modifier_down(event) {
            MODIFIER_SETTLE
        } else {
            KEY_GAP
        });
    }
    Ok(())
}

/// Shift, Ctrl or Alt going down.
fn is_modifier_down(event: &INPUT) -> bool {
    // SAFETY: the keyboard arm is read only for keyboard input.
    event.r#type == INPUT_KEYBOARD && {
        let k = unsafe { event.Anonymous.ki };
        (k.dwFlags & KEYEVENTF_KEYUP).0 == 0
            && [plan::SHIFT, plan::CTRL, plan::ALT]
                .iter()
                .any(|m| m.vk == k.wVk.0)
    }
}

/// Key-ups for every key-down in `sent` (best effort).
fn release(sent: &[INPUT]) {
    // SAFETY: reading the keyboard arm of INPUTs we built as keyboard input.
    let ups: Vec<INPUT> = sent
        .iter()
        .filter(|i| i.r#type == INPUT_KEYBOARD)
        .map(|i| unsafe { i.Anonymous.ki })
        .filter(|k| (k.dwFlags & KEYEVENTF_KEYUP).0 == 0)
        .map(|k| key_event(k.wVk.0, k.wScan, k.dwFlags | KEYEVENTF_KEYUP))
        .collect();
    let _ = send(&ups);
}

/// Everything that must still hold before the next event.
fn ready(guard: &Guard, target: isize, done: usize, total: usize) -> Result<(), ActError> {
    if guard.aborted() {
        return Err(ActError::Aborted { done, total });
    }
    if windows::screen_locked() {
        return Err(ActError::ScreenLocked);
    }
    if !is_front(target) {
        return Err(ActError::NotForeground);
    }
    Ok(())
}

/// Whether `target` (a top-level window) is the foreground window.
pub fn is_front(target: isize) -> bool {
    // SAFETY: read-only queries.
    unsafe {
        let front = GetForegroundWindow();
        !front.is_invalid() && win32::raw(GetAncestor(front, GA_ROOT)) == target
    }
}

/// The top-level window under a screen point.
pub fn root_at(x: i32, y: i32) -> isize {
    // SAFETY: read-only queries.
    unsafe { win32::raw(GetAncestor(WindowFromPoint(POINT { x, y }), GA_ROOT)) }
}

fn send(inputs: &[INPUT]) -> Result<(), ActError> {
    // SAFETY: `inputs` is a valid slice of initialised INPUTs.
    let sent = unsafe { SendInput(inputs, size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        return Err(ActError::Failed("Windows didn't accept the input".into()));
    }
    Ok(())
}

fn virtual_desk() -> Rect {
    // SAFETY: no preconditions.
    unsafe {
        let (x, y) = (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
        );
        Rect {
            x,
            y,
            width: GetSystemMetrics(SM_CXVIRTUALSCREEN),
            height: GetSystemMetrics(SM_CYVIRTUALSCREEN),
        }
    }
}

fn mouse(flags: MOUSE_EVENT_FLAGS, dx: i32, dy: i32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Moves to `(x, y)` (physical screen px) and clicks. The point must be on `target`, checked just before.
pub fn click(guard: &Guard, target: isize, x: i32, y: i32, button: Button) -> Result<(), ActError> {
    ready(guard, target, 0, 1)?;
    if root_at(x, y) != target {
        return Err(ActError::Covered);
    }
    let (nx, ny) = plan::to_absolute(x, y, virtual_desk());
    let at = MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;
    send(&[mouse(MOUSEEVENTF_MOVE | at, nx, ny)])?;
    std::thread::sleep(PAUSE);
    ready(guard, target, 0, 1)?;
    if root_at(x, y) != target {
        return Err(ActError::Covered);
    }
    let (down, up) = match button {
        Button::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
        Button::Left | Button::Double => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
    };
    let one = [mouse(down | at, nx, ny), mouse(up | at, nx, ny)];
    send(&one)?;
    if button == Button::Double {
        std::thread::sleep(Duration::from_millis(40));
        ready(guard, target, 1, 2)?;
        send(&one)?;
    }
    Ok(())
}

fn key_event(vk: u16, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn key(k: Key, up: bool) -> INPUT {
    let mut flags = if up {
        KEYEVENTF_KEYUP
    } else {
        KEYBD_EVENT_FLAGS(0)
    };
    if k.extended {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    key_event(k.vk, 0, flags)
}

/// One unit's events, and whether it went as `VK_PACKET`. A character the target's layout has a key for is typed
/// as that key (with Shift if needed, Caps Lock allowed for), like a real keyboard; anything else as Unicode.
fn unit(u: Unit, layout: HKL) -> (Vec<INPUT>, bool) {
    match u {
        Unit::Char(c) => {
            // SAFETY: plain queries on a character and our own keyboard state.
            let (scan, caps) = unsafe {
                (
                    VkKeyScanExW(c, layout),
                    GetKeyState(VK_CAPITAL.0 as i32) & 1 != 0,
                )
            };
            match plan::key_for(scan, caps) {
                Some((vk, shift)) => {
                    // SAFETY: maps a virtual key to its scan code in the given layout.
                    let sc =
                        unsafe { MapVirtualKeyExW(u32::from(vk), MAPVK_VK_TO_VSC, Some(layout)) };
                    let k = |up: bool| {
                        let flags = if up {
                            KEYEVENTF_KEYUP
                        } else {
                            KEYBD_EVENT_FLAGS(0)
                        };
                        key_event(vk, sc as u16, flags)
                    };
                    let mut inputs = Vec::with_capacity(4);
                    if shift {
                        inputs.push(key(plan::SHIFT, false));
                    }
                    inputs.extend([k(false), k(true)]);
                    if shift {
                        inputs.push(key(plan::SHIFT, true));
                    }
                    (inputs, false)
                }
                None => (
                    vec![
                        key_event(0, c, KEYEVENTF_UNICODE),
                        key_event(0, c, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP),
                    ],
                    true,
                ),
            }
        }
        Unit::Key(k) => (vec![key(k, false), key(k, true)], false),
    }
}

/// The keyboard layout of the window's thread (what it will translate keys with).
fn layout_of(target: isize) -> HKL {
    // SAFETY: read-only queries; a dead window gives thread 0, i.e. our own layout.
    unsafe { GetKeyboardLayout(GetWindowThreadProcessId(win32::from_raw(target), None)) }
}

/// Types `units`, one per `SendInput` call. Characters go as real keys where the layout has them: a
/// `KEYEVENTF_UNICODE` (`VK_PACKET`) character is only read back when the app translates the message, so in an app
/// that handles input slowly (Windows 11 Notepad) queued packets all come out as the latest one ("Hello" → "ooooo",
/// C11). Packets are kept for what the layout can't type (emoji, other scripts), each followed by a longer pause.
/// The target and your input are checked before every unit; `check` runs every `TYPE_BATCH` units (TopBar asks
/// UIA whether the focus moved into a password field). Returns how many units were typed.
pub fn type_units(
    guard: &Guard,
    target: isize,
    units: &[Unit],
    mut check: impl FnMut() -> Result<(), ActError>,
) -> Result<usize, ActError> {
    let total = units.len();
    let layout = layout_of(target);
    for (i, u) in units.iter().enumerate() {
        ready(guard, target, i, total)?;
        if i % plan::TYPE_BATCH == 0 {
            check()?;
        }
        let (inputs, packet) = unit(*u, layout);
        send_paced(&inputs)?;
        if packet {
            std::thread::sleep(PACKET_PAUSE);
        }
    }
    Ok(total)
}

/// Presses each chord in order: modifiers down, key down and up, modifiers up (always released).
pub fn press(guard: &Guard, target: isize, chords: &[Chord]) -> Result<(), ActError> {
    let total = chords.len();
    for (i, chord) in chords.iter().enumerate() {
        ready(guard, target, i, total)?;
        let mut inputs: Vec<INPUT> = chord.mods.iter().map(|m| key(*m, false)).collect();
        inputs.push(key(chord.key, false));
        inputs.push(key(chord.key, true));
        inputs.extend(chord.mods.iter().rev().map(|m| key(*m, true)));
        send_paced(&inputs)?;
        std::thread::sleep(Duration::from_millis(60));
    }
    Ok(())
}
