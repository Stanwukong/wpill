//! Cursor poller: owns expanded/collapsed state, click-through, and fullscreen hiding.

mod logic;

use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

use crate::error::AppResult;
use crate::ipc::{events, IslandStatePayload, VisibilityPayload};
use crate::window::{self, Geometry};
use crate::{fullscreen, win32};
use logic::{PanelRect, Phase, Sample, Transition};

const POLL_INTERVAL: Duration = Duration::from_millis(16);
const FULLSCREEN_INTERVAL: Duration = Duration::from_millis(250);

struct Shared {
    /// Reported by React; `None` until the first expand.
    panel: Option<PanelRect>,
    force_collapse: bool,
    expanded: bool,
    hidden: bool,
}

/// Tauri-managed state shared between the poller thread and commands.
pub struct Island(Mutex<Shared>);

impl Default for Island {
    fn default() -> Self {
        Self(Mutex::new(Shared {
            panel: None,
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
    let mut phase = Phase::Collapsed;
    let mut ignoring = true;
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
                if fullscreen {
                    s.expanded = false;
                }
                changed
            };
            if changed {
                log::info!(
                    "fullscreen app in foreground: {fullscreen}; island hidden: {fullscreen}"
                );
                if fullscreen {
                    if phase.is_expanded() {
                        emit(&app, events::HOVER_LEAVE, ());
                    }
                    phase = Phase::Collapsed;
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

        let (panel, force, hidden) = {
            let mut s = island.lock();
            (s.panel, std::mem::take(&mut s.force_collapse), s.hidden)
        };
        if hidden {
            continue;
        }

        let mut cursor = POINT::default();
        // SAFETY: cursor is a valid out-pointer. Fails e.g. on the secure desktop; skip the tick.
        if unsafe { GetCursorPos(&mut cursor) }.is_err() {
            continue;
        }

        let sample = Sample {
            now,
            in_hot_zone: logic::in_hot_zone(&geo, cursor.x, cursor.y),
            in_panel: logic::in_panel(
                &geo,
                panel.unwrap_or_else(|| PanelRect::default_for(&geo)),
                cursor.x,
                cursor.y,
            ),
        };
        let (next, transition) = if force {
            logic::force_collapse(phase)
        } else {
            logic::step(phase, &sample)
        };
        if std::mem::discriminant(&next) != std::mem::discriminant(&phase) {
            log::debug!(
                "hover {phase:?} -> {next:?} at ({}, {})",
                cursor.x,
                cursor.y
            );
        }
        phase = next;

        // Accept clicks only while the cursor is over the panel, so the transparent
        // margins (and the panel during its leave delay) stay click-through.
        let want_ignore = phase != Phase::Expanded;
        if want_ignore != ignoring {
            window::set_click_through(win32::from_raw(own), want_ignore);
            ignoring = want_ignore;
        }

        if let Some(transition) = transition {
            let expanded = transition == Transition::Expand;
            {
                let mut s = island.lock();
                s.expanded = expanded;
                if expanded {
                    // Fall back to the default until React measures this expansion.
                    s.panel = None;
                }
            }
            let event = if expanded {
                events::HOVER_ENTER
            } else {
                events::HOVER_LEAVE
            };
            emit(&app, event, ());
        }
    }
}

fn emit<P: serde::Serialize + Clone>(app: &AppHandle, event: &str, payload: P) {
    if let Err(e) = app.emit(event, payload) {
        log::warn!("emit {event} failed: {e}");
    }
}

#[tauri::command]
/// Physical px: the panel's width and its bottom edge measured from the window top.
/// React converts with `devicePixelRatio`, which includes Windows' text-size zoom; Rust can't see that.
pub fn set_panel_rect(island: State<'_, Island>, width: f64, bottom: f64) {
    log::debug!("panel rect reported: width={width} bottom={bottom}");
    let valid = |v: f64| v.is_finite() && v > 0.0 && v < 10_000.0;
    if valid(width) && valid(bottom) {
        island.lock().panel = Some(PanelRect { width, bottom });
    }
}

#[tauri::command]
pub fn request_collapse(island: State<'_, Island>) {
    island.request_collapse();
}

#[tauri::command]
pub fn get_island_state(island: State<'_, Island>) -> IslandStatePayload {
    let s = island.lock();
    IslandStatePayload {
        expanded: s.expanded,
        hidden: s.hidden,
    }
}
