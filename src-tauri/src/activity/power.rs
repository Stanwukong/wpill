//! The `power-watcher` thread: charger plugged/unplugged and low battery.
//!
//! `RegisterPowerSettingNotification` delivers to a window, so this thread owns a message-only window
//! (never the island's HWND, which must stay untouched) and pumps it. Registrations and the window are
//! guards; `WM_CLOSE` ends the loop, the notifications are unregistered, then the window is destroyed.

use std::cell::RefCell;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use windows::core::{w, Result, GUID, PCWSTR};
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{
    RegisterPowerSettingNotification, UnregisterPowerSettingNotification, HPOWERNOTIFY,
    POWERBROADCAST_SETTING,
};
use windows::Win32::System::SystemServices::{
    GUID_ACDC_POWER_SOURCE, GUID_BATTERY_PERCENTAGE_REMAINING,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, PostMessageW,
    PostQuitMessage, RegisterClassW, DEVICE_NOTIFY_WINDOW_HANDLE, HWND_MESSAGE, MSG,
    PBT_POWERSETTINGCHANGE, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE, WM_POWERBROADCAST, WNDCLASSW,
};

use super::{Signal, Watcher};
use crate::error::AppResult;
use crate::ipc::{Activity, PowerActivity, PowerEvent};
use crate::win32;

const CLASS: PCWSTR = w!("TopBarPowerWatcher");
/// Crossing one of these (downwards, on battery) raises a low-battery notice.
const LOW_THRESHOLDS: [u8; 2] = [20, 10];

pub fn start(out: Sender<Signal>) -> AppResult<Watcher> {
    let (hwnd_tx, hwnd_rx) = mpsc::channel::<isize>();
    Watcher::spawn(
        "power-watcher",
        move || {
            if let Err(e) = run(out, hwnd_tx) {
                log::warn!("power activities unavailable: {e}");
            }
        },
        move || stop(&hwnd_rx),
    )
}

fn stop(hwnd: &Receiver<isize>) {
    // The window was sent right after it was created; if it never was, the thread already ended.
    if let Ok(hwnd) = hwnd.recv_timeout(Duration::from_millis(100)) {
        // SAFETY: posting to our own window; a destroyed window just makes this fail.
        let _ =
            unsafe { PostMessageW(Some(win32::from_raw(hwnd)), WM_CLOSE, WPARAM(0), LPARAM(0)) };
    }
}

/// Battery state seen by the window procedure (which runs on this thread only).
struct State {
    out: Sender<Signal>,
    on_ac: Option<bool>,
    level: Option<u8>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

struct MessageWindow(HWND);

impl Drop for MessageWindow {
    fn drop(&mut self) {
        // SAFETY: destroys the window this thread created.
        let _ = unsafe { DestroyWindow(self.0) };
    }
}

struct PowerNotification(HPOWERNOTIFY);

impl PowerNotification {
    fn register(hwnd: HWND, setting: &GUID) -> Result<Self> {
        // SAFETY: hwnd is our message-only window; setting outlives the call.
        let handle = unsafe {
            RegisterPowerSettingNotification(HANDLE(hwnd.0), setting, DEVICE_NOTIFY_WINDOW_HANDLE)?
        };
        Ok(Self(handle))
    }
}

impl Drop for PowerNotification {
    fn drop(&mut self) {
        // SAFETY: unregisters the handle returned by RegisterPowerSettingNotification.
        if let Err(e) = unsafe { UnregisterPowerSettingNotification(self.0) } {
            log::debug!("removing a power notification failed: {e}");
        }
    }
}

fn run(out: Sender<Signal>, hwnd_tx: Sender<isize>) -> Result<()> {
    STATE.set(Some(State {
        out,
        on_ac: None,
        level: None,
    }));
    // SAFETY: standard window-class registration and creation on this thread; the guards below release
    // everything in reverse order once the message loop ends.
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassW(&class);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            CLASS,
            w!(""),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(instance.into()),
            None,
        )?;
        let _window = MessageWindow(hwnd);
        // Windows sends each setting's current value right away; those are the baseline.
        let _ac = PowerNotification::register(hwnd, &GUID_ACDC_POWER_SOURCE)?;
        let _battery = PowerNotification::register(hwnd, &GUID_BATTERY_PERCENTAGE_REMAINING)?;
        let _ = hwnd_tx.send(win32::raw(hwnd));

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            DispatchMessageW(&msg);
        }
    }
    log::info!("power watcher stopped");
    Ok(())
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_POWERBROADCAST if wparam.0 == PBT_POWERSETTINGCHANGE as usize => {
            // SAFETY: for PBT_POWERSETTINGCHANGE, lparam points at a POWERBROADCAST_SETTING that is valid
            // for this call, followed by `DataLength` bytes of data.
            let (setting, value) = unsafe {
                let s = &*(lparam.0 as *const POWERBROADCAST_SETTING);
                if s.DataLength < 4 {
                    return LRESULT(1);
                }
                (
                    s.PowerSetting,
                    (s.Data.as_ptr() as *const u32).read_unaligned(),
                )
            };
            STATE.with_borrow_mut(|state| {
                if let Some(state) = state {
                    state.on_setting(setting, value);
                }
            });
            LRESULT(1)
        }
        WM_CLOSE => {
            // SAFETY: ends this thread's message loop; the guards in `run` clean up.
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        // SAFETY: default handling for everything else.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

impl State {
    fn on_setting(&mut self, setting: GUID, value: u32) {
        let event = if setting == GUID_ACDC_POWER_SOURCE {
            // 0 = AC, 1 = battery, 2 = short-term source (UPS): treat that as battery.
            let on_ac = value == 0;
            let previous = self.on_ac.replace(on_ac);
            match previous {
                Some(was) if was != on_ac => Some(if on_ac {
                    PowerEvent::PluggedIn
                } else {
                    PowerEvent::Unplugged
                }),
                _ => None,
            }
        } else if setting == GUID_BATTERY_PERCENTAGE_REMAINING {
            let level = value.min(100) as u8;
            let previous = self.level.replace(level);
            (self.on_ac == Some(false) && crossed_low(previous, level)).then_some(PowerEvent::Low)
        } else {
            None
        };
        // No battery level means no battery (a desktop): nothing worth showing.
        if let (Some(event), Some(level)) = (event, self.level) {
            log::debug!("power: {event:?} at {level}%");
            let _ = self
                .out
                .send(Activity::Power(PowerActivity { event, level }).into());
        }
    }
}

/// True when the level drops to or below a threshold it was above before.
fn crossed_low(previous: Option<u8>, level: u8) -> bool {
    let Some(previous) = previous else {
        return false;
    };
    LOW_THRESHOLDS.iter().any(|&t| previous > t && level <= t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_battery_fires_once_per_threshold_going_down() {
        assert!(!crossed_low(None, 5));
        assert!(crossed_low(Some(21), 20));
        assert!(!crossed_low(Some(20), 19));
        assert!(crossed_low(Some(11), 10));
        assert!(crossed_low(Some(25), 8));
        assert!(!crossed_low(Some(9), 30));
    }

    #[test]
    fn plug_events_need_a_baseline_and_a_battery() {
        let (tx, rx) = mpsc::channel();
        let mut state = State {
            out: tx,
            on_ac: None,
            level: None,
        };
        state.on_setting(GUID_ACDC_POWER_SOURCE, 0);
        state.on_setting(GUID_BATTERY_PERCENTAGE_REMAINING, 32);
        assert!(rx.try_recv().is_err());
        state.on_setting(GUID_ACDC_POWER_SOURCE, 1);
        let Ok(Signal::Offer(Activity::Power(p))) = rx.try_recv() else {
            panic!("expected an unplugged event");
        };
        assert_eq!((p.event, p.level), (PowerEvent::Unplugged, 32));
        state.on_setting(GUID_BATTERY_PERCENTAGE_REMAINING, 20);
        assert!(
            matches!(rx.try_recv(), Ok(Signal::Offer(Activity::Power(p))) if p.event == PowerEvent::Low)
        );
    }
}
