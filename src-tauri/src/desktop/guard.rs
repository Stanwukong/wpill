//! Your hands come first (Phase 2.7d): while an action runs, low-level keyboard and mouse hooks watch for input
//! that isn't injected (`LLKHF_INJECTED` / `LLMHF_INJECTED` unset, so it came from a real device). Before acting,
//! TopBar waits until you've let go for `plan::QUIET`; once it's acting, any input of yours aborts it before its
//! next event. The hooks live on their own thread with a message loop, only while an action runs; their callback
//! only stores a timestamp and a flag, and always passes the input on (nothing is swallowed).
//!
//! One action runs at a time (the lock in `mod.rs`), so the state is a few statics.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, PeekMessageW, PostThreadMessageW, SetWindowsHookExW,
    UnhookWindowsHookEx, KBDLLHOOKSTRUCT, LLKHF_INJECTED, LLKHF_LOWER_IL_INJECTED, LLMHF_INJECTED,
    LLMHF_LOWER_IL_INJECTED, MSG, MSLLHOOKSTRUCT, PM_NOREMOVE, WH_KEYBOARD_LL, WH_MOUSE_LL,
    WM_QUIT, WM_USER,
};

use super::plan::{ActError, QUIET, QUIET_MAX};

/// `GetTickCount64` of your latest input.
static LAST_INPUT: AtomicU64 = AtomicU64::new(0);
/// Once armed, your input aborts.
static ARMED: AtomicBool = AtomicBool::new(false);
static ABORTED: AtomicBool = AtomicBool::new(false);

fn ticks() -> u64 {
    // SAFETY: no preconditions.
    unsafe { GetTickCount64() }
}

fn yours() {
    LAST_INPUT.store(ticks(), Ordering::Relaxed);
    if ARMED.load(Ordering::Relaxed) {
        ABORTED.store(true, Ordering::Relaxed);
    }
}

unsafe extern "system" fn keyboard(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        // SAFETY: for WH_KEYBOARD_LL with code >= 0, lparam points to a KBDLLHOOKSTRUCT.
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        if (info.flags.0 & (LLKHF_INJECTED.0 | LLKHF_LOWER_IL_INJECTED.0)) == 0 {
            yours();
        }
    }
    // SAFETY: passes the event on unchanged.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn mouse(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        // SAFETY: for WH_MOUSE_LL with code >= 0, lparam points to an MSLLHOOKSTRUCT.
        let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        if (info.flags & (LLMHF_INJECTED | LLMHF_LOWER_IL_INJECTED)) == 0 {
            yours();
        }
    }
    // SAFETY: passes the event on unchanged.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Stops the current action from outside (Stop, the stop hotkey, a cancelled call, a timeout).
pub fn abort() {
    ABORTED.store(true, Ordering::Relaxed);
}

/// The hooks, installed for one action. Dropping it removes them.
pub struct Guard {
    thread: u32,
    join: Option<std::thread::JoinHandle<()>>,
}

impl Guard {
    pub fn start() -> Result<Guard, ActError> {
        ARMED.store(false, Ordering::Relaxed);
        ABORTED.store(false, Ordering::Relaxed);
        // You just clicked Allow: count that as your latest input, so the quiet wait starts now.
        LAST_INPUT.store(ticks(), Ordering::Relaxed);
        let (tx, rx) = mpsc::channel::<Result<u32, String>>();
        let join = std::thread::Builder::new()
            .name("input-guard".into())
            .spawn(move || {
                // SAFETY: hooks installed and removed on this thread, which pumps messages so they're called.
                unsafe {
                    let mut msg = MSG::default();
                    let _ = PeekMessageW(&mut msg, None, WM_USER, WM_USER, PM_NOREMOVE);
                    let kb = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard), None, 0);
                    let ms = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse), None, 0);
                    let (kb, ms) = match (kb, ms) {
                        (Ok(kb), Ok(ms)) => (kb, ms),
                        (kb, ms) => {
                            for h in [kb.ok(), ms.ok()].into_iter().flatten() {
                                let _ = UnhookWindowsHookEx(h);
                            }
                            let _ = tx.send(Err("the input hooks couldn't be installed".into()));
                            return;
                        }
                    };
                    let _ = tx.send(Ok(GetCurrentThreadId()));
                    while GetMessageW(&mut msg, None, 0, 0).0 > 0 {}
                    let _ = UnhookWindowsHookEx(kb);
                    let _ = UnhookWindowsHookEx(ms);
                }
            })
            .map_err(|e| ActError::Failed(e.to_string()))?;
        match rx.recv_timeout(Duration::from_secs(2)) {
            Ok(Ok(thread)) => Ok(Guard {
                thread,
                join: Some(join),
            }),
            Ok(Err(why)) => Err(ActError::Failed(why)),
            Err(_) => Err(ActError::Failed("the input hooks didn't start".into())),
        }
    }

    /// Waits until you've let go for `QUIET`, then arms: from then on your input aborts. `Busy` if you're still
    /// at it after `QUIET_MAX`, `Aborted` if stopped meanwhile.
    pub fn wait_quiet(&self) -> Result<(), ActError> {
        let started = Instant::now();
        loop {
            if ABORTED.load(Ordering::Relaxed) {
                return Err(ActError::Aborted { done: 0, total: 1 });
            }
            let idle = ticks().saturating_sub(LAST_INPUT.load(Ordering::Relaxed));
            if idle >= QUIET.as_millis() as u64 {
                ARMED.store(true, Ordering::Relaxed);
                return Ok(());
            }
            if started.elapsed() >= QUIET_MAX {
                return Err(ActError::Busy);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// Whether you (or Stop) interrupted since arming.
    pub fn aborted(&self) -> bool {
        ABORTED.load(Ordering::Relaxed)
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        ARMED.store(false, Ordering::Relaxed);
        // SAFETY: ends our own hook thread's message loop; it unhooks on the way out.
        let _ = unsafe { PostThreadMessageW(self.thread, WM_QUIT, WPARAM(0), LPARAM(0)) };
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
