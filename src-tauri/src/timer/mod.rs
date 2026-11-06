//! Timer, stopwatch and focus (Phase 5a, D56). One timer at a time; `logic.rs` holds the rules.
//!
//! The `timer` thread parks on a condvar while nothing runs. While a timer runs it wakes on each whole second (and at
//! the deadline), emits `timer:state` and, at the end, offers the `timerDone` activity. Commands change the timer,
//! save it to `timer.json` in Cay's data folder (so a restart picks it back up), emit and wake the thread. React
//! draws the latest `timer:state` and runs no clock.

mod logic;

use std::path::PathBuf;
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Emitter, Manager, State};
use windows::core::w;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::activity::{self, Activities, Signal, Watcher};
use crate::error::{AppError, AppResult};
use crate::ipc::{events, Activity, TimerDoneActivity, TimerKind, TimerPayload, TimerPhase};
use logic::{Timer, TimerError};

const FILE: &str = "timer.json";

#[derive(Default)]
struct Inner {
    timer: Option<Timer>,
    stop: bool,
    /// Bumped by every command, so the thread recomputes its wait.
    changed: u64,
}

/// Tauri-managed state shared by the thread and the commands.
#[derive(Default)]
pub struct Timers {
    inner: Mutex<Inner>,
    wake: Condvar,
}

impl Timers {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn payload(&self, now: i64) -> TimerPayload {
        TimerPayload {
            timer: self.lock().timer.as_ref().map(|t| t.state(now)),
        }
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

fn path(app: &AppHandle) -> Option<PathBuf> {
    Some(app.path().app_config_dir().ok()?.join(FILE))
}

fn load(app: &AppHandle, now: i64) -> Option<Timer> {
    let path = path(app)?;
    let text = std::fs::read_to_string(&path).ok()?;
    let restored = serde_json::from_str(&text)
        .ok()
        .and_then(|t| logic::restore(t, now));
    if restored.is_none() {
        log::info!("dropping the saved timer (stale or unreadable)");
        let _ = std::fs::remove_file(&path);
    }
    restored
}

/// Saves the timer, or removes the file when there's none. A failure only costs the restore after a restart.
fn save(app: &AppHandle, timer: Option<&Timer>) {
    let Some(path) = path(app) else { return };
    let result = match timer {
        Some(t) => serde_json::to_string(t)
            .map_err(std::io::Error::other)
            .and_then(|json| std::fs::write(&path, json)),
        None => match std::fs::remove_file(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        },
    };
    if let Err(e) = result {
        log::debug!("saving {} failed: {e}", path.display());
    }
}

fn emit(app: &AppHandle, payload: &TimerPayload) {
    if let Err(e) = app.emit(events::TIMER_STATE, payload) {
        log::debug!("timer:state emit failed: {e}");
    }
}

fn offer_done(activities: &Activities, timer: &Timer, now: i64) {
    let Some(ended_at_ms) = timer.ended_ms else {
        return;
    };
    let done = Activity::TimerDone(TimerDoneActivity {
        kind: timer.kind,
        ended_at_ms,
        late: timer.late(now),
    });
    let _ = activities.sender().send(Signal::Offer(done));
}

fn retract_done(activities: &Activities) {
    let _ = activities
        .sender()
        .send(Signal::Retract(activity::TIMER_KEY.into()));
}

pub fn spawn(app: &AppHandle, activities: &Activities) {
    let now = now_ms();
    let restored = load(app, now);
    if let Some(t) = &restored {
        log::info!("timer restored: {:?} {:?}", t.kind, t.phase());
        if t.phase() == TimerPhase::Ended {
            save(app, Some(t));
            offer_done(activities, t, now);
        }
    }
    app.state::<Timers>().lock().timer = restored;
    let body_app = app.clone();
    let stop_app = app.clone();
    let watcher = Watcher::spawn(
        "timer",
        move || run(&body_app),
        move || {
            let timers = stop_app.state::<Timers>();
            timers.lock().stop = true;
            timers.wake.notify_all();
        },
    );
    match watcher {
        Ok(w) => activities.add_watcher(w),
        Err(e) => log::warn!("timer thread failed to start: {e}"),
    }
}

fn run(app: &AppHandle) {
    let timers = app.state::<Timers>();
    let activities = app.state::<Activities>();
    let mut guard = timers.lock();
    loop {
        if guard.stop {
            break;
        }
        let now = now_ms();
        let mut ended = None;
        let mut running = false;
        if let Some(timer) = guard.timer.as_mut() {
            if timer.tick(now) {
                ended = Some(timer.clone());
            }
            running = timer.phase() == TimerPhase::Running;
        }
        let wait = guard.timer.as_ref().and_then(|t| t.next_wake_ms(now));
        let seen = guard.changed;
        if running || ended.is_some() {
            let payload = TimerPayload {
                timer: guard.timer.as_ref().map(|t| t.state(now)),
            };
            drop(guard);
            if let Some(timer) = &ended {
                log::debug!("timer ended ({:?}, late: {})", timer.kind, timer.late(now));
                save(app, Some(timer));
                offer_done(&activities, timer, now);
            }
            emit(app, &payload);
            guard = timers.lock();
            if guard.changed != seen {
                // A command came in while the lock was free: look again before waiting.
                continue;
            }
        }
        guard = match wait {
            Some(ms) => {
                timers
                    .wake
                    .wait_timeout_while(guard, Duration::from_millis(ms as u64), |s| {
                        !s.stop && s.changed == seen
                    })
                    .unwrap_or_else(|e| e.into_inner())
                    .0
            }
            None => timers
                .wake
                .wait_while(guard, |s| !s.stop && s.changed == seen)
                .unwrap_or_else(|e| e.into_inner()),
        };
    }
}

/// Applies `change` to the timer, saves it, wakes the thread and emits the new state.
fn update(
    app: &AppHandle,
    timers: &Timers,
    change: impl FnOnce(&mut Option<Timer>, i64) -> Result<(), TimerError>,
) -> AppResult<TimerPayload> {
    let now = now_ms();
    let saved = {
        let mut inner = timers.lock();
        change(&mut inner.timer, now).map_err(|e| AppError::Timer(e.to_string()))?;
        inner.changed = inner.changed.wrapping_add(1);
        inner.timer.clone()
    };
    timers.wake.notify_all();
    save(app, saved.as_ref());
    let payload = timers.payload(now);
    emit(app, &payload);
    Ok(payload)
}

fn current(timer: &mut Option<Timer>) -> Result<&mut Timer, TimerError> {
    timer.as_mut().ok_or(TimerError::NotRunning)
}

/// Starts a timer, replacing the one that runs (and its "done" activity).
#[tauri::command]
pub fn timer_start(
    app: AppHandle,
    timers: State<'_, Timers>,
    activities: State<'_, Activities>,
    kind: TimerKind,
    minutes: Option<u32>,
) -> AppResult<TimerPayload> {
    let payload = update(&app, &timers, |timer, now| {
        *timer = Some(Timer::start(kind, minutes, now)?);
        Ok(())
    })?;
    retract_done(&activities);
    log::debug!("timer started: {kind:?}");
    Ok(payload)
}

#[tauri::command]
pub fn timer_pause(app: AppHandle, timers: State<'_, Timers>) -> AppResult<TimerPayload> {
    update(&app, &timers, |timer, now| current(timer)?.pause(now))
}

#[tauri::command]
pub fn timer_resume(app: AppHandle, timers: State<'_, Timers>) -> AppResult<TimerPayload> {
    update(&app, &timers, |timer, now| current(timer)?.resume(now))
}

/// One more minute; on an ended countdown it runs again with a minute left.
#[tauri::command]
pub fn timer_add_minute(
    app: AppHandle,
    timers: State<'_, Timers>,
    activities: State<'_, Activities>,
) -> AppResult<TimerPayload> {
    let payload = update(&app, &timers, |timer, now| current(timer)?.add_minute(now))?;
    retract_done(&activities);
    Ok(payload)
}

/// Cancels the timer, or dismisses an ended one.
#[tauri::command]
pub fn timer_cancel(
    app: AppHandle,
    timers: State<'_, Timers>,
    activities: State<'_, Activities>,
) -> AppResult<TimerPayload> {
    let payload = update(&app, &timers, |timer, _| {
        *timer = None;
        Ok(())
    })?;
    retract_done(&activities);
    Ok(payload)
}

#[tauri::command]
pub fn get_timer(timers: State<'_, Timers>) -> TimerPayload {
    timers.payload(now_ms())
}

/// Opens Windows' Notifications settings, where Do not disturb is. The user switches it; Cay can't (no public API).
#[tauri::command]
pub fn timer_open_dnd() -> AppResult<()> {
    // SAFETY: static NUL-terminated wide strings; ShellExecuteW returns at once.
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            w!("ms-settings:notifications"),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    // Values above 32 mean success.
    if result.0 as usize <= 32 {
        return Err(AppError::Win32(windows::core::Error::from_thread()));
    }
    Ok(())
}
