//! Cursor poller: owns expanded/collapsed state, activities, click-through, and fullscreen hiding.

mod logic;

use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

use crate::activity::{self, Activities};
use crate::error::AppResult;
use crate::ipc::{
    events, Activity, ActivityHidePayload, CapsLockActivity, IslandStatePayload, VisibilityPayload,
};
use crate::window::{self, Geometry};
use crate::{fullscreen, win32};
use logic::{Machine, Output, PanelRect, Sample};

const POLL_INTERVAL: Duration = Duration::from_millis(16);
const FULLSCREEN_INTERVAL: Duration = Duration::from_millis(250);

/// Which island shape a reported rect measures. Reports for the other shape are stale and ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RectMode {
    Panel,
    Activity,
}

struct Shared {
    /// Reported by React for the current panel or activity; `None` until it measures.
    rect: Option<(RectMode, PanelRect)>,
    force_collapse: bool,
    expanded: bool,
    hidden: bool,
}

/// Tauri-managed state shared between the poller thread and commands.
pub struct Island(Mutex<Shared>);

impl Default for Island {
    fn default() -> Self {
        Self(Mutex::new(Shared {
            rect: None,
            force_collapse: false,
            expanded: false,
            hidden: false,
        }))
    }
}

impl Island {
    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn request_collapse(&self) {
        self.lock().force_collapse = true;
    }
}

pub fn spawn(app: AppHandle, window: &WebviewWindow, geo: Geometry) -> AppResult<()> {
    let own = win32::raw(window.hwnd()?);
    thread::Builder::new()
        .name("hover-poller".into())
        .spawn(move || run(app, geo, own))
        .map_err(tauri::Error::from)?;
    Ok(())
}

fn run(app: AppHandle, geo: Geometry, own: isize) {
    let island = app.state::<Island>();
    let activities = app.state::<Activities>();
    let incoming = activities.take_receiver();
    let mut machine = Machine::default();
    let mut out = Vec::new();
    let mut ignoring = true;
    let mut caps_lock: Option<bool> = None;
    let mut last_fullscreen_check: Option<Instant> = None;

    loop {
        thread::sleep(POLL_INTERVAL);
        let now = Instant::now();

        if last_fullscreen_check.is_none_or(|t| now.duration_since(t) >= FULLSCREEN_INTERVAL) {
            last_fullscreen_check = Some(now);
            let fullscreen = fullscreen::foreground_is_fullscreen(win32::from_raw(own));
            // Self-heal: on a cold start Tauri can apply the builder's `visible(false)` after our
            // initial show, so the island would stay hidden.
            if !fullscreen && !window::is_visible(win32::from_raw(own)) {
                window::show_no_activate(win32::from_raw(own));
            }
            // Also self-heal the extended style in case anything (e.g. Tao) rewrote it.
            window::set_click_through(win32::from_raw(own), ignoring);
            let changed = {
                let mut s = island.lock();
                let changed = s.hidden != fullscreen;
                s.hidden = fullscreen;
                changed
            };
            if changed {
                log::info!(
                    "fullscreen app in foreground: {fullscreen}; island hidden: {fullscreen}"
                );
                if fullscreen {
                    machine.hide(&mut out);
                    flush(&app, &island, &activities, &mut out);
                    if !ignoring {
                        window::set_click_through(win32::from_raw(own), true);
                        ignoring = true;
                    }
                    window::hide(win32::from_raw(own));
                }
                let reason = fullscreen.then_some("fullscreen");
                emit(
                    &app,
                    events::VISIBILITY,
                    VisibilityPayload {
                        hidden: fullscreen,
                        reason,
                    },
                );
            }
        }

        let (rect, force, hidden) = {
            let mut s = island.lock();
            (s.rect, std::mem::take(&mut s.force_collapse), s.hidden)
        };
        let (dragging, dismiss) = activities.poll_flags();

        // Activities arrive even while hidden: notices wait (and may go stale), feedback is dropped.
        let offer = |machine: &mut Machine, activity: Activity, out: &mut Vec<Output>| {
            if activities.allows(&activity) {
                machine.offer(activity, now, hidden, out);
            }
        };
        if let Some(rx) = &incoming {
            for activity in rx.try_iter() {
                offer(&mut machine, activity, &mut out);
            }
        }
        let on = activity::caps_lock_on();
        if caps_lock.replace(on).is_some_and(|was| was != on) {
            offer(
                &mut machine,
                Activity::CapsLock(CapsLockActivity { on }),
                &mut out,
            );
        }
        if let Some(id) = dismiss {
            machine.dismiss(id, &mut out);
        }
        if hidden {
            flush(&app, &island, &activities, &mut out);
            continue;
        }

        let mut cursor = POINT::default();
        // SAFETY: cursor is a valid out-pointer. Fails e.g. on the secure desktop; skip the tick.
        if unsafe { GetCursorPos(&mut cursor) }.is_err() {
            flush(&app, &island, &activities, &mut out);
            continue;
        }

        let measured = |mode: RectMode| rect.filter(|(m, _)| *m == mode).map(|(_, r)| r);
        let panel = measured(RectMode::Panel).unwrap_or_else(|| PanelRect::default_for(&geo));
        let activity_rect =
            measured(RectMode::Activity).unwrap_or_else(|| PanelRect::default_activity(&geo));
        let sample = Sample {
            now,
            in_hot_zone: logic::in_hot_zone(&geo, cursor.x, cursor.y),
            in_panel: logic::in_panel(&geo, panel, cursor.x, cursor.y),
            in_activity: logic::in_panel(&geo, activity_rect, cursor.x, cursor.y),
            dragging,
        };
        let before = machine.phase;
        if force {
            machine.force_collapse(&mut out);
        } else {
            machine.tick(&sample, &mut out);
        }
        if std::mem::discriminant(&machine.phase) != std::mem::discriminant(&before) {
            log::debug!(
                "hover {before:?} -> {:?} at ({}, {})",
                machine.phase,
                cursor.x,
                cursor.y
            );
        }

        // Accept clicks only while the cursor is over the panel or an interactive activity, so the
        // transparent margins (and the panel during its leave delay) stay click-through.
        let want_ignore = !machine.accepts_clicks();
        if want_ignore != ignoring {
            window::set_click_through(win32::from_raw(own), want_ignore);
            ignoring = want_ignore;
        }
        flush(&app, &island, &activities, &mut out);
    }
}

/// Applies and emits the machine's outputs in order.
fn flush(app: &AppHandle, island: &Island, activities: &Activities, out: &mut Vec<Output>) {
    for output in out.drain(..) {
        match output {
            Output::Expand | Output::Collapse => {
                let expanded = output == Output::Expand;
                {
                    let mut s = island.lock();
                    s.expanded = expanded;
                    if expanded {
                        // Fall back to the default until React measures this expansion.
                        s.rect = None;
                    }
                }
                let event = if expanded {
                    events::HOVER_ENTER
                } else {
                    events::HOVER_LEAVE
                };
                emit(app, event, ());
            }
            Output::Show(payload) => {
                if activities.current_id() != Some(payload.id) {
                    // A new activity may have a different size; use the default until it's measured.
                    island.lock().rect = None;
                    log::debug!("activity:show {:?}", payload.activity);
                }
                activities.set_current(Some(payload.clone()));
                emit(app, events::ACTIVITY_SHOW, payload);
            }
            Output::Hide(id) => {
                activities.set_current(None);
                emit(app, events::ACTIVITY_HIDE, ActivityHidePayload { id });
            }
        }
    }
}

fn emit<P: serde::Serialize + Clone>(app: &AppHandle, event: &str, payload: P) {
    if let Err(e) = app.emit(event, payload) {
        log::warn!("emit {event} failed: {e}");
    }
}

#[tauri::command]
/// Physical px: the island's width and its bottom edge measured from the window top, for the shape
/// named by `mode`. React converts with `devicePixelRatio`, which includes Windows' text-size zoom;
/// Rust can't see that.
pub fn set_panel_rect(island: State<'_, Island>, width: f64, bottom: f64, mode: RectMode) {
    log::debug!("{mode:?} rect reported: width={width} bottom={bottom}");
    let valid = |v: f64| v.is_finite() && v > 0.0 && v < 10_000.0;
    if valid(width) && valid(bottom) {
        island.lock().rect = Some((mode, PanelRect { width, bottom }));
    }
}

#[tauri::command]
pub fn request_collapse(island: State<'_, Island>) {
    island.request_collapse();
}

#[tauri::command]
pub fn get_island_state(
    island: State<'_, Island>,
    activities: State<'_, Activities>,
) -> IslandStatePayload {
    let s = island.lock();
    IslandStatePayload {
        expanded: s.expanded,
        hidden: s.hidden,
        activity: activities.current(),
    }
}
