//! Local automation agent (Phase 2.7, CASESTUDY D21–D23): headless Claude Code runs started from the command
//! window, shown in the island, with their permission prompts answered from it.
//!
//! - `run.rs` starts `claude.exe -p` (stream-json both ways) in a Job Object; its stdout feeds `lifecycle.rs`.
//! - Claude Code reaches Cay's MCP server through `cay.exe mcp` (`bridge.rs`) and the agent pipe
//!   (`pipe.rs`). The `agent-server` thread accepts bridges; each connection proves its run with a token, and its
//!   `approve` calls (`mcp.rs`) become `agentApproval` activities (`approvals.rs`, `describe.rs`).
//! - The desktop tools (2.7c, `tools.rs` → `crate::desktop`) look at windows and the screen. Each look is
//!   approved (Claude Code's prompt, then Cay's own grant check), runs on its own thread, and lights the
//!   screen edges (`window::glow`) while it runs.
//! - The desktop actions (2.7d, `actions.rs` → `crate::desktop::act`) focus, invoke, set fields, click, type,
//!   press keys and launch. Each is approved on its own (never "for this run"), waits until you let go of the
//!   mouse and keyboard, shows `agentActing` and the acting glow, and stops at once if you touch either.
//! - The Google tools (6c, `google_tools.rs` → `crate::google::run_tool`) read Calendar, Tasks and Gmail and save
//!   Gmail drafts. Gmail always asks in Cay; a run that has read mail has its desktop actions asked about again.
//! - Hotkeys (`hotkey.rs`) open the command window or stop everything. Everything is logged (`audit.rs`).
//! - Routines (2.7e, `routines.rs`, `schedule.rs`) are saved instructions, run from the command window or by the
//!   `agent-routines` thread on a schedule while Cay runs. A routine's run is an ordinary run: same approvals,
//!   same Stop, same audit; nothing about it is pre-approved.
//!
//! Rust owns runs, approvals and timeouts; React only renders `agent:runs`, `agent:output` and the activities.

mod acp;
mod actions;
mod approvals;
mod attach;
mod attachments;
mod audit;
mod backend;
mod bridge;
mod clipboard;
mod describe;
mod folder;
mod google_tools;
mod hotkey;
mod installs;
pub mod interact;
mod job;
mod lifecycle;
mod links;
mod mcp;
mod mode;
mod patch;
mod pipe;
mod preview;
mod routines;
mod run;
mod schedule;
mod settings;
mod stream;
mod tools;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, State};
use windows::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::System::Threading::{
    ResetEvent, SetEvent, WaitForMultipleObjects, WaitForSingleObject, INFINITE,
};

use crate::activity::{approval_key, Activities, Signal, Watcher, ACTING_KEY};
use crate::desktop::{self, refs::Refs};
use crate::error::{AppError, AppResult};
use crate::ipc::{
    events, Activity, AgentActingActivity, AgentActingPhase, AgentEntry, AgentEvent,
    AgentFinishedActivity, AgentMode, AgentOptions, AgentOutputPayload, AgentRisk, AgentRunRoutine,
    AgentRunsPayload, AgentStarted, AgentStatus, ClaudeInstall, Routine, RoutineLastRun,
    RoutineSkip, RoutineTrigger, RoutineView, RoutinesPayload, SkipReason,
};
use crate::ipc::{AgentBackend, AgentBackendInfo, AgentBackendsPayload};
use crate::win32::Handle;
use crate::window::{self, glow};
use approvals::{Approvals, Asked, Pending, Purpose, ReplyTo, Request};
use audit::{now_ms, Audit};
use lifecycle::Run;
use routines::RoutineDraft;
use schedule::{Check, Times, Verdict};
use settings::AgentSettings;
use stream::StreamEvent;

/// A bridge gets this long to say hello after connecting.
const HELLO_TIMEOUT_MS: u32 = 2000;
/// Ended runs kept for the panel and the command window.
const MAX_ENDED: usize = 20;
const MAX_PROMPT: usize = 32 * 1024;
/// The routine scheduler looks again at least this often, which also notices sleep, resume and clock changes.
const SCHEDULER_TICK_MS: i64 = 60_000;
/// The wall clock jumping this far past the expected wake-up means the PC slept (or the clock was changed).
const RESUME_JUMP_MS: i64 = 2 * 60_000;
/// "Approvals wait for me": you count as away while the screen is locked or after this long without input.
const AWAY_IDLE_MS: u64 = 5 * 60_000;
/// Messages named per run for approvals, at most.
const MAX_MAIL_LABELS: usize = 200;
/// An ACP run that's working, isn't waiting on you and has sent nothing (not even reasoning) for this long is stopped:
/// a stalled model would otherwise hold the run open until you press Stop (2.9b).
const ACP_SILENCE: Duration = Duration::from_secs(10 * 60);

/// `cay.exe mcp`: `main.rs` calls this before anything else starts.
pub fn mcp_bridge() -> i32 {
    bridge::run()
}

/// How an approval reads (display name, one-line summary, risk), shared with terminal sessions' requests (2.8a).
pub fn describe_tool(tool: &str, input: &Value, cwd: &str) -> (String, String, AgentRisk) {
    (
        describe::display_tool(tool),
        describe::summary(tool, input, cwd),
        describe::risk(tool),
    )
}

/// A terminal session's live step and to-do progress (2.8d), from its `PreToolUse` hook.
pub fn step_of(tool: &str, input: &Value, cwd: &str) -> (String, Option<crate::ipc::TodoCount>) {
    (
        describe::step_line(tool, input, cwd),
        describe::todo_count(tool, input),
    )
}

/// A terminal session's request (2.8b): the card's and the command window's diff or command. `cut` names the input
/// fields shortened on the way (`sessions::ask`). For a `Write`, the file it replaces is read here (≤256 KB).
pub fn preview_tool(
    tool: &str,
    input: &Value,
    cwd: &str,
    cut: &[String],
) -> Option<(
    Option<crate::ipc::ApprovalPreview>,
    crate::ipc::ApprovalPreview,
)> {
    let existing = preview::read_existing(tool, input, cwd, cut);
    preview::build(tool, input, cwd, &existing, cut).map(|p| (p.card, p.full))
}

struct Slot {
    run: Run,
    token: String,
    launch: run::Launch,
    process: Option<run::Process>,
    /// An ACP agent's protocol state for the current process (2.9b); `None` for Claude Code.
    acp: Option<acp::Client>,
    /// When the run's agent last sent anything: an ACP run silent for `ACP_SILENCE` is stopped (2.9b).
    heard: Instant,
    /// Bumped per process, so a late line or exit from an earlier process is ignored.
    generation: u32,
    dir: PathBuf,
    /// A scheduled routine's run with "Approvals wait for me": its approvals wait while you're away.
    patient: bool,
}

impl Slot {
    fn is_going(&self) -> bool {
        self.run.is_live() || self.process.is_some()
    }
}

struct Conn {
    pipe: Arc<pipe::Pipe>,
    run_id: String,
}

#[derive(Default)]
struct Inner {
    settings: AgentSettings,
    slots: Vec<Slot>,
    approvals: Approvals,
    conns: HashMap<u64, Conn>,
    next_conn: u64,
    audit: Option<Arc<Audit>>,
    installs: Option<Vec<ClaudeInstall>>,
    hotkey_errors: Vec<String>,
    signals: Option<Sender<Signal>>,
    /// Ends the server, connection and timer threads.
    stop: Option<Arc<Handle>>,
    /// What each run's looks have seen: `ui_tree` refs and screenshot positions, for actions.
    refs: Refs,
    /// The action running now (one at a time): its run and call, so Stop or a cancel can abort it.
    acting: Option<(String, u64, Value)>,
    routines: routines::Store,
    /// Wakes the `agent-routines` thread after a change.
    routines_wake: Option<Arc<Handle>>,
    /// Each agent's model list, once fetched (2.9c): `opencode models` takes a few seconds.
    models: HashMap<AgentBackend, Vec<String>>,
    /// Runs that have read mail (Cay's Gmail tools or a claude.ai Gmail connector): their desktop actions ask in
    /// Cay even in Auto, since the mail could carry someone else's instructions (D46).
    read_mail: HashSet<String>,
    /// How each run's messages are named in approvals (`"Invoice 42" from Bob`), from its searches and reads.
    mail_labels: HashMap<String, HashMap<String, String>>,
    launched_ms: i64,
    /// Missed routines wait until then to catch up (Cay's start or the PC's wake, plus a little).
    settle_until_ms: i64,
}

/// Tauri-managed agent state. Every thread and command goes through it; replies to bridges are written after
/// the lock is released.
#[derive(Default)]
pub struct Agents(Mutex<Inner>);

/// Work to do once the lock is released.
/// An approved desktop look, run after the lock is released.
struct LookJob {
    pipe: Arc<pipe::Pipe>,
    rpc_id: Value,
    run_id: String,
    tool: String,
    args: Value,
    pid: Option<u32>,
}

#[derive(Default)]
struct After {
    replies: Vec<(Arc<pipe::Pipe>, Value)>,
    looks: Vec<LookJob>,
    signals: Vec<Signal>,
    outputs: Vec<AgentOutputPayload>,
    drafts: Vec<crate::ipc::AgentTextDraft>,
    runs_changed: bool,
    routines_changed: bool,
}

impl After {
    fn run(self, app: &AppHandle, agents: &Agents) {
        for (pipe, message) in self.replies {
            if let Err(e) = pipe.write(message.to_string().as_bytes()) {
                log::debug!("agent reply not delivered: {e}");
            }
        }
        for job in self.looks {
            run_look(app, job);
        }
        if let Some(tx) = agents.lock().signals.clone() {
            for signal in self.signals {
                let _ = tx.send(signal);
            }
        }
        for draft in self.drafts {
            if let Err(e) = app.emit_to(window::command::LABEL, events::AGENT_TEXT_DRAFT, draft) {
                log::debug!("emit {} failed: {e}", events::AGENT_TEXT_DRAFT);
            }
        }
        for output in self.outputs {
            if let Err(e) = app.emit_to(window::command::LABEL, events::AGENT_OUTPUT, output) {
                log::debug!("emit {} failed: {e}", events::AGENT_OUTPUT);
            }
        }
        if self.runs_changed {
            let payload = agents.payload();
            if let Err(e) = app.emit(events::AGENT_RUNS, payload) {
                log::warn!("emit {} failed: {e}", events::AGENT_RUNS);
            }
        }
        if self.routines_changed {
            let payload = agents.routines_payload();
            crate::tray::show_routines_paused(payload.paused);
            if let Err(e) = app.emit(events::ROUTINES_CHANGED, payload) {
                log::warn!("emit {} failed: {e}", events::ROUTINES_CHANGED);
            }
        }
    }
}

impl Agents {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn payload(&self) -> AgentRunsPayload {
        let inner = self.lock();
        let mut runs: Vec<_> = inner
            .slots
            .iter()
            .map(|s| s.run.ipc(inner.approvals.pending_for(&s.run.id)))
            .collect();
        runs.sort_by_key(|r| std::cmp::Reverse(r.started_at_ms));
        AgentRunsPayload { runs }
    }

    fn routines_payload(&self) -> RoutinesPayload {
        let inner = self.lock();
        let now = now_ms();
        let paused = inner.routines.file.paused;
        let routines = inner
            .routines
            .file
            .routines
            .iter()
            .map(|r| RoutineView {
                next_run_at_ms: (!paused && r.enabled)
                    .then(|| {
                        let times = Times {
                            since_ms: r.schedule_since_ms,
                            launched_ms: inner.launched_ms,
                        };
                        schedule::next(&r.schedule, times, now, &routines::LocalZone)
                    })
                    .flatten()
                    .filter(|&n| n > r.schedule_since_ms),
                running_run_id: inner.running_run_of(&r.id),
                routine: r.clone(),
            })
            .collect();
        RoutinesPayload { routines, paused }
    }

    /// Kills every run's process tree. Called on exit; the jobs would also die with Cay.
    pub fn shutdown(&self) {
        let mut inner = self.lock();
        for slot in &mut inner.slots {
            if let Some(process) = slot.process.take() {
                slot.run.stop();
                process.job.kill();
                let _ = std::fs::remove_dir_all(&slot.dir);
            }
        }
        if let Some(stop) = inner.stop.take() {
            // SAFETY: signals the event the agent threads wait on.
            let _ = unsafe { SetEvent(stop.0) };
        }
    }
}

impl Inner {
    fn slot(&mut self, run_id: &str) -> Option<&mut Slot> {
        self.slots.iter_mut().find(|s| s.run.id == run_id)
    }

    fn audit(&self, run_id: &str, kind: &str, fields: Value) {
        if let Some(audit) = &self.audit {
            audit.log(run_id, kind, fields);
        }
    }

    /// A line in `routines.jsonl`: due times run or skipped.
    fn audit_routine(&self, kind: &str, fields: Value) {
        if let Some(audit) = &self.audit {
            audit.log_routine(kind, fields);
        }
    }

    /// The run of routine `id` going now, if any.
    fn running_run_of(&self, id: &str) -> Option<String> {
        self.slots
            .iter()
            .find(|s| s.is_going() && s.run.routine.as_ref().is_some_and(|r| r.id == id))
            .map(|s| s.run.id.clone())
    }

    fn wake_scheduler(&self) {
        if let Some(wake) = &self.routines_wake {
            // SAFETY: signals our own event.
            let _ = unsafe { SetEvent(wake.0) };
        }
    }

    /// A due time of routine `id` didn't start a run: noted on the routine and in `routines.jsonl`.
    fn record_skip(&mut self, id: &str, due_ms: i64, why: SkipReason, now: i64) {
        let Some(r) = self.routines.get_mut(id) else {
            return;
        };
        r.last_due_ms = r.last_due_ms.max(due_ms);
        r.last_skip = Some(RoutineSkip {
            at_ms: now,
            due_ms,
            why,
        });
        let name = r.name.clone();
        log::info!("routine {id} ({name}) skipped: {why:?}");
        self.audit_routine(
            "skipped",
            json!({ "routine": id, "name": name, "dueMs": due_ms, "why": why }),
        );
    }

    /// Answers and withdraws approvals (denied unless `allow`), noting the decision in the transcript.
    fn settle(&mut self, settled: Vec<Pending>, allow: bool, by: &'static str, after: &mut After) {
        for p in settled {
            let message = match by {
                "timeout" => "Nobody answered in Cay within the time limit, so this was denied.",
                "runEnded" => "The run ended before this was answered.",
                "cancelled" => "The call was cancelled before it was answered.",
                _ => {
                    "The user denied this in Cay. Don't retry it; adapt or explain what you needed."
                }
            };
            if p.purpose == Purpose::Agent {
                let run_id = p.activity.run_id.clone();
                self.answer_acp(&run_id, &p.reply.rpc_id, allow);
            } else if let Some(conn) = self.conns.get(&p.reply.conn) {
                let rpc_id = p.reply.rpc_id.clone();
                match p.purpose {
                    Purpose::Agent => {}
                    Purpose::Prompt => {
                        // A plan sent back carries your feedback instead (2.8c).
                        let message = p.message.as_deref().unwrap_or(message);
                        let reply = mcp::decision(rpc_id, allow, p.input.clone(), message);
                        after.replies.push((conn.pipe.clone(), reply));
                    }
                    // Cay's own check before a look: allowed means look now, denied is the tool's error.
                    Purpose::Look if allow => after.looks.push(LookJob {
                        pipe: conn.pipe.clone(),
                        rpc_id,
                        run_id: p.activity.run_id.clone(),
                        tool: p.tool.clone(),
                        args: p.input.clone(),
                        pid: p.target_pid,
                    }),
                    Purpose::Look => after
                        .replies
                        .push((conn.pipe.clone(), tools::refused(rpc_id, message))),
                }
            }
            after
                .signals
                .push(Signal::Retract(approval_key(p.activity.request_id)));
            let request_id = p.activity.request_id;
            let run_id = p.activity.run_id.clone();
            self.audit(
                &run_id,
                "decision",
                json!({ "requestId": request_id, "allow": allow, "by": by }),
            );
            let waiting = self.approvals.has_pending(&run_id);
            if let Some(slot) = self.slot(&run_id) {
                let at = now_ms();
                let entry = slot.run.note(
                    AgentEvent::Decision {
                        request_id,
                        allow,
                        by,
                    },
                    at,
                );
                slot.run.set_waiting(waiting, at);
                after.outputs.push(AgentOutputPayload { run_id, entry });
            }
        }
        after.runs_changed = true;
    }

    /// The run just ended: withdraw its approvals, offer `agentFinished` (not after Stop: you know), clean up.
    fn finished(&mut self, run_id: &str, after: &mut After) {
        self.refs.end_run(run_id);
        let pending = self.approvals.end_run(run_id);
        self.settle(pending, false, "runEnded", after);
        let Some(slot) = self.slot(run_id) else {
            return;
        };
        let (status, reason) = (slot.run.status, slot.run.reason);
        let project = slot.run.project.clone();
        let routine = slot.run.routine.clone();
        let cost_usd = slot.run.ipc(vec![]).cost_usd;
        let fields = json!({ "status": status, "reason": reason, "turns": slot.run.ipc(vec![]).turns,
                             "costUsd": cost_usd, "error": slot.run.ipc(vec![]).error });
        self.audit(run_id, "result", fields);
        if let Some(tag) = &routine {
            let last = self
                .routines
                .get_mut(&tag.id)
                .and_then(|r| r.last_run.as_mut())
                .filter(|l| l.run_id == run_id);
            if let Some(last) = last {
                last.status = Some(status);
                last.reason = reason;
                last.cost_usd = cost_usd;
                self.routines.save();
            }
            after.routines_changed = true;
        }
        if status != AgentStatus::Stopped {
            after.signals.push(Signal::Offer(Activity::AgentFinished(
                AgentFinishedActivity {
                    run_id: run_id.to_string(),
                    project,
                    routine_name: routine.map(|r| r.name),
                    status,
                    reason,
                },
            )));
        }
        // Keep the panel short: forget the oldest ended runs.
        let ended: Vec<_> = self
            .slots
            .iter()
            .filter(|s| !s.run.is_live() && s.process.is_none())
            .map(|s| (s.run.ipc(vec![]).started_at_ms, s.run.id.clone()))
            .collect();
        if ended.len() > MAX_ENDED {
            let mut ended = ended;
            ended.sort();
            for (_, id) in ended.iter().take(ended.len() - MAX_ENDED) {
                self.slots.retain(|s| &s.run.id != id);
            }
        }
        let slots = &self.slots;
        self.read_mail
            .retain(|id| slots.iter().any(|s| &s.run.id == id));
        self.mail_labels
            .retain(|id, _| slots.iter().any(|s| &s.run.id == id));
    }
}

/// Starts the server, timer and hotkey threads. Logged and skipped if the pipe can't be created.
pub fn spawn(app: &AppHandle, activities: &Activities) {
    let settings = settings::load(app);
    let dir = app
        .path()
        .app_data_dir()
        .map(|d| d.join("agent"))
        .unwrap_or_else(|_| std::env::temp_dir().join("cay-agent"));
    let audit = Arc::new(Audit::new(dir));
    audit.prune();
    let stop = match Handle::event() {
        Ok(stop) => Arc::new(stop),
        Err(e) => {
            log::warn!("agent unavailable: {e}");
            return;
        }
    };
    let routines_path = app
        .path()
        .app_config_dir()
        .map(|d| d.join("routines.json"))
        .ok();
    let wake = Handle::event().ok().map(Arc::new);
    {
        let agents = app.state::<Agents>();
        let mut inner = agents.lock();
        inner.settings = settings.clone();
        inner.audit = Some(audit);
        inner.signals = Some(activities.sender());
        inner.stop = Some(stop.clone());
        if let Some(path) = routines_path {
            inner.routines = routines::Store::load(path);
        }
        inner.routines_wake = wake.clone();
        let now = now_ms();
        inner.launched_ms = now;
        inner.settle_until_ms = now + schedule::SETTLE;
    }

    // Probe the installs now (each runs `claude.exe --version`), so the command window opens with them ready.
    let probe = app.clone();
    let _ = std::thread::Builder::new()
        .name("agent-installs".into())
        .spawn(move || {
            installs(&probe.state::<Agents>());
        });

    match start_server(app.clone(), stop.clone()) {
        Ok(watcher) => activities.add_watcher(watcher),
        Err(e) => log::warn!("agent pipe unavailable, approvals won't work: {e}"),
    }
    match app
        .get_webview_window(window::LABEL)
        .and_then(|w| w.hwnd().ok())
        .map(glow::spawn)
    {
        Some(Ok(watcher)) => activities.add_watcher(watcher),
        Some(Err(e)) => log::warn!("glow unavailable: {e}"),
        None => log::warn!("glow unavailable: no island window"),
    }
    match start_timer(app.clone(), stop.clone()) {
        Ok(watcher) => activities.add_watcher(watcher),
        Err(e) => log::warn!("agent timer unavailable: {e}"),
    }
    match wake.map(|wake| start_scheduler(app.clone(), stop, wake)) {
        Some(Ok(watcher)) => activities.add_watcher(watcher),
        Some(Err(e)) => log::warn!("routine scheduler unavailable: {e}"),
        None => log::warn!("routine scheduler unavailable: no event"),
    }
    let handle = app.clone();
    match hotkey::start(
        settings.hotkey,
        settings.stop_hotkey,
        move |key| match key {
            hotkey::Hotkey::OpenCommand => {
                if let Err(e) = window::command::show(&handle, None) {
                    log::warn!("opening the command window failed: {e}");
                }
            }
            hotkey::Hotkey::StopAll => stop_all(&handle),
        },
    ) {
        Ok((watcher, errors)) => {
            activities.add_watcher(watcher);
            app.state::<Agents>().lock().hotkey_errors = errors;
        }
        Err(e) => log::warn!("agent hotkeys unavailable: {e}"),
    }
}

fn start_server(app: AppHandle, stop: Arc<Handle>) -> AppResult<Watcher> {
    let first = pipe::create(true)?;
    let signal = stop.clone();
    Watcher::spawn(
        "agent-server",
        move || {
            log::info!("agent pipe ready");
            let mut next = Some(first);
            loop {
                let instance = match next.take().map_or_else(|| pipe::create(false), Ok) {
                    Ok(instance) => instance,
                    Err(e) => {
                        log::warn!("agent pipe stopped: {e}");
                        return;
                    }
                };
                match instance.accept(stop.0) {
                    Ok(true) => {
                        let (app, stop) = (app.clone(), stop.clone());
                        let spawned = std::thread::Builder::new()
                            .name("agent-conn".into())
                            .spawn(move || serve(&app, Arc::new(instance), &stop));
                        if let Err(e) = spawned {
                            log::warn!("agent connection thread: {e}");
                        }
                    }
                    Ok(false) => return,
                    Err(e) => log::debug!("agent pipe accept failed: {e}"),
                }
            }
        },
        move || {
            // SAFETY: signals the event both sides hold through the Arc.
            let _ = unsafe { SetEvent(signal.0) };
        },
    )
}

/// One bridge: hello, then MCP messages until it disconnects.
fn serve(app: &AppHandle, pipe: Arc<pipe::Pipe>, stop: &Handle) {
    let agents = app.state::<Agents>();
    let hello = match pipe.read(&[stop.0], HELLO_TIMEOUT_MS) {
        Ok(pipe::Read::Message(m)) => serde_json::from_slice::<mcp::Hello>(&m).ok(),
        _ => None,
    };
    let conn = {
        let mut inner = agents.lock();
        let valid = hello.as_ref().is_some_and(|h| {
            h.cay == mcp::HELLO_VERSION
                && inner
                    .slots
                    .iter()
                    .any(|s| s.run.id == h.run && s.token == h.token && s.process.is_some())
        });
        if !valid {
            log::warn!("agent pipe: refused a connection without a valid run token");
            return;
        }
        let run_id = hello.map(|h| h.run).unwrap_or_default();
        let offer = inner
            .slots
            .iter()
            .find(|s| s.run.id == run_id)
            .map_or(mcp::Offer::ALL, |s| {
                backend::offer(s.run.backend, s.launch.desktop_tools, s.launch.google_tools)
            });
        inner.next_conn += 1;
        let id = inner.next_conn;
        inner.conns.insert(
            id,
            Conn {
                pipe: pipe.clone(),
                run_id,
            },
        );
        (id, offer)
    };
    let (conn, offer) = conn;
    loop {
        let message = match pipe.read(&[stop.0], INFINITE) {
            Ok(pipe::Read::Message(m)) => m,
            Ok(pipe::Read::TooLong) => continue,
            _ => break,
        };
        let Ok(message) = serde_json::from_slice::<Value>(&message) else {
            continue;
        };
        match mcp::handle(&message, offer) {
            mcp::Action::Reply(reply) => {
                let _ = pipe.write(reply.to_string().as_bytes());
            }
            mcp::Action::Approve {
                rpc_id,
                tool_name,
                input,
            } => approve(app, conn, rpc_id, tool_name, input),
            mcp::Action::Look { rpc_id, name, args } => look(app, conn, rpc_id, name, args),
            mcp::Action::Act { rpc_id, name, args } => act(app, conn, rpc_id, name, args),
            mcp::Action::Google { rpc_id, name, args } => google(app, conn, rpc_id, name, args),
            mcp::Action::Cancel { request_id } => cancel(app, conn, &request_id),
            mcp::Action::Ignore => {}
        }
    }
    let mut after = After::default();
    {
        let mut inner = agents.lock();
        inner.conns.remove(&conn);
        let gone = inner.approvals.drop_conn(conn);
        inner.settle(gone, false, "runEnded", &mut after);
    }
    after.run(app, &agents);
}

/// Claude Code's permission prompt for `tool`.
fn approve(app: &AppHandle, conn: u64, rpc_id: Value, tool: String, input: Value) {
    let target = desktop_target(&tool, &input);
    let agents = app.state::<Agents>();
    let mut after = After::default();
    {
        let mut inner = agents.lock();
        let Some(c) = inner.conns.get(&conn) else {
            return;
        };
        let (pipe, run_id) = (c.pipe.clone(), c.run_id.clone());
        inner.audit(
            &run_id,
            "approval",
            json!({ "tool": tool, "input": audit::input_for_log(&redact(&tool, &input)) }),
        );
        let element = actions::element_for(&tool, &input, &run_id, &inner.refs, Instant::now());
        let target = if describe::is_google(&tool) {
            (inner.mail_label(&run_id, &input), None)
        } else {
            target
        };
        let reply = ReplyTo { conn, rpc_id };
        let asked = Ask {
            run_id: &run_id,
            tool: &tool,
            purpose: Purpose::Prompt,
            target,
            element,
        };
        if let Some((reply, input, _)) = ask(&mut inner, asked, input, reply, &mut after) {
            after
                .replies
                .push((pipe, mcp::decision(reply.rpc_id, true, input, "")));
        }
    }
    after.run(app, &agents);
}

/// Whether the run's desktop tools follow its mode (Auto) instead of Cay asking itself (2.8e).
fn follows_claude(inner: &mut Inner, run_id: &str) -> bool {
    inner
        .slot(run_id)
        .is_some_and(|s| mode::desktop_follows_claude(s.run.mode()))
}

/// One of the desktop tools was called: it runs if its prompt was approved (a grant) or its kind is allowed for
/// this run; otherwise Cay asks in the island itself first.
fn look(app: &AppHandle, conn: u64, rpc_id: Value, name: &'static str, args: Value) {
    let tool = format!("mcp__{}__{name}", mcp::SERVER_NAME);
    let target = desktop_target(&tool, &args);
    let agents = app.state::<Agents>();
    let mut after = After::default();
    {
        let mut inner = agents.lock();
        let Some(c) = inner.conns.get(&conn) else {
            return;
        };
        let (pipe, run_id) = (c.pipe.clone(), c.run_id.clone());
        if let Err(why) = tools::parse(name, &args) {
            after.replies.push((pipe, tools::refused(rpc_id, &why)));
        } else if let Some(pid) = inner.approvals.take_grant(&run_id, &tool, &args) {
            after.looks.push(LookJob {
                pipe,
                rpc_id,
                run_id,
                tool,
                args,
                pid,
            });
        } else if follows_claude(&mut inner, &run_id) {
            // Auto (2.8e, D40): Claude Code's classifier let it through, as for any other tool.
            after.looks.push(LookJob {
                pipe,
                rpc_id,
                run_id,
                tool,
                args,
                pid: target.1,
            });
        } else {
            log::info!("agent look {tool} arrived without an approved prompt; asking in Cay");
            let reply = ReplyTo { conn, rpc_id };
            let asked = Ask {
                run_id: &run_id,
                tool: &tool,
                purpose: Purpose::Look,
                target,
                element: None,
            };
            if let Some((reply, args, pid)) = ask(&mut inner, asked, args, reply, &mut after) {
                after.looks.push(LookJob {
                    pipe,
                    rpc_id: reply.rpc_id,
                    run_id,
                    tool,
                    args,
                    pid,
                });
            }
        }
    }
    after.run(app, &agents);
}

/// One of the desktop actions was called. Like a look, it runs only with the one-time grant its approved
/// prompt left (actions are never remembered for the run); without one, Cay asks in the island itself.
fn act(app: &AppHandle, conn: u64, rpc_id: Value, name: &'static str, args: Value) {
    let tool = format!("mcp__{}__{name}", mcp::SERVER_NAME);
    let target = desktop_target(&tool, &args);
    let agents = app.state::<Agents>();
    let mut after = After::default();
    {
        let mut inner = agents.lock();
        let Some(c) = inner.conns.get(&conn) else {
            return;
        };
        let (pipe, run_id) = (c.pipe.clone(), c.run_id.clone());
        let checked =
            actions::parse(name, &args).map_err(|why| tools::refused(rpc_id.clone(), &why));
        let checked = checked.and_then(|call| {
            actions::resolve(&call, &run_id, &inner.refs, Instant::now())
                .map_err(|e| actions::reply(rpc_id.clone(), &Err(e)))
        });
        if let Err(reply) = checked {
            after.replies.push((pipe, reply));
        } else if let Some(pid) = inner.approvals.take_grant(&run_id, &tool, &args) {
            after.looks.push(LookJob {
                pipe,
                rpc_id,
                run_id,
                tool,
                args,
                pid,
            });
        } else if follows_claude(&mut inner, &run_id) && !inner.read_mail.contains(&run_id) {
            after.looks.push(LookJob {
                pipe,
                rpc_id,
                run_id,
                tool,
                args,
                pid: target.1,
            });
        } else {
            log::info!("agent action {tool} arrived without an approved prompt; asking in Cay");
            let element = actions::element_for(&tool, &args, &run_id, &inner.refs, Instant::now());
            let reply = ReplyTo { conn, rpc_id };
            let asked = Ask {
                run_id: &run_id,
                tool: &tool,
                purpose: Purpose::Look,
                target,
                element,
            };
            if let Some((reply, args, pid)) = ask(&mut inner, asked, args, reply, &mut after) {
                after.looks.push(LookJob {
                    pipe,
                    rpc_id: reply.rpc_id,
                    run_id,
                    tool,
                    args,
                    pid,
                });
            }
        }
    }
    after.run(app, &agents);
}

/// One of the Google tools was called (6c). Like a look it runs with the grant its approved prompt left, or a
/// remembered "for this run"; Calendar and Tasks also follow Auto (Claude Code's classifier let them through).
/// Gmail never follows Auto: without a grant, Cay asks itself.
fn google(app: &AppHandle, conn: u64, rpc_id: Value, name: &'static str, args: Value) {
    let tool = format!("mcp__{}__{name}", mcp::SERVER_NAME);
    let agents = app.state::<Agents>();
    let mut after = After::default();
    {
        let mut inner = agents.lock();
        let Some(c) = inner.conns.get(&conn) else {
            return;
        };
        let (pipe, run_id) = (c.pipe.clone(), c.run_id.clone());
        let job = |rpc_id, args| LookJob {
            pipe: pipe.clone(),
            rpc_id,
            run_id: run_id.clone(),
            tool: tool.clone(),
            args,
            pid: None,
        };
        if let Err(why) = crate::google::tools::parse(name, &args) {
            after
                .replies
                .push((pipe.clone(), tools::refused(rpc_id, &why)));
        } else if inner.approvals.take_grant(&run_id, &tool, &args).is_some()
            || (google_tools::follows_auto(name) && follows_claude(&mut inner, &run_id))
        {
            // An approved prompt's grant, or Auto for Calendar and Tasks (Claude Code's classifier let it through).
            after.looks.push(job(rpc_id, args));
        } else {
            log::info!("agent google {tool} arrived without an approved prompt; asking in Cay");
            let label = inner.mail_label(&run_id, &args);
            let reply = ReplyTo { conn, rpc_id };
            let asked = Ask {
                run_id: &run_id,
                tool: &tool,
                purpose: Purpose::Look,
                target: (label, None),
                element: None,
            };
            if let Some((reply, args, _)) = ask(&mut inner, asked, args, reply, &mut after) {
                after.looks.push(job(reply.rpc_id, args));
            }
        }
    }
    after.run(app, &agents);
}

/// Claude Code gave up on call `rpc_id`: its approval is withdrawn, and if it's the action running now, that
/// action stops before its next event.
fn cancel(app: &AppHandle, conn: u64, rpc_id: &Value) {
    let agents = app.state::<Agents>();
    let mut after = After::default();
    {
        let mut inner = agents.lock();
        if inner
            .acting
            .as_ref()
            .is_some_and(|(_, c, id)| *c == conn && id == rpc_id)
        {
            log::info!("agent action cancelled by Claude Code; stopping it");
            desktop::abort_action();
        }
        let cancelled = inner.approvals.cancel(conn, rpc_id);
        inner.settle(cancelled, false, "cancelled", &mut after);
    }
    after.run(app, &agents);
}

/// Brings `hwnd` to the front on the main thread (the foreground fallback chain needs a message queue).
fn focus_on_main(app: &AppHandle, hwnd: isize) -> Result<(), desktop::ActError> {
    let (tx, rx) = std::sync::mpsc::channel();
    let posted = app.run_on_main_thread(move || {
        let _ = tx.send(crate::apps::bring_to_front(crate::win32::from_raw(hwnd)));
    });
    if posted.is_err() {
        return Err(desktop::ActError::NotForeground);
    }
    match rx.recv_timeout(Duration::from_secs(3)) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(AppError::WindowGone)) => Err(desktop::ActError::WindowGone),
        _ => Err(desktop::ActError::NotForeground),
    }
}

/// Runs an approved action on its own thread: the acting glow, the island kept out of the way, the
/// `agentActing` activity (waiting for you to let go, then acting), then the reply and the audit line.
fn run_act(app: &AppHandle, job: LookJob) {
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("agent-act".into())
        .spawn(move || {
            let name = actions::short_name(&job.tool).unwrap_or_default();
            let agents = app.state::<Agents>();
            let (prepared, signals, project) = {
                let mut inner = agents.lock();
                let conn = inner
                    .conns
                    .iter()
                    .find(|(_, c)| Arc::ptr_eq(&c.pipe, &job.pipe))
                    .map_or(0, |(id, _)| *id);
                let prepared = actions::parse(name, &job.args)
                    .map_err(desktop::ActError::Refused)
                    .and_then(|call| {
                        let (action, pid) =
                            actions::resolve(&call, &job.run_id, &inner.refs, Instant::now())?;
                        Ok((call.hwnd, action, pid))
                    });
                let project = inner
                    .slot(&job.run_id)
                    .map(|s| s.run.project.clone())
                    .unwrap_or_default();
                if prepared.is_ok() {
                    inner.acting = Some((job.run_id.clone(), conn, job.rpc_id.clone()));
                }
                let element = actions::element_for(
                    &job.tool,
                    &job.args,
                    &job.run_id,
                    &inner.refs,
                    Instant::now(),
                );
                (
                    prepared.map(|p| (p, element)),
                    inner.signals.clone(),
                    project,
                )
            };
            let result = match prepared {
                Err(e) => Err(e),
                Ok(((hwnd, action, refs_pid), element)) => {
                    let label = hwnd
                        .and_then(desktop::identify)
                        .map(|t| tools::target_label(&t));
                    let step = describe::acting_step(
                        &job.tool,
                        &job.args,
                        label.as_deref(),
                        element.as_deref(),
                    );
                    let show = !matches!(action, desktop::Action::Launch(_));
                    let offer = {
                        let (signals, run_id) = (signals.clone(), job.run_id.clone());
                        move |phase: desktop::Phase| {
                            let (Some(tx), true) = (&signals, show) else {
                                return;
                            };
                            let phase = match phase {
                                desktop::Phase::Waiting => AgentActingPhase::Waiting,
                                desktop::Phase::Acting => AgentActingPhase::Acting,
                            };
                            let _ = tx.send(Signal::Offer(Activity::AgentActing(
                                AgentActingActivity {
                                    run_id: run_id.clone(),
                                    project: project.clone(),
                                    step: step.clone(),
                                    phase,
                                },
                            )));
                        }
                    };
                    let focus_app = app.clone();
                    glow::act_started(desktop::monitor_of(hwnd));
                    crate::hover::set_hands_off(true);
                    let result = desktop::act(
                        hwnd,
                        job.pid.or(refs_pid),
                        action,
                        move |h| focus_on_main(&focus_app, h),
                        offer,
                    );
                    crate::hover::set_hands_off(false);
                    glow::act_ended();
                    if let Some(tx) = &signals {
                        let _ = tx.send(Signal::Retract(ACTING_KEY.into()));
                    }
                    result
                }
            };
            let audit = {
                let mut inner = agents.lock();
                inner.acting = None;
                inner.audit.clone()
            };
            if let Some(audit) = audit {
                audit.log(
                    &job.run_id,
                    "act",
                    actions::audit_fields(name, &job.args, &result),
                );
            }
            match &result {
                Ok(a) => log::info!("agent action {name} done: {}", a.what),
                Err(e) => log::info!("agent action {name} failed: {}", e.code()),
            }
            let reply = actions::reply(job.rpc_id, &result);
            if let Err(e) = job.pipe.write(reply.to_string().as_bytes()) {
                log::debug!("agent action reply not delivered: {e}");
            }
        });
    if let Err(e) = spawned {
        log::warn!("agent action thread: {e}");
    }
}

/// The window a desktop look names, resolved before asking: its label for the summary, and its process.
fn desktop_target(tool: &str, input: &Value) -> (Option<String>, Option<u32>) {
    if !describe::is_desktop(tool) && !describe::is_action(tool) {
        return (None, None);
    }
    let target = input
        .get("hwnd")
        .and_then(|v| v.as_i64().or_else(|| v.as_str()?.trim().parse().ok()))
        .and_then(desktop::identify);
    (
        target.as_ref().map(tools::target_label),
        target.map(|t| t.pid),
    )
}

/// A tool's input as the audit keeps it: typed text and draft bodies replaced by their size.
fn redact(tool: &str, input: &Value) -> Value {
    google_tools::redact(tool, &actions::redact(tool, input))
}

/// A claude.ai Gmail connector's tool (`mcp__claude_ai_Gmail__get_thread`): reading through it counts as reading
/// mail too.
fn is_connector_mail(tool: &str) -> bool {
    tool.strip_prefix("mcp__claude_ai_")
        .is_some_and(|rest| rest.to_ascii_lowercase().starts_with("gmail"))
}

impl Inner {
    /// Your answer to an ACP agent's permission request, on its stdin (2.9b).
    fn answer_acp(&mut self, run_id: &str, rpc_id: &Value, allow: bool) {
        let Some(slot) = self.slot(run_id) else {
            return;
        };
        if let (Some(client), Some(process)) = (slot.acp.as_mut(), slot.process.as_mut()) {
            if let Some(bytes) = client.answer(rpc_id, allow) {
                if let Err(e) = process.write(&bytes) {
                    log::debug!("agent run {run_id}: permission answer not delivered: {e}");
                }
            }
        }
    }

    /// How the message a Gmail read or reply names is called in this run (from its searches), for the approval.
    fn mail_label(&self, run_id: &str, input: &Value) -> Option<String> {
        let id = input
            .get("id")
            .or_else(|| input.get("replyToId"))
            .and_then(Value::as_str)?;
        self.mail_labels.get(run_id)?.get(id.trim()).cloned()
    }
}

struct Ask<'a> {
    run_id: &'a str,
    tool: &'a str,
    purpose: Purpose,
    /// For desktop looks and actions: the window's label and process.
    target: (Option<String>, Option<u32>),
    /// For actions on an element: its label from the run's latest `ui_tree`.
    element: Option<String>,
}

/// Asks about a request: `Some` (answered at once) when its kind is remembered for the run, else it waits as an
/// `agentApproval` activity.
fn ask(
    inner: &mut Inner,
    a: Ask<'_>,
    input: Value,
    reply: ReplyTo,
    after: &mut After,
) -> Option<(ReplyTo, Value, Option<u32>)> {
    let timeout = Duration::from_secs(inner.settings.approval_timeout_secs.max(10));
    // The audit's "asked" line never carries typed text: its summary is built from the redacted input.
    let logged_summary = describe::is_action(a.tool).then(|| {
        describe::action_summary(
            a.tool,
            &actions::redact(a.tool, &input),
            a.target.0.as_deref(),
            a.element.as_deref(),
        )
    });
    let slot = inner.slot(a.run_id)?;
    // A bounded read (256 KB) of the file a `Write` would replace, for the card's diff.
    let existing = preview::read_existing(a.tool, &input, &slot.run.cwd, &[]);
    let request = Request {
        run_id: a.run_id.to_string(),
        project: slot.run.project.clone(),
        cwd: slot.run.cwd.clone(),
        tool: a.tool.to_string(),
        input,
        reply,
        purpose: a.purpose,
        target: a.target.0,
        target_pid: a.target.1,
        element: a.element,
        routine_name: slot.run.routine.as_ref().map(|r| r.name.clone()),
        patient: slot.patient,
        existing,
    };
    match inner.approvals.ask(request, Instant::now(), timeout) {
        Asked::Remembered {
            reply,
            input,
            target_pid,
        } => {
            inner.audit(
                a.run_id,
                "decision",
                json!({ "allow": true, "by": "remembered" }),
            );
            Some((reply, input, target_pid))
        }
        Asked::Waiting(p) => {
            let activity = p.activity.clone();
            let by = match a.purpose {
                Purpose::Look => "cay",
                Purpose::Prompt => "claudeCode",
                Purpose::Agent => "agent",
            };
            inner.audit(
                a.run_id,
                "asked",
                json!({ "requestId": activity.request_id,
                        "summary": logged_summary.as_ref().unwrap_or(&activity.summary),
                        "risk": activity.risk, "by": by }),
            );
            if let Some(slot) = inner.slot(a.run_id) {
                let at = now_ms();
                let entry = slot.run.note(
                    AgentEvent::Approval {
                        request_id: activity.request_id,
                        tool: activity.tool.clone(),
                        summary: activity.summary.clone(),
                        risk: activity.risk,
                    },
                    at,
                );
                slot.run.set_waiting(true, at);
                after.outputs.push(AgentOutputPayload {
                    run_id: a.run_id.to_string(),
                    entry,
                });
            }
            log::debug!(
                "agent approval {} for {}: {}",
                activity.request_id,
                activity.tool,
                activity.summary
            );
            after
                .signals
                .push(Signal::Offer(Activity::AgentApproval(activity)));
            after.runs_changed = true;
            None
        }
    }
}

/// Runs an approved look on its own thread with the glow up, then answers the call and logs what was looked at
/// (never the image or the tree).
fn run_look(app: &AppHandle, job: LookJob) {
    if actions::short_name(&job.tool).is_some() {
        return run_act(app, job);
    }
    if google_tools::short_name(&job.tool).is_some() {
        return run_google(app, job);
    }
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("agent-look".into())
        .spawn(move || {
            let name = tools::short_name(&job.tool).unwrap_or_default();
            let result = match tools::parse(name, &job.args) {
                Ok(look) => {
                    glow::look_started(desktop::monitor_for(&look));
                    let result = desktop::run(look, job.pid);
                    glow::look_ended();
                    result
                }
                Err(why) => Err(desktop::LookError::Failed(why)),
            };
            let reply = tools::reply(job.rpc_id, &result);
            let agents = app.state::<Agents>();
            let audit = {
                let mut inner = agents.lock();
                // What the look saw, so actions can name its elements and points (2.7d).
                let now = Instant::now();
                match &result {
                    Ok(desktop::Outcome::Tree {
                        target, elements, ..
                    }) => inner.refs.put_tree(
                        &job.run_id,
                        target.hwnd as i64,
                        target.pid,
                        elements.clone(),
                        now,
                    ),
                    Ok(desktop::Outcome::Image {
                        image,
                        target: Some(target),
                    }) => {
                        let view = desktop::plan::View {
                            origin: image.origin,
                            scale: image.scale,
                            frame: image.origin,
                        };
                        inner
                            .refs
                            .put_view(&job.run_id, target.hwnd as i64, target.pid, view, now);
                    }
                    _ => {}
                }
                inner.audit.clone()
            };
            if let Some(audit) = audit {
                audit.log(&job.run_id, "look", tools::audit_fields(name, &result));
            }
            match &result {
                Ok(_) => log::debug!("agent look {name} done"),
                Err(e) => log::info!("agent look {name} failed: {}", e.code()),
            }
            if let Err(e) = job.pipe.write(reply.to_string().as_bytes()) {
                log::debug!("agent look reply not delivered: {e}");
            }
        });
    if let Err(e) = spawned {
        log::warn!("agent look thread: {e}");
    }
}

/// Runs an approved Google tool call on its own thread (network; never under the lock), then answers it, notes
/// what the run has seen (mail read, how messages are named) and logs counts and ids, never content.
fn run_google(app: &AppHandle, job: LookJob) {
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("agent-google".into())
        .spawn(move || {
            let name = google_tools::short_name(&job.tool).unwrap_or_default();
            let result = crate::google::tools::parse(name, &job.args)
                .map_err(|why| crate::google::ToolError {
                    code: "invalid",
                    message: why,
                })
                .and_then(|call| crate::google::run_tool(&app, &call));
            let reply = google_tools::reply(job.rpc_id, name, &result);
            let agents = app.state::<Agents>();
            {
                let mut inner = agents.lock();
                if let Ok(out) = &result {
                    let labels = inner.mail_labels.entry(job.run_id.clone()).or_default();
                    for l in &out.labels {
                        if labels.len() < MAX_MAIL_LABELS || labels.contains_key(&l.id) {
                            labels.insert(l.id.clone(), l.text.clone());
                        }
                    }
                    if out.read_mail && inner.read_mail.insert(job.run_id.clone()) {
                        log::info!(
                            "agent run {} read mail; its desktop actions ask again",
                            job.run_id
                        );
                    }
                }
                inner.audit(
                    &job.run_id,
                    "google",
                    google_tools::audit_fields(name, &result),
                );
            }
            match &result {
                Ok(_) => log::debug!("agent google {name} done"),
                Err(e) => log::info!("agent google {name} failed: {}", e.code),
            }
            if let Err(e) = job.pipe.write(reply.to_string().as_bytes()) {
                log::debug!("agent google reply not delivered: {e}");
            }
        });
    if let Err(e) = spawned {
        log::warn!("agent google thread: {e}");
    }
}

fn start_timer(app: AppHandle, stop: Arc<Handle>) -> AppResult<Watcher> {
    let signal = stop.clone();
    Watcher::spawn(
        "agent-timer",
        move || {
            let mut last = Instant::now();
            loop {
                // SAFETY: waits on our own event.
                if unsafe { WaitForSingleObject(stop.0, 1000) } != WAIT_TIMEOUT {
                    return;
                }
                let agents = app.state::<Agents>();
                let mut after = After::default();
                let now = Instant::now();
                let elapsed = now - last;
                last = now;
                {
                    let mut inner = agents.lock();
                    // "Approvals wait for me": a scheduled routine's approvals don't time out while you're away.
                    if inner.approvals.has_patient()
                        && (desktop::screen_locked() || desktop::idle_ms() >= AWAY_IDLE_MS)
                    {
                        inner.approvals.postpone(elapsed);
                    }
                    let expired = inner.approvals.expired(now);
                    if !expired.is_empty() {
                        inner.settle(expired, false, "timeout", &mut after);
                    }
                    stop_silent(&mut inner, now, &mut after);
                }
                after.run(&app, &agents);
            }
        },
        move || {
            // SAFETY: as in `start_server`.
            let _ = unsafe { SetEvent(signal.0) };
        },
    )
}

/// Stops ACP runs whose agent has gone quiet (`ACP_SILENCE`). Waiting on your approval or on a desktop action isn't
/// silence: the clock restarts once it's answered or done.
fn stop_silent(inner: &mut Inner, now: Instant, after: &mut After) {
    let quiet: Vec<String> = inner
        .slots
        .iter()
        .filter(|s| s.acp.is_some() && s.process.is_some() && s.run.is_live())
        .filter(|s| now.duration_since(s.heard) >= ACP_SILENCE)
        .map(|s| s.run.id.clone())
        .collect();
    for run_id in quiet {
        let acting = inner.acting.as_ref().is_some_and(|(r, _, _)| *r == run_id);
        if acting || inner.approvals.has_pending(&run_id) {
            if let Some(slot) = inner.slot(&run_id) {
                slot.heard = now;
            }
            continue;
        }
        let minutes = ACP_SILENCE.as_secs() / 60;
        log::warn!("agent run {run_id}: nothing from the agent for {minutes} minutes; stopping it");
        inner.audit(&run_id, "silent", json!({ "minutes": minutes }));
        note_error(
            inner,
            &run_id,
            format!("The agent sent nothing for {minutes} minutes, so Cay stopped it."),
            after,
        );
        stop_run(inner, &run_id, after);
    }
}

/// The `agent-routines` thread: one look at every routine per wake-up, sleeping until the next due time (at
/// most a minute, so sleep, resume and clock changes are noticed) or a change wakes it.
fn start_scheduler(app: AppHandle, stop: Arc<Handle>, wake: Arc<Handle>) -> AppResult<Watcher> {
    let signal = stop.clone();
    Watcher::spawn(
        "agent-routines",
        move || {
            let mut expected = now_ms();
            loop {
                let now = now_ms();
                if now > expected + RESUME_JUMP_MS {
                    log::info!(
                        "routines: the clock jumped {} s (sleep or a clock change)",
                        (now - expected) / 1000
                    );
                    app.state::<Agents>().lock().settle_until_ms = now + schedule::SETTLE;
                }
                let next = scheduler_tick(&app, now);
                let wait = (next - now).clamp(1000, SCHEDULER_TICK_MS);
                expected = now + wait;
                // SAFETY: waits on our own events.
                let woke = unsafe { WaitForMultipleObjects(&[stop.0, wake.0], false, wait as u32) };
                if woke == WAIT_OBJECT_0 {
                    return;
                }
                if woke.0 == WAIT_OBJECT_0.0 + 1 {
                    // SAFETY: our own manual-reset event.
                    let _ = unsafe { ResetEvent(wake.0) };
                    expected = now_ms();
                }
            }
        },
        move || {
            // SAFETY: as in `start_server`.
            let _ = unsafe { SetEvent(signal.0) };
        },
    )
}

/// Runs or skips what's due; returns when to look again.
fn scheduler_tick(app: &AppHandle, now: i64) -> i64 {
    let agents = app.state::<Agents>();
    let zone = routines::LocalZone;
    let mut next = now + SCHEDULER_TICK_MS;
    let mut fire = Vec::new();
    let mut after = After::default();
    {
        let mut guard = agents.lock();
        let inner = &mut *guard;
        let going: HashSet<String> = inner
            .slots
            .iter()
            .filter(|s| s.is_going())
            .filter_map(|s| s.run.routine.as_ref())
            .map(|r| r.id.clone())
            .collect();
        let mut scheduled = inner
            .slots
            .iter()
            .filter(|s| s.is_going())
            .filter(|s| {
                s.run
                    .routine
                    .as_ref()
                    .is_some_and(|r| r.trigger == RoutineTrigger::Schedule)
            })
            .count();
        let mut skips = Vec::new();
        let mut changed = false;
        for r in &mut inner.routines.file.routines {
            let times = Times {
                since_ms: r.schedule_since_ms,
                launched_ms: inner.launched_ms,
            };
            let check = Check {
                schedule: &r.schedule,
                times,
                enabled: r.enabled,
                if_missed: r.if_missed,
                last_due_ms: r.last_due_ms,
                settle_until_ms: inner.settle_until_ms,
                paused: inner.routines.file.paused,
                running: going.contains(&r.id),
                at_limit: scheduled >= schedule::MAX_SCHEDULED,
            };
            match schedule::check(&check, now, &zone) {
                Verdict::Idle => {}
                Verdict::Hold { until_ms } => next = next.min(until_ms),
                Verdict::Skip { due_ms, why } => skips.push((r.id.clone(), due_ms, why)),
                Verdict::Fire { due_ms, .. } if !Path::new(&r.cwd).is_dir() => {
                    skips.push((r.id.clone(), due_ms, SkipReason::FolderMissing))
                }
                Verdict::Fire { due_ms, late } => {
                    r.last_due_ms = due_ms;
                    scheduled += 1;
                    changed = true;
                    fire.push((r.id.clone(), r.name.clone(), due_ms, late));
                }
            }
            if let Some(n) = schedule::next(&r.schedule, times, now, &zone) {
                next = next.min(n);
            }
        }
        for (id, due_ms, why) in skips {
            inner.record_skip(&id, due_ms, why, now);
            changed = true;
        }
        if changed {
            inner.routines.save();
            after.routines_changed = true;
        }
    }
    after.run(app, &agents);
    for (id, name, due_ms, late) in fire {
        let started = run_routine(app, &id, RoutineTrigger::Schedule);
        let mut inner = agents.lock();
        match started {
            Ok(run_id) => {
                log::info!("routine {id} ({name}) started run {run_id} (late: {late})");
                inner.audit_routine(
                    "started",
                    json!({ "routine": id, "name": name, "dueMs": due_ms, "late": late,
                            "runId": run_id, "trigger": RoutineTrigger::Schedule }),
                );
            }
            Err(e) => {
                log::warn!("routine {id} ({name}) didn't start: {e}");
                inner.record_skip(&id, due_ms, SkipReason::StartFailed, now_ms());
                inner.routines.save();
                drop(inner);
                After {
                    routines_changed: true,
                    ..After::default()
                }
                .run(app, &agents);
            }
        }
    }
    next
}

/// Starts routine `id`'s run: an ordinary run with the routine's prompt, folder and limits.
fn run_routine(app: &AppHandle, id: &str, trigger: RoutineTrigger) -> AppResult<String> {
    let r = app
        .state::<Agents>()
        .lock()
        .routines
        .get(id)
        .cloned()
        .ok_or_else(|| AppError::AgentInvalid("That routine was deleted.".into()))?;
    let request = AgentStartRequest {
        prompt: r.prompt,
        cwd: r.cwd,
        model: r.model,
        max_turns: Some(r.max_turns),
        max_budget_usd: Some(r.max_budget_usd),
        claude_path: r.claude_path,
        mode: Some(r.mode),
        backend: Some(r.backend),
        // Routines take no attachments (2.10b).
        attachments: Vec::new(),
    };
    let routine = RoutineStart {
        tag: AgentRunRoutine {
            id: r.id,
            name: r.name,
            trigger,
        },
        wait_for_me: r.wait_for_me,
        desktop_tools: r.desktop_tools,
        google_tools: r.google_tools,
    };
    start_run(app, request, Some(routine))
}

/// Starts `slot`'s next process (the first, or a `--resume` follow-up) and writes `text` to it, with what's
/// `attached` (2.10b).
fn start_process(
    app: &AppHandle,
    inner: &mut Inner,
    run_id: &str,
    text: &str,
    attached: &attach::Prepared,
    after: &mut After,
) -> AppResult<()> {
    let exe_path = std::env::current_exe().map_err(|e| AppError::AgentStart(e.to_string()))?;
    let slot = inner.slot(run_id).ok_or(AppError::AgentRunGone)?;
    if let Some(old) = slot.process.take() {
        // The previous process already gave its result and is only exiting.
        old.job.kill();
    }
    slot.generation += 1;
    let generation = slot.generation;
    std::fs::create_dir_all(&slot.dir).map_err(|e| AppError::AgentStart(e.to_string()))?;
    if slot.run.backend != AgentBackend::Claude {
        return start_acp(app, inner, run_id, text, attached, &exe_path, after);
    }
    let config = slot.dir.join("mcp.json");
    let mcp = json!({ "mcpServers": { mcp::SERVER_NAME: {
        "type": "stdio",
        "command": exe_path.to_string_lossy(),
        "args": ["mcp"],
        "env": { bridge::TOKEN_ENV: slot.token, bridge::RUN_ENV: slot.run.id },
    } } });
    std::fs::write(&config, mcp.to_string()).map_err(|e| AppError::AgentStart(e.to_string()))?;
    if let Some(folder) = &slot.launch.attachments_dir {
        // `--add-dir` needs the folder to exist, even with nothing attached yet.
        std::fs::create_dir_all(folder).map_err(|e| AppError::AgentStart(e.to_string()))?;
    }
    attached.restore().map_err(AppError::AgentInvalid)?;
    slot.launch.resume = slot.run.session.clone().filter(|_| slot.generation > 1);
    // A mode chosen between messages (2.8e) starts this process.
    slot.launch.mode = slot.run.take_mode();

    let (line_app, exit_app) = (app.clone(), app.clone());
    let (line_id, exit_id) = (run_id.to_string(), run_id.to_string());
    let mut process = run::spawn(
        &slot.launch,
        &config,
        move |line| on_line(&line_app, &line_id, generation, line),
        move |code, stderr| on_exit(&exit_app, &exit_id, generation, code, &stderr),
    )
    .map_err(|e| AppError::AgentStart(format!("Couldn't start Claude Code: {e}")))?;
    let entry = slot.run.send_with(text, attached.list.clone(), now_ms());
    process
        .write(&stream::user_content(attached.content(text)))
        .map_err(|e| AppError::AgentStart(e.to_string()))?;
    slot.process = Some(process);
    after.outputs.push(AgentOutputPayload {
        run_id: run_id.to_string(),
        entry,
    });
    after.runs_changed = true;
    Ok(())
}

/// An ACP agent's process for a run (2.9b): `initialize`, the session (new, or resumed for a follow-up), then the
/// message. The agent's mode rules go in its environment for this process only (`backend.rs`).
fn start_acp(
    app: &AppHandle,
    inner: &mut Inner,
    run_id: &str,
    text: &str,
    attached: &attach::Prepared,
    cay_exe: &std::path::Path,
    after: &mut After,
) -> AppResult<()> {
    // A follow-up that resumes an ended run copies its files again (the folder went with the process).
    attached.restore().map_err(AppError::AgentInvalid)?;
    let slot = inner.slot(run_id).ok_or(AppError::AgentRunGone)?;
    let generation = slot.generation;
    let kind = slot.run.backend;
    let mode = slot.run.take_mode();
    slot.launch.mode = mode;
    let launch = match kind {
        AgentBackend::Opencode => {
            backend::opencode(&slot.launch.exe, mode, slot.launch.model.as_deref())
        }
        AgentBackend::Codex => {
            let (node, script) = slot
                .launch
                .exe
                .split_once('\n')
                .ok_or_else(|| AppError::AgentStart("codex-acp wasn't found".into()))?;
            backend::codex(node, script, mode, slot.launch.model.as_deref())
        }
        AgentBackend::Claude => return Err(AppError::AgentStart("not an ACP agent".into())),
    };
    let mcp_servers = json!([{
        "name": mcp::SERVER_NAME,
        "command": cay_exe.to_string_lossy(),
        "args": ["mcp"],
        "env": [
            { "name": bridge::TOKEN_ENV, "value": slot.token },
            { "name": bridge::RUN_ENV, "value": slot.run.id },
        ],
    }]);
    let (mut client, first) = acp::Client::new(acp::Setup {
        cwd: slot.launch.cwd.clone(),
        mcp_servers,
        resume: slot.run.session.clone().filter(|_| generation > 1),
        config: launch.config.clone(),
        // The watchdog (D48): the modes that promise to ask about every edit and command.
        ask_mode: matches!(mode, AgentMode::Default | AgentMode::Plan),
    });
    let (line_app, exit_app) = (app.clone(), app.clone());
    let (line_id, exit_id) = (run_id.to_string(), run_id.to_string());
    let mut process = run::spawn_command(
        &launch.exe,
        &launch.args,
        &launch.env,
        &slot.launch.cwd,
        move |line| on_line(&line_app, &line_id, generation, line),
        move |code, stderr| on_exit(&exit_app, &exit_id, generation, code, &stderr),
    )
    .map_err(|e| AppError::AgentStart(format!("Couldn't start {}: {e}", kind.display())))?;
    process
        .write(&first)
        .map_err(|e| AppError::AgentStart(e.to_string()))?;
    let entry = slot.run.send_with(text, attached.list.clone(), now_ms());
    // ACP has no system prompt: the first message carries what Claude Code gets with `--append-system-prompt`.
    let message = if generation == 1 {
        format!("{}\n\n{text}", backend::ACP_GUIDANCE)
    } else {
        text.to_string()
    };
    // Images go as images only if the agent says it takes them, decided when the prompt is sent (2.10b.2).
    let outs = client.prompt_with(&message, attached.acp_items());
    slot.acp = Some(client);
    slot.process = Some(process);
    slot.heard = Instant::now();
    after.outputs.push(AgentOutputPayload {
        run_id: run_id.to_string(),
        entry,
    });
    after.runs_changed = true;
    acp_outs(inner, run_id, outs, after);
    Ok(())
}

/// What an ACP client produced, carried out under the lock: lines to the agent, events into the run, permission
/// requests to the island, the watchdog's stop.
fn acp_outs(inner: &mut Inner, run_id: &str, outs: Vec<acp::Out>, after: &mut After) {
    for out in outs {
        match out {
            acp::Out::Write(bytes) => {
                let written = inner
                    .slot(run_id)
                    .and_then(|s| s.process.as_mut())
                    .map(|p| p.write(&bytes));
                if let Some(Err(e)) = written {
                    log::debug!("agent run {run_id}: line not delivered: {e}");
                }
            }
            acp::Out::Event(event) => apply_events(inner, run_id, vec![event], after),
            acp::Out::Permission {
                rpc_id,
                tool,
                input,
                ..
            } => {
                inner.audit(
                    run_id,
                    "approval",
                    json!({ "tool": tool, "input": audit::input_for_log(&redact(&tool, &input)) }),
                );
                let asked = Ask {
                    run_id,
                    tool: &tool,
                    purpose: Purpose::Agent,
                    target: desktop_target(&tool, &input),
                    element: None,
                };
                let reply = ReplyTo { conn: 0, rpc_id };
                if let Some((reply, _, _)) = ask(inner, asked, input, reply, after) {
                    inner.answer_acp(run_id, &reply.rpc_id, true);
                }
            }
            acp::Out::Unasked { tool, summary } => {
                log::warn!("agent run {run_id}: {tool} ran without asking; stopping the run");
                inner.audit(
                    run_id,
                    "unasked",
                    json!({ "tool": tool, "summary": audit::input_for_log(&json!(summary)) }),
                );
                note_error(
                    inner,
                    run_id,
                    format!(
                        "The agent ran a step without asking, although this run asks first ({tool}: {summary}). \
                         Cay stopped it."
                    ),
                    after,
                );
                stop_run(inner, run_id, after);
            }
            acp::Out::Notice(message) => note_error(inner, run_id, message, after),
        }
    }
}

fn note_error(inner: &mut Inner, run_id: &str, message: String, after: &mut After) {
    if let Some(slot) = inner.slot(run_id) {
        let entry = slot.run.note(AgentEvent::Error { message }, now_ms());
        after.outputs.push(AgentOutputPayload {
            run_id: run_id.to_string(),
            entry,
        });
        after.runs_changed = true;
    }
}

fn on_line(app: &AppHandle, run_id: &str, generation: u32, line: &[u8]) {
    let agents = app.state::<Agents>();
    let mut after = After::default();
    {
        let mut inner = agents.lock();
        let Some(slot) = inner.slot(run_id).filter(|s| s.generation == generation) else {
            return;
        };
        slot.heard = Instant::now();
        match slot.acp.as_mut() {
            Some(client) => {
                let outs = client.on_line(line);
                let images = client.accepts_images();
                if slot.run.images != images {
                    // What the agent takes, for the Attach button (2.10b.2).
                    slot.run.images = images;
                    after.runs_changed = true;
                }
                acp_outs(&mut inner, run_id, outs, &mut after);
            }
            None => {
                let events = stream::parse(line);
                if events.is_empty() {
                    return;
                }
                apply_events(&mut inner, run_id, events, &mut after);
            }
        }
    }
    after.run(app, &agents);
}

/// A run's events (from stream-json or ACP) into its lifecycle, the audit and the transcript.
fn apply_events(inner: &mut Inner, run_id: &str, events: Vec<StreamEvent>, after: &mut After) {
    {
        let mut finished = false;
        let mut tools = Vec::new();
        let mut mail_read_now = false;
        // Drafts alone change nothing the island shows: no `agent:runs` for them (tens a second while Claude writes).
        let only_drafts = events
            .iter()
            .all(|e| matches!(e, stream::StreamEvent::TextDelta(_)));
        let Some(slot) = inner.slot(run_id) else {
            return;
        };
        for event in events {
            if let stream::StreamEvent::Tool { name, input, .. } = &event {
                tools.push(
                    json!({ "tool": name, "input": audit::input_for_log(&redact(name, input)) }),
                );
                if is_connector_mail(name) {
                    mail_read_now = true;
                }
            }
            let at = now_ms();
            let applied = slot.run.apply(event, at);
            if applied.close_stdin {
                if let Some(process) = &mut slot.process {
                    process.stdin = None;
                }
            }
            finished |= applied.finished;
            if let Some(text) = applied.draft {
                after.drafts.push(crate::ipc::AgentTextDraft {
                    run_id: run_id.to_string(),
                    text,
                });
            }
            for entry in applied.entries {
                after.outputs.push(AgentOutputPayload {
                    run_id: run_id.to_string(),
                    entry,
                });
            }
        }
        for tool in tools {
            inner.audit(run_id, "tool", tool);
        }
        if mail_read_now && inner.read_mail.insert(run_id.to_string()) {
            log::info!(
                "agent run {run_id} read mail through a connector; its desktop actions ask again"
            );
            inner.audit(run_id, "readMail", json!({ "by": "connector" }));
        }
        if finished {
            inner.finished(run_id, after);
        }
        after.runs_changed |= !only_drafts;
    }
}

fn on_exit(app: &AppHandle, run_id: &str, generation: u32, code: Option<i32>, stderr: &str) {
    let agents = app.state::<Agents>();
    let mut after = After::default();
    {
        let mut inner = agents.lock();
        let Some(slot) = inner.slot(run_id).filter(|s| s.generation == generation) else {
            return;
        };
        slot.process = None;
        slot.acp = None;
        let _ = std::fs::remove_dir_all(&slot.dir);
        let applied = slot.run.exited(code, stderr, now_ms());
        for entry in applied.entries {
            after.outputs.push(AgentOutputPayload {
                run_id: run_id.to_string(),
                entry,
            });
        }
        log::info!("agent run {run_id} exited (code {code:?})");
        if applied.finished {
            inner.finished(run_id, &mut after);
        }
        after.runs_changed = true;
    }
    after.run(app, &agents);
}

fn stop_run(inner: &mut Inner, run_id: &str, after: &mut After) {
    if inner
        .acting
        .as_ref()
        .is_some_and(|(run, _, _)| run == run_id)
    {
        desktop::abort_action();
    }
    let Some(slot) = inner.slot(run_id) else {
        return;
    };
    if let Some(process) = &mut slot.process {
        slot.run.stop();
        // An ACP agent hears that its open permission requests are cancelled before it's ended (2.9b).
        if let Some(client) = slot.acp.as_mut() {
            for line in client.cancel_all() {
                let _ = process.write(&line);
            }
        }
        process.stdin = None;
        process.job.kill();
        log::info!("agent run {run_id} stopped");
    }
    let pending = inner.approvals.end_run(run_id);
    inner.settle(pending, false, "runEnded", after);
    inner.audit(run_id, "stop", json!({}));
}

fn stop_all(app: &AppHandle) {
    let agents = app.state::<Agents>();
    let mut after = After::default();
    {
        let mut inner = agents.lock();
        let live: Vec<String> = inner
            .slots
            .iter()
            .filter(|s| s.process.is_some())
            .map(|s| s.run.id.clone())
            .collect();
        for run_id in live {
            stop_run(&mut inner, &run_id, &mut after);
        }
    }
    after.run(app, &agents);
}

fn installs(agents: &Agents) -> Vec<ClaudeInstall> {
    // One probe at a time: a command arriving during the startup probe waits for it instead of repeating it.
    static PROBING: Mutex<()> = Mutex::new(());
    let _probing = PROBING.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(found) = agents.lock().installs.clone() {
        return found;
    }
    // Probing runs `claude.exe --version` per install; do it without holding the lock.
    let found = installs::find();
    log::info!("Claude Code installs: {found:?}");
    agents.lock().installs = Some(found.clone());
    found
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStartRequest {
    prompt: String,
    cwd: String,
    model: Option<String>,
    max_turns: Option<u32>,
    max_budget_usd: Option<f64>,
    claude_path: Option<String>,
    /// The permission mode (2.8e); `None` uses `agent.defaultMode` (Auto).
    #[serde(default)]
    mode: Option<AgentMode>,
    /// The agent (2.9b); `None` is Claude Code.
    #[serde(default)]
    backend: Option<AgentBackend>,
    /// Files to attach to the first message (2.10b): paths, never bytes.
    #[serde(default)]
    attachments: Vec<String>,
}

#[tauri::command]
pub async fn agent_start(app: AppHandle, request: AgentStartRequest) -> AppResult<AgentStarted> {
    let run_id = start_run(&app, request, None)?;
    Ok(AgentStarted { run_id })
}

#[tauri::command]
/// The agents the command window can pick (2.9c): which are installed, their default model and the models they list.
/// OpenCode's list comes from `opencode models` the first time (a few seconds), then from memory.
pub async fn agent_backends(app: AppHandle) -> AppResult<AgentBackendsPayload> {
    tauri::async_runtime::spawn_blocking(move || backends_payload(&app))
        .await
        .map_err(|e| AppError::AgentStart(e.to_string()))
}

fn backends_payload(app: &AppHandle) -> AgentBackendsPayload {
    let agents = app.state::<Agents>();
    let (settings, cached) = {
        let inner = agents.lock();
        (inner.settings.clone(), inner.models.clone())
    };
    let path_of = |b: AgentBackend| {
        settings
            .backends
            .of(b)
            .and_then(|s| s.path.clone())
            .filter(|p| !p.trim().is_empty())
    };
    let model_of = |b: AgentBackend| settings.backends.of(b).and_then(|s| s.model.clone());

    let claude = AgentBackendInfo {
        backend: AgentBackend::Claude,
        available: !installs(&agents).is_empty(),
        install: Some("claude.ai/code".into()),
        default_model: settings.model.clone(),
        models: Vec::new(),
    };
    let opencode_exe = backend::opencode_exe(path_of(AgentBackend::Opencode).as_deref());
    let opencode_models = match (cached.get(&AgentBackend::Opencode), &opencode_exe) {
        (Some(models), _) => models.clone(),
        (None, Some(exe)) => {
            let models = backend::output(exe, &["models"], Duration::from_secs(20))
                .map(|out| backend::opencode_models(&out))
                .unwrap_or_default();
            if !models.is_empty() {
                agents
                    .lock()
                    .models
                    .insert(AgentBackend::Opencode, models.clone());
            }
            models
        }
        (None, None) => Vec::new(),
    };
    let opencode = AgentBackendInfo {
        backend: AgentBackend::Opencode,
        available: opencode_exe.is_some(),
        install: Some("npm install -g opencode-ai".into()),
        default_model: model_of(AgentBackend::Opencode),
        models: opencode_models,
    };
    let codex = AgentBackendInfo {
        backend: AgentBackend::Codex,
        available: backend::codex_adapter(path_of(AgentBackend::Codex).as_deref()).is_some(),
        install: Some("npm install -g @agentclientprotocol/codex-acp".into()),
        default_model: model_of(AgentBackend::Codex),
        // Codex keeps its own list; read fresh each time (it refreshes it as you use Codex).
        models: backend::codex_models_cache()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|text| backend::codex_models(&text))
            .unwrap_or_default(),
    };
    let backends = [claude, opencode, codex]
        .into_iter()
        .map(|mut b| {
            if b.available {
                b.install = None;
            }
            b
        })
        .collect();
    AgentBackendsPayload {
        default_backend: settings.default_backend,
        backends,
    }
}

#[tauri::command]
pub fn routines_list(agents: State<'_, Agents>) -> RoutinesPayload {
    agents.routines_payload()
}

/// Saves a new routine (no `id`) or an edit, checked in Rust.
#[tauri::command]
pub fn routine_save(
    app: AppHandle,
    agents: State<'_, Agents>,
    routine: RoutineDraft,
) -> AppResult<Routine> {
    let saved =
        {
            let mut inner = agents.lock();
            let existing =
                match &routine.id {
                    Some(id) => Some(inner.routines.get(id).cloned().ok_or_else(|| {
                        AppError::AgentInvalid("That routine was deleted.".into())
                    })?),
                    None => None,
                };
            let folder = |p: &str| Path::new(p).is_dir().then(|| folder::long_path(p));
            let saved = routines::validate(routine, existing.as_ref(), now_ms(), folder)
                .map_err(AppError::AgentInvalid)?;
            inner
                .routines
                .upsert(saved.clone())
                .map_err(AppError::AgentInvalid)?;
            inner.routines.save();
            inner.wake_scheduler();
            log::info!("routine {} ({}) saved", saved.id, saved.name);
            saved
        };
    After {
        routines_changed: true,
        ..After::default()
    }
    .run(&app, &agents);
    Ok(saved)
}

/// Deletes a routine. A run it started keeps going (it's an ordinary run).
#[tauri::command]
pub fn routine_delete(app: AppHandle, agents: State<'_, Agents>, id: String) -> AppResult<()> {
    {
        let mut inner = agents.lock();
        if !inner.routines.delete(&id) {
            return Err(AppError::AgentInvalid(
                "That routine was already deleted.".into(),
            ));
        }
        inner.routines.save();
        inner.wake_scheduler();
        log::info!("routine {id} deleted");
    }
    After {
        routines_changed: true,
        ..After::default()
    }
    .run(&app, &agents);
    Ok(())
}

/// Run now (works whether or not its schedule is on).
#[tauri::command]
pub async fn routine_run(app: AppHandle, id: String) -> AppResult<AgentStarted> {
    if app.state::<Agents>().lock().running_run_of(&id).is_some() {
        return Err(AppError::AgentInvalid(
            "This routine is already running.".into(),
        ));
    }
    let run_id = run_routine(&app, &id, RoutineTrigger::Manual)?;
    app.state::<Agents>().lock().audit_routine(
        "started",
        json!({ "routine": id, "runId": run_id, "trigger": RoutineTrigger::Manual }),
    );
    Ok(AgentStarted { run_id })
}

/// Pauses or resumes every schedule (the tray's "Pause routines" and the command window). Due times while
/// paused are skipped, not caught up.
#[tauri::command]
pub fn routines_pause(app: AppHandle, paused: bool) {
    set_routines_paused(&app, paused);
}

pub fn routines_paused(app: &AppHandle) -> bool {
    app.state::<Agents>().lock().routines.file.paused
}

pub fn set_routines_paused(app: &AppHandle, paused: bool) {
    let agents = app.state::<Agents>();
    {
        let mut inner = agents.lock();
        if inner.routines.file.paused != paused {
            inner.routines.file.paused = paused;
            inner.routines.save();
            log::info!("routines paused: {paused}");
        }
        inner.wake_scheduler();
    }
    After {
        routines_changed: true,
        ..After::default()
    }
    .run(app, &agents);
}

/// How a routine's run differs from a new task's: its name and trigger, and its three switches.
struct RoutineStart {
    tag: AgentRunRoutine,
    wait_for_me: bool,
    desktop_tools: bool,
    google_tools: bool,
}

/// Starts a run (from the new-task form, or a routine). Blocks while Claude Code installs are probed the
/// first time.
fn start_run(
    app: &AppHandle,
    request: AgentStartRequest,
    routine: Option<RoutineStart>,
) -> AppResult<String> {
    let agents = app.state::<Agents>();
    let prompt = request.prompt.trim().to_string();
    if prompt.is_empty() || prompt.len() > MAX_PROMPT {
        return Err(AppError::AgentInvalid("Type an instruction first.".into()));
    }
    if !std::path::Path::new(&request.cwd).is_dir() {
        return Err(AppError::AgentInvalid("That folder doesn't exist.".into()));
    }
    let request = AgentStartRequest {
        cwd: folder::long_path(&request.cwd),
        ..request
    };
    let settings = agents.lock().settings.clone();
    let kind = request.backend.unwrap_or(settings.default_backend);
    let configured_path = settings
        .backends
        .of(kind)
        .and_then(|b| b.path.clone())
        .filter(|p| !p.trim().is_empty());
    let exe = match kind {
        AgentBackend::Claude => {
            let found = installs(&agents);
            let configured = request.claude_path.clone().or(settings.claude_path.clone());
            installs::pick(&found, configured.as_deref()).ok_or(AppError::AgentNoClaude)?
        }
        // Node and the adapter's script, kept together in `exe` for `start_acp`.
        AgentBackend::Codex => backend::codex_adapter(configured_path.as_deref())
            .map(|(node, script)| format!("{}\n{}", node.display(), script.display()))
            .ok_or_else(|| {
                AppError::AgentInvalid(
                    "Codex's ACP adapter isn't installed (npm install -g @agentclientprotocol/codex-acp), so it \
                     can't run this."
                        .into(),
                )
            })?,
        AgentBackend::Opencode => backend::opencode_exe(configured_path.as_deref())
            .map(|p| p.to_string_lossy().into_owned())
            .ok_or_else(|| {
                AppError::AgentInvalid(
                    "OpenCode isn't installed (npm install -g opencode-ai), so it can't run this."
                        .into(),
                )
            })?,
    };
    let at = now_ms();
    let run_id = audit::run_id(at);
    // `agent.model` is Claude Code's; another agent uses `agent.backends.<agent>.model`, else its own default.
    let model = match kind {
        AgentBackend::Claude => request.model.clone().or(settings.model.clone()),
        other => request
            .model
            .clone()
            .filter(|m| !m.trim().is_empty())
            .or_else(|| settings.backends.of(other).and_then(|b| b.model.clone())),
    }
    .filter(|m| !m.is_empty());
    let dir = agents
        .lock()
        .audit
        .as_ref()
        .map(|a| a.dir().join("runs").join(&run_id))
        .unwrap_or_else(|| std::env::temp_dir().join(&run_id));
    let attachments_dir = dir.join(attachments::FOLDER);
    let attached = if request.attachments.is_empty() {
        attach::Prepared::default()
    } else if routine.is_some() {
        return Err(AppError::AgentInvalid(
            "Routines take no attachments.".into(),
        ));
    } else {
        // ACP agents also get a copy of each image, linked when the agent doesn't take images (2.10b.2).
        let copy_images = kind != AgentBackend::Claude;
        attach::prepare(&attachments_dir, &request.attachments, copy_images).map_err(|e| {
            let _ = std::fs::remove_dir_all(&dir);
            AppError::AgentInvalid(e)
        })?
    };
    let launch = run::Launch {
        exe: exe.clone(),
        cwd: request.cwd.clone(),
        model: model.clone(),
        max_turns: request.max_turns.unwrap_or(settings.max_turns),
        max_budget_usd: request.max_budget_usd.or(settings.max_budget_usd),
        resume: None,
        strict_mcp_config: settings.strict_mcp_config,
        user_settings: settings.user_settings,
        name: routine.as_ref().map(|r| r.tag.name.clone()),
        desktop_tools: routine.as_ref().is_none_or(|r| r.desktop_tools),
        google_tools: routine.as_ref().is_none_or(|r| r.google_tools),
        mode: request.mode.unwrap_or(settings.default_mode),
        // Only the copies Cay makes for this run (`--add-dir` for Claude Code); routines take no attachments.
        attachments_dir: routine
            .is_none()
            .then(|| attachments_dir.to_string_lossy().into_owned()),
    };
    let mut after = After::default();
    let result = {
        let mut inner = agents.lock();
        inner.audit(
            &run_id,
            "start",
            json!({
                "prompt": prompt, "cwd": request.cwd, "exe": exe, "model": model, "backend": kind,
                "maxTurns": launch.max_turns, "maxBudgetUsd": launch.max_budget_usd,
                "routine": routine.as_ref().map(|r| json!({ "id": r.tag.id, "name": r.tag.name })),
                "trigger": routine.as_ref().map(|r| r.tag.trigger),
                "desktopTools": launch.desktop_tools,
                "googleTools": launch.google_tools,
                "mode": launch.mode,
                "attachments": attachments::for_audit(&attached.list),
            }),
        );
        let mut run = Run::new(run_id.clone(), request.cwd, model, at);
        run.backend = kind;
        run.start_in(launch.mode);
        run.routine = routine.as_ref().map(|r| r.tag.clone());
        let patient = routine
            .as_ref()
            .is_some_and(|r| r.wait_for_me && r.tag.trigger == RoutineTrigger::Schedule);
        inner.slots.push(Slot {
            run,
            token: audit::random_hex(16),
            launch,
            process: None,
            acp: None,
            heard: Instant::now(),
            generation: 0,
            dir,
            patient,
        });
        if let Some(r) = inner
            .routines
            .get_mut(routine.as_ref().map_or("", |r| &r.tag.id))
        {
            r.last_run = Some(RoutineLastRun {
                run_id: run_id.clone(),
                trigger: routine
                    .as_ref()
                    .map_or(RoutineTrigger::Manual, |r| r.tag.trigger),
                at_ms: at,
                status: None,
                reason: None,
                cost_usd: None,
            });
            inner.routines.save();
            after.routines_changed = true;
        }
        let result = start_process(app, &mut inner, &run_id, &prompt, &attached, &mut after);
        if let Err(e) = &result {
            log::warn!("agent run {run_id} didn't start: {e}");
            if let Some(slot) = inner.slot(&run_id) {
                let applied = slot.run.exited(None, &e.to_string(), now_ms());
                for entry in applied.entries {
                    after.outputs.push(AgentOutputPayload {
                        run_id: run_id.clone(),
                        entry,
                    });
                }
            }
            after.runs_changed = true;
        }
        result
    };
    after.run(app, &agents);
    result?;
    log::info!("agent run {run_id} started");
    Ok(run_id)
}

#[tauri::command]
/// A follow-up: queued into the running process, or resumes the finished session in a new one.
pub async fn agent_send(
    app: AppHandle,
    agents: State<'_, Agents>,
    run_id: String,
    text: String,
    attachments: Option<Vec<String>>,
) -> AppResult<()> {
    let text = text.trim().to_string();
    if text.is_empty() || text.len() > MAX_PROMPT {
        return Err(AppError::AgentInvalid("Type a message first.".into()));
    }
    let attachments = attachments.unwrap_or_default();
    let attached = if attachments.is_empty() {
        attach::Prepared::default()
    } else {
        let folder = {
            let mut inner = agents.lock();
            let slot = inner.slot(&run_id).ok_or(AppError::AgentRunGone)?;
            let copy_images = slot.run.backend != AgentBackend::Claude;
            slot.launch
                .attachments_dir
                .clone()
                .map(|f| (f, copy_images))
        };
        let (folder, copy_images) =
            folder.ok_or_else(|| AppError::AgentInvalid("Routines take no attachments.".into()))?;
        tauri::async_runtime::spawn_blocking(move || {
            attach::prepare(std::path::Path::new(&folder), &attachments, copy_images)
        })
        .await
        .map_err(|e| AppError::AgentStart(e.to_string()))?
        .map_err(AppError::AgentInvalid)?
    };
    let mut after = After::default();
    let result = {
        let mut inner = agents.lock();
        inner.audit(
            &run_id,
            "followUp",
            json!({ "text": text, "attachments": attachments::for_audit(&attached.list) }),
        );
        let slot = inner.slot(&run_id).ok_or(AppError::AgentRunGone)?;
        let writable =
            slot.run.is_live() && slot.process.as_ref().is_some_and(|p| p.stdin.is_some());
        if writable && slot.acp.is_some() {
            attached.restore().map_err(AppError::AgentInvalid)?;
            let entry = slot.run.send_with(&text, attached.list.clone(), now_ms());
            let outs = slot
                .acp
                .as_mut()
                .map(|c| c.prompt_with(&text, attached.acp_items()))
                .unwrap_or_default();
            after.outputs.push(AgentOutputPayload {
                run_id: run_id.clone(),
                entry,
            });
            after.runs_changed = true;
            acp_outs(&mut inner, &run_id, outs, &mut after);
            Ok(())
        } else if writable {
            attached.restore().map_err(AppError::AgentInvalid)?;
            let entry = slot.run.send_with(&text, attached.list.clone(), now_ms());
            let written = slot
                .process
                .as_mut()
                .map(|p| p.write(&stream::user_content(attached.content(&text))));
            after.outputs.push(AgentOutputPayload {
                run_id: run_id.clone(),
                entry,
            });
            after.runs_changed = true;
            match written {
                Some(Ok(())) => Ok(()),
                _ => Err(AppError::AgentStart("Claude Code stopped reading.".into())),
            }
        } else if slot.run.session.is_none() {
            Err(AppError::AgentInvalid(
                "This run never started a session, so it can't continue. Start a new one.".into(),
            ))
        } else {
            start_process(&app, &mut inner, &run_id, &text, &attached, &mut after)
        }
    };
    after.run(&app, &agents);
    result
}

#[tauri::command]
/// Your choice of permission mode for a run (2.8e): sent to the running session at once (`set_permission_mode`), or
/// kept for your next message when no process runs. Recorded in the audit.
pub fn agent_set_mode(
    app: AppHandle,
    agents: State<'_, Agents>,
    run_id: String,
    mode: AgentMode,
) -> AppResult<()> {
    let mut after = After::default();
    let result = {
        let mut inner = agents.lock();
        let slot = inner.slot(&run_id).ok_or(AppError::AgentRunGone)?;
        let from = slot.run.mode();
        // An ACP agent's rules are set when its process starts, so its mode changes with your next message.
        let writable = slot.run.backend == AgentBackend::Claude
            && slot.run.is_live()
            && slot.process.as_ref().is_some_and(|p| p.stdin.is_some());
        let at = now_ms();
        let applies = if writable {
            let request_id = format!("mode-{at}");
            let written = slot
                .process
                .as_mut()
                .map(|p| p.write(&mode::set_request(&request_id, mode)));
            match written {
                Some(Ok(())) => {
                    slot.run.switching(mode, at);
                    "now"
                }
                _ => return Err(AppError::AgentStart("Claude Code stopped reading.".into())),
            }
        } else {
            slot.run.choose_next(mode, at);
            "nextMessage"
        };
        inner.audit(
            &run_id,
            "mode",
            json!({ "from": from, "to": mode, "applies": applies, "by": "user" }),
        );
        after.runs_changed = true;
        Ok(())
    };
    after.run(&app, &agents);
    result
}

#[tauri::command]
/// The chip's click (2.8e): the mode after the one shown (or the one waiting for your next message), like Claude
/// Code's Shift+Tab: Ask → Accept edits → Plan → Auto.
pub fn agent_cycle_mode(
    app: AppHandle,
    agents: State<'_, Agents>,
    run_id: String,
) -> AppResult<()> {
    let current = {
        let mut inner = agents.lock();
        let slot = inner.slot(&run_id).ok_or(AppError::AgentRunGone)?;
        let run = slot.run.ipc(vec![]);
        run.next_mode.unwrap_or(run.mode)
    };
    agent_set_mode(app, agents, run_id, mode::next(current))
}

#[tauri::command]
pub fn agent_stop(app: AppHandle, agents: State<'_, Agents>, run_id: String) -> AppResult<()> {
    let mut after = After::default();
    {
        let mut inner = agents.lock();
        inner.slot(&run_id).ok_or(AppError::AgentRunGone)?;
        stop_run(&mut inner, &run_id, &mut after);
    }
    after.run(&app, &agents);
    Ok(())
}

#[tauri::command]
/// Removes an ended run from the list (and its "finished" notice). A live run must be stopped first.
pub fn agent_remove(app: AppHandle, agents: State<'_, Agents>, run_id: String) -> AppResult<()> {
    let mut after = After::default();
    {
        let mut inner = agents.lock();
        let slot = inner.slot(&run_id).ok_or(AppError::AgentRunGone)?;
        if slot.run.is_live() || slot.process.is_some() {
            return Err(AppError::AgentInvalid(
                "Stop the run before removing it.".into(),
            ));
        }
        inner.slots.retain(|s| s.run.id != run_id);
        after
            .signals
            .push(Signal::Retract(format!("agent:{run_id}")));
        after.runs_changed = true;
    }
    after.run(&app, &agents);
    Ok(())
}

#[tauri::command]
pub fn agent_runs(agents: State<'_, Agents>) -> AgentRunsPayload {
    agents.payload()
}

/// Opens a link from a transcript in the browser (2.10a): https only (`links::checked`).
#[tauri::command]
pub fn agent_open_link(url: String) -> AppResult<()> {
    let url = links::checked(&url).map_err(AppError::AgentInvalid)?;
    crate::desktop::open_link(&url).map_err(|e| AppError::AgentInvalid(links::why(e)))
}

/// Ctrl+V in the command window (2.10b.3): the clipboard's files, or its image saved as a file; empty for text.
#[tauri::command]
pub async fn agent_paste_attachments(
    agents: State<'_, Agents>,
) -> AppResult<Vec<crate::ipc::AttachmentPick>> {
    let folder = agents
        .lock()
        .audit
        .as_ref()
        .map(|a| a.dir().join(clipboard::PASTED))
        .unwrap_or_else(|| std::env::temp_dir().join("cay-pasted"));
    tauri::async_runtime::spawn_blocking(move || clipboard::paste(&folder))
        .await
        .map_err(|e| AppError::AgentInvalid(e.to_string()))?
        .map_err(AppError::AgentInvalid)
}

/// Files dropped on the command window (2.10b.3): their names, kinds and sizes for the chips.
#[tauri::command]
pub async fn agent_describe_attachments(
    paths: Vec<String>,
) -> AppResult<Vec<crate::ipc::AttachmentPick>> {
    tauri::async_runtime::spawn_blocking(move || clipboard::describe(&paths))
        .await
        .map_err(|e| AppError::AgentInvalid(e.to_string()))?
        .map_err(AppError::AgentInvalid)
}

#[tauri::command]
pub fn agent_transcript(agents: State<'_, Agents>, run_id: String) -> AppResult<Vec<AgentEntry>> {
    let mut inner = agents.lock();
    Ok(inner
        .slot(&run_id)
        .ok_or(AppError::AgentRunGone)?
        .run
        .transcript())
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Decision {
    Allow,
    Deny,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Remember {
    Once,
    Run,
}

#[tauri::command]
/// The whole diff or command of a waiting approval (2.8b), for the command window; `None` once it's answered or for
/// tools without one.
pub fn agent_approval_preview(
    agents: State<'_, Agents>,
    request_id: u32,
) -> Option<crate::ipc::ApprovalPreview> {
    agents.lock().approvals.preview(request_id)
}

#[tauri::command]
pub fn agent_decide(
    app: AppHandle,
    agents: State<'_, Agents>,
    request_id: u32,
    decision: Decision,
    remember: Remember,
) -> AppResult<()> {
    let allow = matches!(decision, Decision::Allow);
    let mut after = After::default();
    {
        let mut inner = agents.lock();
        // Questions and plans are answered with `agent_answer` / `agent_plan`, never a bare allow; Deny skips them.
        if allow
            && inner
                .approvals
                .tool_of(request_id)
                .is_some_and(interact::is_interactive)
        {
            return Err(AppError::AgentInvalid(
                "Answer this in Cay instead of allowing it.".into(),
            ));
        }
        let p = inner
            .approvals
            .decide(request_id, allow, matches!(remember, Remember::Run))
            .ok_or(AppError::AgentApprovalGone)?;
        log::debug!("agent approval {request_id}: allow {allow}");
        inner.settle(vec![p], allow, "user", &mut after);
    }
    after.run(&app, &agents);
    Ok(())
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionAnswer {
    pub question: String,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub other: Option<String>,
}

#[tauri::command]
/// Your answer to one question of a run's waiting `AskUserQuestion` (2.8c). The card shows the answers so far; once
/// every question has one, Claude gets them (allow with `updatedInput`). Errors `agentInvalid`, `agentApprovalGone`.
pub fn agent_answer(
    app: AppHandle,
    agents: State<'_, Agents>,
    request_id: u32,
    answer: QuestionAnswer,
) -> AppResult<()> {
    let mut after = After::default();
    {
        let mut inner = agents.lock();
        let answered = inner.approvals.answer(
            request_id,
            &answer.question,
            &answer.labels,
            answer.other.as_deref(),
        );
        match answered {
            Ok(approvals::Answered::Partial(card)) => {
                after
                    .signals
                    .push(Signal::Offer(Activity::AgentApproval(card)));
                after.runs_changed = true;
            }
            Ok(approvals::Answered::Complete(p)) => {
                log::debug!("agent question {request_id}: answered");
                inner.settle(vec![p], true, "answered", &mut after);
            }
            Err(approvals::AnswerError::Gone) => return Err(AppError::AgentApprovalGone),
            Err(approvals::AnswerError::Invalid) => {
                return Err(AppError::AgentInvalid(
                    "That isn't one of the answers.".into(),
                ))
            }
        }
    }
    after.run(&app, &agents);
    Ok(())
}

#[tauri::command]
/// Approve a run's waiting plan, or send it back with `feedback` (2.8c): a deny whose message is your words.
pub fn agent_plan(
    app: AppHandle,
    agents: State<'_, Agents>,
    request_id: u32,
    approve: bool,
    feedback: Option<String>,
) -> AppResult<()> {
    let feedback = if approve {
        None
    } else {
        Some(
            interact::feedback(feedback.as_deref().unwrap_or(""))
                .ok_or_else(|| AppError::AgentInvalid("Write what to change first.".into()))?,
        )
    };
    let mut after = After::default();
    {
        let mut inner = agents.lock();
        let p = inner
            .approvals
            .plan(request_id, feedback)
            .ok_or(AppError::AgentApprovalGone)?;
        let by = if approve { "approved" } else { "feedback" };
        inner.settle(vec![p], approve, by, &mut after);
    }
    after.run(&app, &agents);
    Ok(())
}

#[tauri::command]
pub async fn agent_options(agents: State<'_, Agents>) -> AppResult<AgentOptions> {
    let found = installs(&agents);
    let inner = agents.lock();
    let mut own: Vec<(i64, String)> = inner
        .slots
        .iter()
        .map(|s| (s.run.ipc(vec![]).started_at_ms, s.run.cwd.clone()))
        .collect();
    own.sort_by_key(|a| std::cmp::Reverse(a.0));
    let own: Vec<String> = own.into_iter().map(|(_, cwd)| cwd).collect();
    Ok(AgentOptions {
        default_install: installs::pick(&found, inner.settings.claude_path.as_deref()),
        installs: found,
        recent_folders: installs::recent_folders(&own),
        model: inner.settings.model.clone(),
        max_turns: inner.settings.max_turns,
        max_budget_usd: inner.settings.max_budget_usd,
        hotkey: inner.settings.hotkey.clone(),
        hotkey_error: (!inner.hotkey_errors.is_empty()).then(|| inner.hotkey_errors.join(" ")),
        mode: inner.settings.default_mode,
    })
}

#[tauri::command]
/// "Browse…": sync, so it runs on the main thread (the shell dialog needs its STA).
pub fn agent_pick_folder(app: AppHandle) -> AppResult<Option<String>> {
    let owner = window::command::hwnd(&app).ok_or(AppError::NoCommandWindow)?;
    Ok(folder::pick(owner)?)
}

#[tauri::command]
pub async fn command_window_show(app: AppHandle, run_id: Option<String>) -> AppResult<()> {
    window::command::show(&app, run_id)
}

#[tauri::command]
/// Opens the command window on a terminal session's held request (2.8b): its whole diff or command.
pub async fn command_window_show_ask(app: AppHandle, request_id: u32) -> AppResult<()> {
    window::command::show_ask(&app, request_id)
}

#[tauri::command]
pub fn command_take_focus() -> Option<crate::ipc::CommandFocus> {
    window::command::take_focus()
}

#[tauri::command]
pub async fn command_window_hide(app: AppHandle) -> AppResult<()> {
    window::command::hide(&app)
}

/// Debug builds only: drive the agent from outside for verification, through a second launch that the
/// single-instance plugin forwards here. `cay.exe agent-test start <cwd> <prompt> [model] [maxTurns]`,
/// `… decide <requestId> allow|deny [run]`, `… send <runId> <text>`, `… stop <runId>`, `… stop-all`,
/// `… show [runId]`, `… act <tool> [argsJson]` (an action without Claude or approvals), `… look <tool> [argsJson] [imageFile]` (a desktop look without Claude or approvals, glow
/// included). Results go to the log.
#[cfg(debug_assertions)]
pub fn debug_command(app: &AppHandle, args: &[String]) {
    let app = app.clone();
    let args = args.to_vec();
    tauri::async_runtime::spawn(async move {
        let arg = |i: usize| args.get(i).cloned().unwrap_or_default();
        let agents = app.state::<Agents>();
        let result: AppResult<String> = match arg(0).as_str() {
            "start" => agent_start(
                app.clone(),
                AgentStartRequest {
                    cwd: arg(1),
                    prompt: arg(2),
                    model: args.get(3).cloned().filter(|m| !m.is_empty()),
                    max_turns: args.get(4).and_then(|n| n.parse().ok()),
                    max_budget_usd: None,
                    claude_path: None,
                    // Tests from outside start in Ask unless a mode is given, so approvals still show.
                    mode: Some(
                        args.get(5)
                            .and_then(|m| mode::parse(m))
                            .unwrap_or(AgentMode::Default),
                    ),
                    backend: match args.get(6).map(String::as_str) {
                        Some("opencode") => Some(AgentBackend::Opencode),
                        Some("codex") => Some(AgentBackend::Codex),
                        _ => None,
                    },
                    // Paths after the agent are attachments (2.10b).
                    attachments: args.iter().skip(7).cloned().collect(),
                },
            )
            .await
            .map(|s| s.run_id),
            "mode" => agent_set_mode(
                app.clone(),
                agents,
                arg(1),
                mode::parse(&arg(2)).unwrap_or(AgentMode::Default),
            )
            .map(|()| "mode set".into()),
            "decide" => agent_decide(
                app.clone(),
                agents,
                arg(1).parse().unwrap_or(0),
                if arg(2) == "allow" {
                    Decision::Allow
                } else {
                    Decision::Deny
                },
                if arg(3) == "run" {
                    Remember::Run
                } else {
                    Remember::Once
                },
            )
            .map(|()| "decided".into()),
            // A terminal session's permission request (2.8a); its id is in the `permission request <id>` log line.
            "session-decide" => crate::sessions::decide_for_test(
                &app,
                arg(1).parse().unwrap_or(0),
                arg(2) == "allow",
            )
            .map(|()| "decided".into()),
            "send" => agent_send(
                app.clone(),
                agents,
                arg(1),
                arg(2),
                Some(args.iter().skip(3).cloned().collect()),
            )
            .await
            .map(|()| "sent".into()),
            "stop" => agent_stop(app.clone(), agents, arg(1)).map(|()| "stopped".into()),
            "stop-all" => {
                stop_all(&app);
                Ok("stopped all".into())
            }
            "show" => window::command::show(&app, args.get(1).cloned()).map(|()| "shown".into()),
            "hide" => window::command::hide(&app).map(|()| "hidden".into()),
            // `look <tool> [argsJson] [imageFile]`: a desktop look without Claude or approvals (glow included).
            "look" => {
                let name = arg(1);
                let args: Value = serde_json::from_str(&arg(2)).unwrap_or(json!({}));
                let name = tools::NAMES.iter().copied().find(|n| *n == name);
                match name.map(|n| tools::parse(n, &args)) {
                    Some(Ok(look)) => {
                        glow::look_started(desktop::monitor_for(&look));
                        let result = desktop::run(look, None);
                        glow::look_ended();
                        if let (Ok(desktop::Outcome::Image { image, .. }), false) =
                            (&result, arg(3).is_empty())
                        {
                            let _ = std::fs::write(arg(3), &image.bytes);
                        }
                        let audit = tools::audit_fields(name.unwrap_or_default(), &result);
                        let reply = tools::reply(json!(0), &result);
                        let text: Vec<_> = reply["result"]["content"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|c| c["text"].as_str())
                            .collect();
                        Ok(format!(
                            "{audit}\n{}",
                            describe::truncate(&text.join("\n"), 3000)
                        ))
                    }
                    Some(Err(e)) => Err(AppError::AgentInvalid(e)),
                    None => Err(AppError::AgentInvalid("unknown tool".into())),
                }
            }
            // `act <tool> [argsJson]`: an action without Claude or approvals (no refs: points, text, keys, launch).
            // The waits, the guard and the glow work as in a run.
            "act" => {
                let name = actions::NAMES.iter().copied().find(|n| *n == arg(1));
                let args: Value = serde_json::from_str(&arg(2)).unwrap_or(json!({}));
                match name.map(|n| actions::parse(n, &args)) {
                    Some(Ok(call)) => {
                        match actions::resolve(&call, "", &Refs::default(), Instant::now()) {
                            Ok((action, _)) => {
                                let focus_app = app.clone();
                                glow::act_started(desktop::monitor_of(call.hwnd));
                                crate::hover::set_hands_off(true);
                                let result = tauri::async_runtime::spawn_blocking(move || {
                                    desktop::act(
                                        call.hwnd,
                                        None,
                                        action,
                                        move |h| focus_on_main(&focus_app, h),
                                        |phase| log::info!("agent-test act: {phase:?}"),
                                    )
                                })
                                .await
                                .unwrap_or_else(|e| Err(desktop::ActError::Failed(e.to_string())));
                                crate::hover::set_hands_off(false);
                                glow::act_ended();
                                let reply = actions::reply(json!(0), &result);
                                Ok(reply["result"]["content"][0]["text"]
                                    .as_str()
                                    .unwrap_or_default()
                                    .to_string())
                            }
                            Err(e) => Err(AppError::AgentInvalid(e.message())),
                        }
                    }
                    Some(Err(e)) => Err(AppError::AgentInvalid(e)),
                    None => Err(AppError::AgentInvalid("unknown tool".into())),
                }
            }
            // `routine list | save <draftJson> | run <id> | due <id> | delete <id> | pause on|off`. `due` fires it
            // as its schedule would right now (same checks: paused, still running, the limit, the folder).
            "routine" => match arg(1).as_str() {
                "list" => Ok(serde_json::to_string(&agents.routines_payload()).unwrap_or_default()),
                "save" => serde_json::from_str::<RoutineDraft>(&arg(2))
                    .map_err(|e| AppError::AgentInvalid(e.to_string()))
                    .and_then(|draft| routine_save(app.clone(), agents, draft))
                    .map(|r| format!("saved {} {}", r.id, r.name)),
                "run" => routine_run(app.clone(), arg(2))
                    .await
                    .map(|s| format!("routine run {}", s.run_id)),
                "due" => debug_due(&app, &arg(2)),
                "delete" => routine_delete(app.clone(), agents, arg(2)).map(|()| "deleted".into()),
                "pause" => {
                    set_routines_paused(&app, arg(2) != "off");
                    Ok(format!("paused: {}", routines_paused(&app)))
                }
                other => Err(AppError::AgentInvalid(format!(
                    "unknown routine command {other:?}"
                ))),
            },
            // A clean quit (tray "Quit Cay" does the same), to check runs stop with it.
            "quit" => {
                app.exit(0);
                Ok("quitting".into())
            }
            other => Err(AppError::AgentInvalid(format!(
                "unknown agent-test command {other:?}"
            ))),
        };
        match result {
            Ok(text) => log::info!("agent-test: {text}"),
            Err(e) => log::warn!("agent-test failed: {e}"),
        }
    });
}

/// Debug builds only: routine `id` is due now, as if its schedule said so.
#[cfg(debug_assertions)]
fn debug_due(app: &AppHandle, id: &str) -> AppResult<String> {
    let agents = app.state::<Agents>();
    let now = now_ms();
    let why = {
        let mut inner = agents.lock();
        let r = inner
            .routines
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::AgentInvalid("no such routine".into()))?;
        let scheduled = inner
            .slots
            .iter()
            .filter(|s| s.is_going())
            .filter(|s| {
                s.run
                    .routine
                    .as_ref()
                    .is_some_and(|t| t.trigger == RoutineTrigger::Schedule)
            })
            .count();
        let why = if inner.routines.file.paused {
            Some(SkipReason::Paused)
        } else if inner.running_run_of(id).is_some() {
            Some(SkipReason::StillRunning)
        } else if scheduled >= schedule::MAX_SCHEDULED {
            Some(SkipReason::TooMany)
        } else if !Path::new(&r.cwd).is_dir() {
            Some(SkipReason::FolderMissing)
        } else {
            None
        };
        if let Some(why) = why {
            inner.record_skip(id, now, why, now);
            inner.routines.save();
        }
        why
    };
    if let Some(why) = why {
        After {
            routines_changed: true,
            ..After::default()
        }
        .run(app, &agents);
        return Ok(format!("skipped: {why:?}"));
    }
    let run_id = run_routine(app, id, RoutineTrigger::Schedule)?;
    agents.lock().audit_routine(
        "started",
        json!({ "routine": id, "dueMs": now, "late": false, "runId": run_id,
                "trigger": RoutineTrigger::Schedule, "debug": true }),
    );
    Ok(format!("scheduled run {run_id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connector_mail_counts_as_reading_mail() {
        assert!(is_connector_mail("mcp__claude_ai_Gmail__get_thread"));
        assert!(is_connector_mail("mcp__claude_ai_gmail__search_threads"));
        assert!(!is_connector_mail(
            "mcp__claude_ai_Google_Calendar__list_events"
        ));
        assert!(
            !is_connector_mail("mcp__cay__gmail_read"),
            "Cay's own tools mark it themselves"
        );
        let r = redact("mcp__cay__gmail_draft", &json!({ "body": "hello" }));
        assert_eq!(r["body"]["chars"], 5);
        let r = redact("mcp__cay__type_text", &json!({ "hwnd": 1, "text": "pw" }));
        assert_eq!(r["text"]["chars"], 2, "stream tool lines are redacted too");
    }
}
