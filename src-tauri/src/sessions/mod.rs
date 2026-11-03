//! Claude Code sessions through Claude Code hooks (CASESTUDY D17, D19).
//!
//! `cay.exe hook` (`forward.rs`) runs as the hook command and writes a trimmed message to a per-user
//! named pipe. The `session-listener` thread (`pipe.rs`, started through the activity `Watcher` harness)
//! keeps session state (`sessions.rs`), offers and withdraws activities on the shared activity channel,
//! and emits `sessions:changed` for the panel. `hooks.rs` installs the hooks from the tray.
//!
//! Permission requests (2.8a) have their own pipe: the forwarder holds each one open until you answer it in the
//! island or it's handed back to the terminal (`ask.rs` on the wire, `asks.rs` in Cay).

mod ask;
mod asks;
mod claude;
mod codex;
mod files;
mod focus;
mod forward;
mod opencode;
mod pipe;
mod process;
mod protocol;
mod table;
mod version;

use claude::{hooks, statusline};

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tauri::menu::MenuItem;
use tauri::{AppHandle, Emitter, Manager, State, Wry};
use windows::core::HSTRING;
use windows::Win32::System::Threading::SetEvent;
use windows::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, IDNO, IDOK, IDYES, MB_ICONINFORMATION, MB_ICONQUESTION, MB_ICONWARNING, MB_OK,
    MB_OKCANCEL, MB_SETFOREGROUND, MB_TOPMOST, MB_YESNOCANCEL, MESSAGEBOX_STYLE,
};

use crate::activity::{session_key, Activities, Signal, Watcher};
use crate::error::{AppError, AppResult};
use crate::hover::Island;
use crate::ipc::{events, AgentKind, ClaudeHooksStatus, SessionsPayload, UsageLimits};
use crate::win32::Handle;
use crate::{apps, win32, window};
pub use asks::Asks;
use protocol::Event;
use table::{Effect, Session, Table};

/// With no client, the listener wakes this often to drop sessions whose process is gone.
const PRUNE_INTERVAL: Duration = Duration::from_secs(30);

/// The forwarder: `main.rs` calls this for `cay.exe hook`, before anything else starts.
pub fn forward_hook() {
    forward::run();
}

/// `cay.exe statusline` (2.8f): `main.rs` calls this before anything else starts.
pub fn forward_statusline() {
    statusline::run();
}

/// The island's name for a working folder (its last component, shortened), shared with agent runs.
pub fn project_name(cwd: &str) -> String {
    table::project(cwd)
}

/// Tauri-managed copy of the listener's sessions and usage limits, for commands.
#[derive(Default)]
pub struct Sessions(Mutex<Shared>);

#[derive(Default)]
struct Shared {
    sessions: Vec<Session>,
    limits: Option<UsageLimits>,
}

impl Sessions {
    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn session(&self, id: u32) -> Option<Session> {
        self.lock().sessions.iter().find(|s| s.id == id).cloned()
    }

    /// Our id for `agent`'s own session id, if the listener has seen that session.
    fn row_of(&self, agent: AgentKind, native_id: &str) -> Option<u32> {
        self.lock()
            .sessions
            .iter()
            .find(|s| s.agent == agent && s.native_id() == native_id)
            .map(|s| s.id)
    }

    fn payload(&self) -> SessionsPayload {
        let shared = self.lock();
        SessionsPayload {
            sessions: table::snapshot(&shared.sessions),
            limits: shared.limits,
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
    match asks::start(app.clone(), activities.sender()) {
        Ok(watcher) => activities.add_watcher(watcher),
        Err(e) => log::warn!("Claude Code permission requests unavailable: {e}"),
    }
}

fn start(app: AppHandle, out: Sender<Signal>) -> AppResult<Watcher> {
    let server = pipe::Server::create()?;
    let stop = Arc::new(Handle::event()?);
    let signal = stop.clone();
    Watcher::spawn(
        "session-listener",
        move || listen(&app, &out, &server, &stop),
        move || {
            // SAFETY: signals the event both sides hold through the Arc.
            let _ = unsafe { SetEvent(signal.0) };
        },
    )
}

fn listen(app: &AppHandle, out: &Sender<Signal>, server: &pipe::Server, stop: &Handle) {
    log::info!("listening for Claude Code hooks");
    let mut sessions = Table::default();
    let mut limits: Option<UsageLimits> = None;
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
                // Agent runs started by Cay fire the same hooks; `agent/` shows those itself.
                Some(msg) if msg.chain.iter().any(process::is_self) => {
                    log::debug!(
                        "{} hook {:?} from an agent run; ignored",
                        msg.agent.display(),
                        msg.event
                    );
                }
                // The status line (2.8f): account-wide limits, not a session's status.
                Some(msg) if msg.event == Event::Limits => {
                    if let Some(reading) = msg.limits {
                        let mut next = statusline::newest(limits, reading);
                        statusline::mark_stale(&mut next, now_ms());
                        changed = limits != Some(next);
                        limits = Some(next);
                    }
                }
                Some(msg) => {
                    log::debug!(
                        "{} hook {:?} ({})",
                        msg.agent.display(),
                        msg.event,
                        table::project(&msg.cwd)
                    );
                    if msg.agent == AgentKind::Codex {
                        codex::saw_hook();
                    }
                    let asks = app.state::<Asks>();
                    let held = asks.holds_for(&msg.session);
                    let event = msg.event;
                    match event {
                        Event::Prompt
                        | Event::Stop
                        | Event::StopFailure
                        | Event::SessionEnd
                        | Event::SessionStart { compact: false } => {
                            asks.release_session(&msg.session)
                        }
                        _ => {}
                    }
                    (changed, effects) = sessions.apply(msg);
                    // The island's card already asks; a second "needs input" would repeat it.
                    if held && event == Event::NeedsInput {
                        effects.retain(|e| !matches!(e, Effect::Offer(_)));
                    }
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
            if let Some(l) = limits.as_mut() {
                changed |= statusline::mark_stale(l, now_ms());
            }
        }
        for effect in effects {
            let signal = match effect {
                Effect::Offer(activity) => Signal::Offer(activity),
                Effect::Retract(id) => Signal::Retract(session_key(id)),
            };
            let _ = out.send(signal);
        }
        if changed {
            let claude = app.state::<Sessions>();
            {
                let mut shared = claude.lock();
                shared.sessions = sessions.list().to_vec();
                shared.limits = limits;
            }
            if let Err(e) = app.emit(events::SESSIONS, claude.payload()) {
                log::warn!("emit {} failed: {e}", events::SESSIONS);
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
pub fn get_sessions(claude: State<'_, Sessions>) -> SessionsPayload {
    claude.payload()
}

#[tauri::command]
/// Brings the session's terminal or editor window to the front and acknowledges its activity. Sync, so it
/// runs on the main thread, which `apps::bring_to_front` needs.
pub fn session_focus(
    app: AppHandle,
    claude: State<'_, Sessions>,
    activities: State<'_, Activities>,
    island: State<'_, Island>,
    session_id: u32,
) -> AppResult<()> {
    let session = claude.session(session_id).ok_or(AppError::SessionGone)?;
    // Clicking it counts as handling it, even if its window can't be found.
    let _ = activities
        .sender()
        .send(Signal::Retract(session_key(session_id)));
    let own = app
        .get_webview_window(window::LABEL)
        .ok_or(AppError::NoIslandWindow)?
        .hwnd()?;
    let hwnd = focus::find(&session.chain, &session.project, own)
        .ok_or(AppError::SessionWindowNotFound)?;
    apps::bring_to_front(win32::from_raw(hwnd))?;
    island.request_collapse();
    Ok(())
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Answer {
    Allow,
    Deny,
}

#[tauri::command]
/// Your answer to a terminal session's permission request (2.8a). The request's thread replies and withdraws the card.
pub fn session_decide(app: AppHandle, request_id: u32, answer: Answer) -> AppResult<()> {
    asks::decide(&app, request_id, matches!(answer, Answer::Allow))
}

#[tauri::command]
/// The session requests the island holds (2.8b), for the command window.
pub fn session_asks(asks: tauri::State<'_, asks::Asks>) -> crate::ipc::SessionAsksPayload {
    crate::ipc::SessionAsksPayload { asks: asks.list() }
}

#[tauri::command]
/// A held request's whole diff or command; `None` once it's answered or handed back.
pub fn session_ask_preview(
    asks: tauri::State<'_, asks::Asks>,
    request_id: u32,
) -> Option<crate::ipc::ApprovalPreview> {
    asks.preview(request_id)
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionAnswer {
    pub question: String,
    /// The chosen option labels (one for a single-select).
    #[serde(default)]
    pub labels: Vec<String>,
    /// Your own words instead of an option.
    #[serde(default)]
    pub other: Option<String>,
}

#[tauri::command]
/// Your answer to one question of a session's held `AskUserQuestion` (2.8c). Errors `agentInvalid` for an answer
/// that isn't one of its options, `agentApprovalGone` once it's no longer held.
pub fn session_answer(app: AppHandle, request_id: u32, answer: QuestionAnswer) -> AppResult<()> {
    asks::answer(
        &app,
        request_id,
        &answer.question,
        &answer.labels,
        answer.other.as_deref(),
    )
}

#[tauri::command]
/// Approve a session's held plan, or send it back with `feedback` (2.8c).
pub fn session_plan(
    app: AppHandle,
    request_id: u32,
    approve: bool,
    feedback: Option<String>,
) -> AppResult<()> {
    asks::plan(&app, request_id, approve, feedback.as_deref().unwrap_or(""))
}

/// `agent-test session-decide <requestId> allow|deny` (debug builds).
pub fn decide_for_test(app: &AppHandle, request_id: u32, allow: bool) -> AppResult<()> {
    asks::decide(app, request_id, allow)
}

#[tauri::command]
pub fn claude_hooks_status() -> ClaudeHooksStatus {
    let state = hook_state();
    ClaudeHooksStatus {
        installed: state == hooks::State::Connected,
        out_of_date: state == hooks::State::OutOfDate,
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

/// Connected, connected by an earlier version (needs an update), or not connected (2.8f).
fn hook_state() -> hooks::State {
    hooks::settings_path()
        .and_then(|path| hooks::read(&path).ok().flatten())
        .map_or(hooks::State::NotConnected, |settings| {
            hooks::state(&settings)
        })
}

/// Tray "Connect Codex…" (2.9a).
pub fn codex_tray_label() -> &'static str {
    codex::tray_label()
}

pub fn codex_toggle_from_tray(item: MenuItem<Wry>) {
    codex::toggle_from_tray(item);
}

/// Tray "Connect OpenCode…" (2.9a).
pub fn opencode_tray_label() -> &'static str {
    opencode::tray_label()
}

pub fn opencode_toggle_from_tray(item: MenuItem<Wry>) {
    opencode::toggle_from_tray(item);
}

pub fn codex_keep_tray_item(item: MenuItem<Wry>) {
    codex::keep_tray_item(item);
}

pub fn tray_label() -> &'static str {
    match hook_state() {
        hooks::State::Connected => "Disconnect Claude Code…",
        hooks::State::OutOfDate => "Update Claude Code connection…",
        hooks::State::NotConnected => "Connect Claude Code…",
    }
}

fn this_exe() -> Result<std::path::PathBuf, String> {
    std::env::current_exe().map_err(|e| format!("couldn't find Cay's own path: {e}"))
}

fn settings_path() -> Result<std::path::PathBuf, String> {
    hooks::settings_path().ok_or_else(|| "couldn't find your user folder".to_string())
}

/// The user's settings with Cay connected as this version writes it (hooks and the chained status line), and what
/// to keep of their own status line.
fn connected(
    settings: serde_json::Value,
) -> Result<(serde_json::Value, statusline::Saved), String> {
    let shell = hooks::detect_shell();
    let exe = this_exe()?;
    let saved = statusline::saved();
    let saved_line = saved.as_ref().and_then(|s| s.status_line.as_ref());
    let settings = hooks::with_hooks(settings, &hooks::command(&exe, shell), shell)?;
    let (settings, original) =
        hooks::with_status_line(settings, &hooks::status_command(&exe, shell), saved_line)?;
    Ok((
        settings,
        statusline::Saved {
            status_line: original,
            shell,
        },
    ))
}

/// Connects (or updates). Returns the backup path, if there was a file to back up.
fn install() -> Result<Option<std::path::PathBuf>, String> {
    let path = settings_path()?;
    let settings = hooks::read(&path)?.unwrap_or_else(|| serde_json::json!({}));
    let (settings, saved) = connected(settings)?;
    // Kept first: if writing their settings fails, the next Connect still knows their original status line.
    statusline::keep(Some(&saved))?;
    hooks::write(&path, &settings)
}

/// Disconnects: Cay's hooks out, the user's own status line back in place.
fn remove() -> Result<Option<std::path::PathBuf>, String> {
    let path = settings_path()?;
    let Some(settings) = hooks::read(&path)? else {
        return Ok(None);
    };
    let saved = statusline::saved();
    let settings = hooks::without_status_line(
        hooks::without_hooks(settings)?,
        saved.as_ref().and_then(|s| s.status_line.as_ref()),
    )?;
    let backup = hooks::write(&path, &settings)?;
    statusline::keep(None)?;
    Ok(backup)
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
                &format!("Cay couldn't change the Claude Code settings:\n\n{e}"),
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
    let state = hook_state();
    if state == hooks::State::Connected {
        if !confirm(&remove_text(&path)) {
            return Ok(());
        }
        return disconnect(&path);
    }
    let shell = hooks::detect_shell();
    let exe = this_exe()?;
    let current = hooks::read(&path)?.unwrap_or_else(|| serde_json::json!({}));
    let saved = statusline::saved();
    let change = format!(
        "{}\n\n{}",
        hooks::preview(&hooks::command(&exe, shell), shell),
        hooks::status_preview(
            &hooks::without_hooks(current)?,
            &hooks::status_command(&exe, shell),
            saved.as_ref().and_then(|s| s.status_line.as_ref()),
        )
    );
    let tail = "Your other settings and hooks stay. A backup of the file is saved next to it first. Claude Code \
                sessions that are already open may need a restart to pick the change up.";
    if state == hooks::State::OutOfDate {
        let text = format!(
            "Cay's connection in\n{}\nis from an earlier version. Update it to this?\n\n{change}\n\n{tail}\n\n\
             Yes: update.   No: disconnect Cay instead.",
            path.display()
        );
        return match message(&text, MB_YESNOCANCEL | MB_ICONQUESTION) {
            IDYES => connect(&path, "Claude Code's connection is up to date."),
            IDNO => disconnect(&path),
            _ => Ok(()),
        };
    }
    let text = format!(
        "Cay will add this to\n{}\n\n{change}\n\n{tail}",
        path.display()
    );
    if !confirm(&text) {
        return Ok(());
    }
    connect(&path, "Claude Code is connected.")
}

fn remove_text(path: &std::path::Path) -> String {
    format!(
        "Remove Cay from\n{}?\n\nOnly hooks whose command ends in \"cay.exe' hook\" (or TopBar's) are removed, and \
         your own status line is put back as it was; your other settings and hooks stay. A backup of the file is \
         saved next to it first.",
        path.display()
    )
}

fn connect(path: &std::path::Path, done: &str) -> Result<(), String> {
    let backup = install()?;
    log::info!("connected Claude Code in {}", path.display());
    message(&done_text(done, backup), MB_OK | MB_ICONINFORMATION);
    Ok(())
}

fn disconnect(path: &std::path::Path) -> Result<(), String> {
    let backup = remove()?;
    log::info!("disconnected Claude Code in {}", path.display());
    message(
        &done_text("Claude Code is disconnected.", backup),
        MB_OK | MB_ICONINFORMATION,
    );
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
            &HSTRING::from("Cay"),
            style | MB_SETFOREGROUND | MB_TOPMOST,
        )
    }
}
