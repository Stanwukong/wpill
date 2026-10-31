//! Live Activities: system events → `activity:show` / `activity:hide`.
//!
//! Each source is a watcher thread that owns its subscriptions (Phase 2 pattern, CASESTUDY D11) and sends
//! finished `Activity` values down one channel. The hover poller drains that channel every tick and owns
//! the queue (`queue.rs`) as part of its state machine, so all timing happens in one place, in Rust.

mod audio;
mod brightness;
mod drag;
mod power;
mod queue;
mod screenshots;
mod settings;

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager, State};
use windows::core::HSTRING;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CAPITAL};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::error::{AppError, AppResult};
use crate::ipc::{Activity, ActivityPayload};
use crate::window;
pub use queue::{approval_key, claude_key, Queue, ACTING_KEY};
use settings::Switches;

#[cfg(test)]
pub use queue::tests as test_activities;

/// Bounded wait for every watcher to remove its subscriptions on exit (they stop in parallel).
const SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(800);

/// What sources send the hover poller.
#[derive(Debug)]
pub enum Signal {
    Offer(Activity),
    /// Withdraws the activity with this key (`queue::claude_key`), waiting or on screen.
    Retract(String),
}

impl From<Activity> for Signal {
    fn from(activity: Activity) -> Self {
        Self::Offer(activity)
    }
}

type Start = fn(Sender<Signal>) -> AppResult<Watcher>;

/// A source thread. `stop` asks it to finish; `done` disconnects once it has (and its guards have dropped).
/// Other features' sources (e.g. `claude/`) use it too, so `shutdown` stops them with the rest.
pub struct Watcher {
    name: &'static str,
    stop: Box<dyn FnOnce() + Send>,
    done: Receiver<()>,
}

impl Watcher {
    pub fn spawn(
        name: &'static str,
        body: impl FnOnce() + Send + 'static,
        stop: impl FnOnce() + Send + 'static,
    ) -> AppResult<Self> {
        let (done_tx, done) = mpsc::channel::<()>();
        thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let _done = done_tx;
                body();
            })
            .map_err(tauri::Error::from)?;
        Ok(Self {
            name,
            stop: Box::new(stop),
            done,
        })
    }
}

struct Inner {
    tx: Sender<Signal>,
    /// Taken by the hover poller, the only consumer.
    rx: Option<Receiver<Signal>>,
    switches: Switches,
    /// What's on screen, for `get_island_state` and the screenshot commands.
    current: Option<ActivityPayload>,
    /// A screenshot drag is running (the poller holds the activity meanwhile).
    dragging: bool,
    /// An activity the commands finished early; the poller ends it.
    dismiss: Option<u32>,
    watchers: Vec<Watcher>,
}

/// Tauri-managed state shared between the sources, the hover poller and commands.
pub struct Activities(Mutex<Inner>);

impl Default for Activities {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self(Mutex::new(Inner {
            tx,
            rx: Some(rx),
            switches: Switches::default(),
            current: None,
            dragging: false,
            dismiss: None,
            watchers: Vec::new(),
        }))
    }
}

impl Activities {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn take_receiver(&self) -> Option<Receiver<Signal>> {
        self.lock().rx.take()
    }

    /// For sources outside this module; switches still apply to what they offer.
    pub fn sender(&self) -> Sender<Signal> {
        self.lock().tx.clone()
    }

    pub fn add_watcher(&self, watcher: Watcher) {
        self.lock().watchers.push(watcher);
    }

    pub fn claude_enabled(&self) -> bool {
        self.lock().switches.claude
    }

    pub fn allows(&self, activity: &Activity) -> bool {
        self.lock().switches.allows(activity)
    }

    pub fn current(&self) -> Option<ActivityPayload> {
        self.lock().current.clone()
    }

    pub fn current_id(&self) -> Option<u32> {
        self.lock().current.as_ref().map(|c| c.id)
    }

    pub fn set_current(&self, current: Option<ActivityPayload>) {
        self.lock().current = current;
    }

    /// `(dragging, activity to dismiss)`, read once per poller tick.
    pub fn poll_flags(&self) -> (bool, Option<u32>) {
        let mut inner = self.lock();
        (inner.dragging, inner.dismiss.take())
    }

    fn screenshot_path(&self, id: u32) -> AppResult<std::path::PathBuf> {
        match &self.lock().current {
            Some(ActivityPayload {
                id: current,
                activity: Activity::Screenshot(s),
                ..
            }) if *current == id => Ok(s.path.clone()),
            _ => Err(AppError::ActivityGone),
        }
    }

    /// Asks every watcher to stop and waits (bounded) until they've removed their subscriptions.
    pub fn shutdown(&self) {
        let watchers = std::mem::take(&mut self.lock().watchers);
        let mut pending = Vec::new();
        for Watcher { name, stop, done } in watchers {
            stop();
            pending.push((name, done));
        }
        let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
        for (name, done) in pending {
            let left = deadline.saturating_duration_since(Instant::now());
            match done.recv_timeout(left) {
                Err(RecvTimeoutError::Timeout) => log::debug!("{name} didn't stop in time"),
                _ => log::debug!("{name} stopped"),
            }
        }
    }
}

/// Loads the switches and starts every source. A source that can't start is logged and skipped.
pub fn spawn(app: &AppHandle, activities: &Activities) {
    let switches = settings::load(app);
    let tx = {
        let mut inner = activities.lock();
        inner.switches = switches;
        inner.tx.clone()
    };
    let starts: [(&str, Start); 4] = [
        ("audio", audio::start),
        ("brightness", brightness::start),
        ("power", power::start),
        ("screenshots", screenshots::start),
    ];
    for (name, start) in starts {
        match start(tx.clone()) {
            Ok(watcher) => activities.lock().watchers.push(watcher),
            Err(e) => log::warn!("{name} activities unavailable: {e}"),
        }
    }
}

/// Toggle state of Caps Lock. Polled by the hover poller; `GetKeyState` tracks it from a thread with no
/// message queue too (checked with SendInput), so no keyboard hook is needed (CASESTUDY D1).
pub fn caps_lock_on() -> bool {
    // SAFETY: read-only key state query.
    unsafe { GetKeyState(VK_CAPITAL.0 as i32) & 1 != 0 }
}

#[tauri::command]
/// Starts dragging the screenshot of activity `id` out of the island. Sync, so it runs on the main thread,
/// which Tao initialized for OLE; the call returns when the drop (or cancel) is done.
pub fn activity_drag_screenshot(
    app: AppHandle,
    activities: State<'_, Activities>,
    id: u32,
) -> AppResult<()> {
    let path = activities.screenshot_path(id)?;
    let island = app
        .get_webview_window(window::LABEL)
        .ok_or(AppError::NoIslandWindow)?;
    let hwnd = island.hwnd()?;
    activities.lock().dragging = true;
    let result = drag::drag_file(hwnd, &path);
    let mut inner = activities.lock();
    inner.dragging = false;
    let dropped = result?;
    log::debug!("screenshot drag finished; dropped: {dropped}");
    if dropped {
        inner.dismiss = Some(id);
    }
    Ok(())
}

#[tauri::command]
/// Opens the screenshot of activity `id` in the default viewer and ends the activity.
pub fn activity_open_screenshot(activities: State<'_, Activities>, id: u32) -> AppResult<()> {
    let path = activities.screenshot_path(id)?;
    let file = HSTRING::from(path.as_os_str());
    // SAFETY: file is a valid NUL-terminated wide string for the duration of the call.
    let result = unsafe {
        ShellExecuteW(
            None,
            windows::core::w!("open"),
            &file,
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    // Values above 32 mean success.
    if result.0 as usize <= 32 {
        return Err(AppError::Win32(windows::core::Error::from_thread()));
    }
    activities.lock().dismiss = Some(id);
    Ok(())
}
