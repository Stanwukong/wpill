//! Global hotkeys on the `agent-hotkeys` thread. `RegisterHotKey` without a window posts `WM_HOTKEY` to the
//! registering thread's queue, so no window is needed (not even a message-only one), and the island's HWND is
//! never involved. `WM_QUIT` (posted by the watcher's stop) ends the loop; hotkeys are unregistered on the way out.

use std::sync::mpsc;
use std::time::Duration;

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey};
use windows::Win32::UI::WindowsAndMessaging::{
    GetMessageW, PeekMessageW, PostThreadMessageW, MSG, PM_NOREMOVE, WM_HOTKEY, WM_QUIT, WM_USER,
};

use super::settings::parse_hotkey;
use crate::activity::Watcher;
use crate::error::AppResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hotkey {
    OpenCommand,
    StopAll,
}

const ALL: [Hotkey; 2] = [Hotkey::OpenCommand, Hotkey::StopAll];

/// Starts the thread and returns the watcher plus a message per hotkey that couldn't be registered.
pub fn start(
    open: String,
    stop_all: String,
    on: impl Fn(Hotkey) + Send + 'static,
) -> AppResult<(Watcher, Vec<String>)> {
    let (ready_tx, ready_rx) = mpsc::channel::<(u32, Vec<String>)>();
    let (thread_tx, thread_rx) = mpsc::channel::<u32>();
    let watcher = Watcher::spawn(
        "agent-hotkeys",
        move || {
            // SAFETY: no preconditions. Peeking creates this thread's message queue before anything is posted.
            let thread = unsafe { GetCurrentThreadId() };
            let mut msg = MSG::default();
            // SAFETY: `msg` is a valid out-pointer.
            let _ = unsafe { PeekMessageW(&mut msg, None, WM_USER, WM_USER, PM_NOREMOVE) };
            let mut errors = Vec::new();
            let mut registered = Vec::new();
            for (id, (hotkey, text)) in ALL.iter().zip([&open, &stop_all]).enumerate() {
                let id = id as i32 + 1;
                let Some((mods, vk)) = parse_hotkey(text) else {
                    errors.push(format!("\"{text}\" isn't a hotkey TopBar understands."));
                    continue;
                };
                // SAFETY: registers for this thread (no window); undone below on this same thread.
                match unsafe { RegisterHotKey(None, id, mods, vk) } {
                    Ok(()) => registered.push((id, *hotkey)),
                    Err(e) => {
                        log::warn!("hotkey {text} unavailable: {e}");
                        errors.push(format!("{text} is already used by another app."));
                    }
                }
            }
            let _ = ready_tx.send((thread, errors));
            // SAFETY: `msg` is a valid out-pointer; GetMessageW returns 0 on WM_QUIT and -1 on error.
            while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
                if msg.message == WM_HOTKEY {
                    if let Some((_, hotkey)) =
                        registered.iter().find(|(id, _)| *id == msg.wParam.0 as i32)
                    {
                        on(*hotkey);
                    }
                }
            }
            for (id, _) in registered {
                // SAFETY: unregisters what this thread registered.
                let _ = unsafe { UnregisterHotKey(None, id) };
            }
        },
        move || {
            if let Ok(thread) = thread_rx.recv_timeout(Duration::from_millis(100)) {
                // SAFETY: posting WM_QUIT to our own thread; fails harmlessly if it already ended.
                let _ = unsafe { PostThreadMessageW(thread, WM_QUIT, WPARAM(0), LPARAM(0)) };
            }
        },
    )?;
    let (thread, errors) = ready_rx
        .recv_timeout(Duration::from_secs(2))
        .unwrap_or((0, vec!["The hotkeys couldn't be registered.".into()]));
    let _ = thread_tx.send(thread);
    Ok((watcher, errors))
}
