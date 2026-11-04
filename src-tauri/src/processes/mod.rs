//! Memory-heavy apps (Phase 3): the panel's "Memory" section, off by default (`processes.enabled`).
//!
//! The `processes-watcher` thread sleeps until the hover poller says the panel is open (`Processes::panel`), then
//! takes one snapshot every 2 s (`snapshot.rs`, ~3.5 ms), groups it into apps (`logic.rs`) and pushes the biggest as
//! `processes:heavy`. While the panel is closed it does nothing at all. `end_process_group` (3b, `end.rs`) ends an
//! app after a native confirm box, only ever acting on processes this module listed.

mod end;
mod logic;
mod names;
mod settings;
mod snapshot;

use std::collections::{HashMap, HashSet};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager, State};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

use crate::activity::{Activities, Watcher};
use crate::apps::{self, IconCache};
use crate::error::AppError;
use crate::ipc::{
    events, EndProcessOutcome, EndResult, HeavyProcessesPayload, ProcessGroup, ProcessId,
};
use crate::{win32, window};
use logic::{Group, Proc};

const REFRESH: Duration = Duration::from_secs(2);

#[derive(Default)]
struct Inner {
    enabled: bool,
    limit: u32,
    /// The panel is open: the watcher refreshes.
    open: bool,
    stop: bool,
    /// Refresh now rather than after the usual wait (an app was just ended).
    dirty: bool,
    /// An end is in progress (its confirm box or its wait): one at a time.
    ending: bool,
    latest: Vec<ProcessGroup>,
}

/// Tauri-managed state shared by the watcher, the hover poller and the command.
#[derive(Default)]
pub struct Processes {
    inner: Mutex<Inner>,
    wake: Condvar,
}

impl Processes {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Called by the hover poller when the panel opens or closes.
    pub fn panel(&self, open: bool) {
        let mut inner = self.lock();
        if inner.enabled && inner.open != open {
            inner.open = open;
            self.wake.notify_all();
        }
    }

    /// Refresh at once if the panel is open, so an ended app leaves the list.
    fn refresh_soon(&self) {
        self.lock().dirty = true;
        self.wake.notify_all();
    }

    fn payload(&self, limit: Option<u32>) -> HeavyProcessesPayload {
        let inner = self.lock();
        let take = limit.unwrap_or(inner.limit) as usize;
        HeavyProcessesPayload {
            enabled: inner.enabled,
            groups: inner.latest.iter().take(take).cloned().collect(),
        }
    }
}

pub fn spawn(app: &AppHandle, activities: &Activities) {
    let settings = settings::load(app);
    let processes = app.state::<Processes>();
    {
        let mut inner = processes.lock();
        inner.enabled = settings.enabled;
        inner.limit = settings.limit;
    }
    if !settings.enabled {
        return;
    }
    let body_app = app.clone();
    let stop_app = app.clone();
    let watcher = Watcher::spawn(
        "processes-watcher",
        move || run(&body_app),
        move || {
            let processes = stop_app.state::<Processes>();
            processes.lock().stop = true;
            processes.wake.notify_all();
        },
    );
    match watcher {
        Ok(w) => activities.add_watcher(w),
        Err(e) => log::warn!("processes watcher failed to start: {e}"),
    }
}

/// What the watcher keeps between refreshes so a refresh doesn't open every process again.
#[derive(Default)]
struct Caches {
    /// Exe paths by `(pid, created)`; pruned to the processes still running.
    paths: HashMap<(u32, u64), Option<String>>,
    /// Display names by exe path.
    names: HashMap<String, String>,
}

fn run(app: &AppHandle) {
    // SAFETY: SHGetFileInfoW (exe icons) needs COM on this thread; it lives as long as the thread.
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let processes = app.state::<Processes>();
    let own_pid = std::process::id();
    let mut caches = Caches::default();
    loop {
        {
            let guard = processes.lock();
            let guard = processes
                .wake
                .wait_while(guard, |s| !s.open && !s.stop)
                .unwrap_or_else(|e| e.into_inner());
            if guard.stop {
                break;
            }
        }
        let limit = {
            let mut inner = processes.lock();
            inner.dirty = false;
            inner.limit as usize
        };
        if let Some(groups) = refresh(app, own_pid, limit, &mut caches) {
            processes.lock().latest = groups.clone();
            let payload = HeavyProcessesPayload {
                enabled: true,
                groups,
            };
            if let Err(e) = app.emit(events::PROCESSES_HEAVY, payload) {
                log::warn!("emit {} failed: {e}", events::PROCESSES_HEAVY);
            }
        }
        let guard = processes.lock();
        let (guard, _) = processes
            .wake
            .wait_timeout_while(guard, REFRESH, |s| s.open && !s.stop && !s.dirty)
            .unwrap_or_else(|e| e.into_inner());
        if guard.stop {
            break;
        }
    }
    log::debug!("processes watcher stopped");
}

fn refresh(
    app: &AppHandle,
    own_pid: u32,
    limit: usize,
    caches: &mut Caches,
) -> Option<Vec<ProcessGroup>> {
    let started = Instant::now();
    let raw = snapshot::read()?;
    let read_at = started.elapsed();

    let alive: HashSet<(u32, u64)> = raw.iter().map(|p| (p.pid, p.created)).collect();
    caches.paths.retain(|k, _| alive.contains(k));
    let procs: Vec<Proc> = raw
        .into_iter()
        .map(|p| {
            let path = caches
                .paths
                .entry((p.pid, p.created))
                .or_insert_with(|| {
                    if p.pid <= 4 {
                        None
                    } else {
                        apps::process_path(p.pid)
                    }
                })
                .clone();
            Proc {
                pid: p.pid,
                parent: p.parent,
                created: p.created,
                session: p.session,
                name: p.name,
                path,
                private_bytes: p.private_bytes,
            }
        })
        .collect();
    let count = procs.len();
    let groups = logic::group(&procs, own_pid);

    // A window of the app gives the icon Alt+Tab shows (UWP apps' exe icons are often blank).
    let own_hwnd = app
        .get_webview_window(window::LABEL)
        .and_then(|w| w.hwnd().ok())
        .map(win32::raw)
        .unwrap_or_default();
    let windows: HashMap<u32, isize> = apps::top_level_windows(win32::from_raw(own_hwnd))
        .into_iter()
        .rev()
        .map(|w| (w.pid, w.hwnd))
        .collect();
    let icons = app.state::<IconCache>();
    let out: Vec<ProcessGroup> = groups
        .into_iter()
        .take(limit)
        .map(|g| to_dto(g, &windows, &icons, &mut caches.names))
        .collect();
    log::debug!(
        "processes: {count} read in {:.1} ms, {} apps shown, {:.1} ms in all",
        read_at.as_secs_f64() * 1000.0,
        out.len(),
        started.elapsed().as_secs_f64() * 1000.0
    );
    Some(out)
}

fn to_dto(
    g: Group,
    windows: &HashMap<u32, isize>,
    icons: &IconCache,
    names: &mut HashMap<String, String>,
) -> ProcessGroup {
    let fallback = || {
        g.exe_name
            .strip_suffix(".exe")
            .or_else(|| g.exe_name.strip_suffix(".EXE"))
            .unwrap_or(&g.exe_name)
            .to_string()
    };
    let name = match &g.path {
        Some(path) => names
            .entry(path.clone())
            .or_insert_with(|| names::file_description(path).unwrap_or_else(fallback))
            .clone(),
        None => fallback(),
    };
    let hwnd = g
        .members
        .iter()
        .find_map(|(pid, _)| windows.get(pid).copied())
        .unwrap_or_default();
    ProcessGroup {
        icon: icons.icon_for(win32::from_raw(hwnd), g.path.as_deref()),
        key: g.key,
        name,
        pids: g
            .members
            .iter()
            .map(|&(pid, created)| ProcessId {
                pid,
                created: created.to_string(),
            })
            .collect(),
        memory_bytes: g.bytes,
        protected: g.protected,
    }
}

/// The latest list (empty until the panel has been open once), and whether the section is switched on.
#[tauri::command]
pub fn list_heavy_processes(
    processes: State<'_, Processes>,
    limit: Option<u32>,
) -> HeavyProcessesPayload {
    processes.payload(limit)
}

/// Ends one app from the list (Phase 3b): asks in a native confirm box, asks the app to close, then ends what's left.
/// Only pids Rust listed for `key` are touched, each checked again by creation time; protected apps are refused.
#[tauri::command]
pub async fn end_process_group(
    app: AppHandle,
    key: String,
    pids: Vec<ProcessId>,
) -> Result<EndResult, AppError> {
    tauri::async_runtime::spawn_blocking(move || end_group(&app, &key, &pids)).await?
}

/// Clears `ending` however `end_group` returns.
struct Ending<'a>(&'a Processes);

impl Drop for Ending<'_> {
    fn drop(&mut self) {
        self.0.lock().ending = false;
        self.0.refresh_soon();
    }
}

fn end_group(app: &AppHandle, key: &str, pids: &[ProcessId]) -> Result<EndResult, AppError> {
    let processes = app.state::<Processes>();
    let group = {
        let mut inner = processes.lock();
        let group = inner
            .latest
            .iter()
            .find(|g| g.key == key)
            .cloned()
            .ok_or(AppError::ProcessGroupGone)?;
        if inner.ending {
            return Err(AppError::ProcessEndBusy);
        }
        inner.ending = true;
        group
    };
    let _ending = Ending(&processes);
    let listed: Vec<end::Id> = group.pids.iter().filter_map(parse_id).collect();
    // An id the page sent that doesn't parse can't be one Rust listed.
    let requested: Vec<end::Id> = pids
        .iter()
        .map(|p| parse_id(p).unwrap_or((p.pid, 0)))
        .collect();
    let own_pid = std::process::id();
    let own_hwnd = app
        .get_webview_window(window::LABEL)
        .and_then(|w| w.hwnd().ok())
        .map(win32::raw)
        .unwrap_or_default();

    let fresh = || end::now().ok_or(AppError::ProcessGroupGone);
    let first = end::check(&requested, &listed, &fresh()?, own_pid);
    if first.go.is_empty() {
        log::info!(
            "end {}: nothing to end ({})",
            group.name,
            end::tally(&first.settled)
        );
        return Ok(result(first.settled));
    }
    if !end::confirm(&end::confirm_text(
        &group.name,
        first.go.len(),
        group.memory_bytes,
    )) {
        log::info!("end {}: cancelled", group.name);
        return Ok(EndResult {
            cancelled: true,
            outcomes: Vec::new(),
        });
    }
    // The box may have been open a while: check again.
    let checked = end::check(&requested, &listed, &fresh()?, own_pid);
    let mut outcomes = checked.settled;
    outcomes.extend(end::end(&checked.go, own_hwnd));
    log::info!(
        "end {}: {} processes: {}",
        group.name,
        outcomes.len(),
        end::tally(&outcomes)
    );
    Ok(result(outcomes))
}

fn parse_id(p: &ProcessId) -> Option<end::Id> {
    p.created.parse().ok().map(|c| (p.pid, c))
}

fn result(outcomes: Vec<(end::Id, crate::ipc::EndOutcome)>) -> EndResult {
    EndResult {
        cancelled: false,
        outcomes: outcomes
            .into_iter()
            .map(|((pid, created), outcome)| EndProcessOutcome {
                pid,
                created: created.to_string(),
                outcome,
            })
            .collect(),
    }
}
