//! Claude Code sessions through Claude Code hooks (CASESTUDY D17, D19).
//!
//! `topbar.exe hook` (`forward.rs`) runs as the hook command and writes a trimmed message to a per-user
//! named pipe. The `claude-listener` thread (`pipe.rs`, started through the activity `Watcher` harness)
//! keeps session state (`sessions.rs`), offers and withdraws activities on the shared activity channel,
//! and emits `claude:sessions` for the panel. `hooks.rs` installs the hooks from the tray.

mod focus;
mod forward;
mod hooks;
mod pipe;
mod process;
mod protocol;
mod sessions;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tauri::menu::MenuItem;
use tauri::{AppHandle, Emitter, Manager, State, Wry};
use windows::core::HSTRING;
use windows::Win32::System::Threading::SetEvent;
use windows::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, IDOK, MB_ICONINFORMATION, MB_ICONQUESTION, MB_ICONWARNING, MB_OK, MB_OKCANCEL,
    MB_SETFOREGROUND, MB_TOPMOST, MESSAGEBOX_STYLE,
};

use crate::activity::{claude_key, Activities, Signal, Watcher};
use crate::error::{AppError, AppResult};
use crate::hover::Island;
use crate::ipc::{events, ClaudeHooksStatus, ClaudeSessionsPayload};
use crate::win32::Handle;
use crate::{apps, win32, window};
use sessions::{Effect, Session, Sessions};

/// With no client, the listener wakes this often to drop sessions whose process is gone.
const PRUNE_INTERVAL: Duration = Duration::from_secs(30);

/// The forwarder: `main.rs` calls this for `topbar.exe hook`, before anything else starts.
pub fn forward_hook() {
    forward::run();
}

/// Tauri-managed copy of the listener's sessions, for commands.
#[derive(Default)]
pub struct Claude(Mutex<Vec<Session>>);

impl Claude {
    fn lock(&self) -> MutexGuard<'_, Vec<Session>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn session(&self, id: u32) -> Option<Session> {
        self.lock().iter().find(|s| s.id == id).cloned()
    }

    fn payload(&self) -> ClaudeSessionsPayload {
        ClaudeSessionsPayload {
            sessions: sessions::snapshot(&self.lock()),
        }
    }
}

/// Starts the listener unless the `claude` switch is off. Logged and skipped if the pipe can't be created
/// (e.g. another process holds the name).
pub fn spawn(app: &AppHandle, activities: &Activities) {
    if !activities.claude_enabled() {
        log::info!("Claude Code activities are switched off");
        return;
    }
    match start(app.clone(), activities.sender()) {
        Ok(watcher) => activities.add_watcher(watcher),
        Err(e) => log::warn!("Claude Code activities unavailable: {e}"),
    }
}

fn start(app: AppHandle, out: Sender<Signal>) -> AppResult<Watcher> {
    let server = pipe::Server::create()?;
    let stop = Arc::new(Handle::event()?);
    let signal = stop.clone();
    Watcher::spawn(
        "claude-listener",
        move || listen(&app, &out, &server, &stop),
        move || {
            // SAFETY: signals the event both sides hold through the Arc.
            let _ = unsafe { SetEvent(signal.0) };
        },
    )
}

fn listen(app: &AppHandle, out: &Sender<Signal>, server: &pipe::Server, stop: &Handle) {
    log::info!("listening for Claude Code hooks");
    let mut sessions = Sessions::default();
    let mut last_prune = Instant::now();
    loop {
        let next = match server.next(stop, PRUNE_INTERVAL.as_millis() as u32) {
            Ok(next) => next,
            Err(e) => {
                log::warn!("Claude Code listener stopped: {e}");
                return;
            }
        };
        let (mut changed, mut effects) = (false, Vec::new());
        match next {
            pipe::Next::Stop => {
                log::info!("Claude Code listener stopped");
                return;
            }
            pipe::Next::Message(bytes) => match protocol::decode(&bytes) {
                Some(msg) => {
                    log::debug!(
                        "claude hook {:?} ({})",
                        msg.event,
                        sessions::project(&msg.cwd)
                    );
                    (changed, effects) = sessions.apply(msg);
                }
                None => log::debug!("ignored a malformed Claude Code message"),
            },
            pipe::Next::Nothing | pipe::Next::Idle => {}
        }
        if last_prune.elapsed() >= PRUNE_INTERVAL {
            last_prune = Instant::now();
            let (dropped, retracts) =
                sessions.prune(now_ms(), |s| s.process().is_none_or(process::alive));
            changed |= dropped;
            effects.extend(retracts);
        }
        for effect in effects {
            let signal = match effect {
                Effect::Offer(activity) => Signal::Offer(activity),
                Effect::Retract(id) => Signal::Retract(claude_key(id)),
            };
            let _ = out.send(signal);
        }
        if changed {
            let claude = app.state::<Claude>();
            *claude.lock() = sessions.list().to_vec();
            if let Err(e) = app.emit(events::CLAUDE_SESSIONS, claude.payload()) {
                log::warn!("emit {} failed: {e}", events::CLAUDE_SESSIONS);
            }
        }
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[tauri::command]
pub fn get_claude_sessions(claude: State<'_, Claude>) -> ClaudeSessionsPayload {
    claude.payload()
}

#[tauri::command]
/// Brings the session's terminal or editor window to the front and acknowledges its activity. Sync, so it
/// runs on the main thread, which `apps::bring_to_front` needs.
pub fn claude_focus_session(
    app: AppHandle,
    claude: State<'_, Claude>,
    activities: State<'_, Activities>,
    island: State<'_, Island>,
    session_id: u32,
) -> AppResult<()> {
    let session = claude
        .session(session_id)
        .ok_or(AppError::ClaudeSessionGone)?;
    // Clicking it counts as handling it, even if its window can't be found.
    let _ = activities
        .sender()
        .send(Signal::Retract(claude_key(session_id)));
    let own = app
        .get_webview_window(window::LABEL)
        .ok_or(AppError::NoIslandWindow)?
        .hwnd()?;
    let hwnd =
        focus::find(&session.chain, &session.project, own).ok_or(AppError::ClaudeWindowNotFound)?;
    apps::bring_to_front(win32::from_raw(hwnd))?;
    island.request_collapse();
    Ok(())
}

#[tauri::command]
pub fn claude_hooks_status() -> ClaudeHooksStatus {
    ClaudeHooksStatus {
        installed: hooks_installed(),
    }
}

#[tauri::command]
pub fn claude_hooks_install() -> AppResult<()> {
    install().map(|_| ()).map_err(AppError::ClaudeSettings)
}

#[tauri::command]
pub fn claude_hooks_remove() -> AppResult<()> {
    remove().map(|_| ()).map_err(AppError::ClaudeSettings)
}

pub fn hooks_installed() -> bool {
    hooks::settings_path()
        .and_then(|path| hooks::read(&path).ok().flatten())
        .is_some_and(|settings| hooks::installed(&settings))
}

pub fn tray_label() -> &'static str {
    if hooks_installed() {
        "Disconnect Claude Code…"
    } else {
        "Connect Claude Code…"
    }
}

fn this_exe() -> Result<std::path::PathBuf, String> {
    std::env::current_exe().map_err(|e| format!("couldn't find TopBar's own path: {e}"))
}

fn settings_path() -> Result<std::path::PathBuf, String> {
    hooks::settings_path().ok_or_else(|| "couldn't find your user folder".to_string())
}

/// Returns the backup path, if there was a file to back up.
fn install() -> Result<Option<std::path::PathBuf>, String> {
    let path = settings_path()?;
    let shell = hooks::detect_shell();
    let command = hooks::command(&this_exe()?, shell);
    let settings = hooks::read(&path)?.unwrap_or_else(|| serde_json::json!({}));
    hooks::write(&path, &hooks::with_hooks(settings, &command, shell)?)
}

fn remove() -> Result<Option<std::path::PathBuf>, String> {
    let path = settings_path()?;
    let Some(settings) = hooks::read(&path)? else {
        return Ok(None);
    };
    hooks::write(&path, &hooks::without_hooks(settings)?)
}

/// Only one dialog at a time (a second tray click while one is open does nothing).
static DIALOG_OPEN: AtomicBool = AtomicBool::new(false);

/// Tray "Connect/Disconnect Claude Code…": shows exactly what changes, asks, then writes. The island can't
/// take keyboard input (CLAUDE.md rule 2), so this is a native message box on its own thread.
pub fn toggle_from_tray(item: MenuItem<Wry>) {
    if DIALOG_OPEN.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        if let Err(e) = run_dialog() {
            message(
                &format!("TopBar couldn't change the Claude Code settings:\n\n{e}"),
                MB_OK | MB_ICONWARNING,
            );
        }
        if let Err(e) = item.set_text(tray_label()) {
            log::warn!("updating tray label failed: {e}");
        }
        DIALOG_OPEN.store(false, Ordering::SeqCst);
    });
}

fn run_dialog() -> Result<(), String> {
    let path = settings_path()?;
    if hooks_installed() {
        let text = format!(
            "Remove TopBar's hooks from\n{}?\n\nOnly hooks whose command ends in \"topbar.exe' hook\" are removed; your \
             other settings and hooks stay. A backup of the file is saved next to it first.",
            path.display()
        );
        if !confirm(&text) {
            return Ok(());
        }
        let backup = remove()?;
        log::info!("removed Claude Code hooks from {}", path.display());
        message(
            &done_text("Claude Code is disconnected.", backup),
            MB_OK | MB_ICONINFORMATION,
        );
    } else {
        let shell = hooks::detect_shell();
        let command = hooks::command(&this_exe()?, shell);
        let text = format!(
            "TopBar will add these hooks to\n{}\n\n{}\n\nYour other settings and hooks stay. A backup of the file is \
             saved next to it first. Claude Code sessions that are already open may need a restart to pick them up.",
            path.display(),
            hooks::preview(&command, shell)
        );
        if !confirm(&text) {
            return Ok(());
        }
        let backup = install()?;
        log::info!("added Claude Code hooks to {}", path.display());
        message(
            &done_text("Claude Code is connected.", backup),
            MB_OK | MB_ICONINFORMATION,
        );
    }
    Ok(())
}

fn done_text(what: &str, backup: Option<std::path::PathBuf>) -> String {
    match backup {
        Some(backup) => format!("{what}\n\nBackup: {}", backup.display()),
        None => what.to_string(),
    }
}

fn confirm(text: &str) -> bool {
    message(text, MB_OKCANCEL | MB_ICONQUESTION) == IDOK
}

fn message(
    text: &str,
    style: MESSAGEBOX_STYLE,
) -> windows::Win32::UI::WindowsAndMessaging::MESSAGEBOX_RESULT {
    // SAFETY: both strings are valid for the call; no owner window, so it never involves the island.
    unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(text),
            &HSTRING::from("TopBar"),
            style | MB_SETFOREGROUND | MB_TOPMOST,
        )
    }
}
