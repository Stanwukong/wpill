//! Memory-heavy apps (Phase 3): the panel's "Memory" section, off by default (`processes.enabled`).
//!
//! The `processes-watcher` thread sleeps until the hover poller says the panel is open (`Processes::panel`), then
//! takes one snapshot every 2 s (`snapshot.rs`, ~3.5 ms), groups it into apps (`logic.rs`) and pushes the biggest as
//! `processes:heavy`. While the panel is closed it does nothing at all. Ending a group is stage 3b.

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
use crate::ipc::{events, HeavyProcessesPayload, ProcessGroup, ProcessId};
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
        let limit = processes.lock().limit as usize;
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
            .wait_timeout_while(guard, REFRESH, |s| s.open && !s.stop)
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
