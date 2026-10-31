//! Local automation agent (Phase 2.7, CASESTUDY D21–D23): headless Claude Code runs started from the command
//! window, shown in the island, with their permission prompts answered from it.
//!
//! - `run.rs` starts `claude.exe -p` (stream-json both ways) in a Job Object; its stdout feeds `lifecycle.rs`.
//! - Claude Code reaches TopBar's MCP server through `topbar.exe mcp` (`bridge.rs`) and the agent pipe
//!   (`pipe.rs`). The `agent-server` thread accepts bridges; each connection proves its run with a token, and its
//!   `approve` calls (`mcp.rs`) become `agentApproval` activities (`approvals.rs`, `describe.rs`).
//! - The desktop tools (2.7c, `tools.rs` → `crate::desktop`) look at windows and the screen. Each look is
//!   approved (Claude Code's prompt, then TopBar's own grant check), runs on its own thread, and lights the
//!   screen edges (`window::glow`) while it runs.
//! - The desktop actions (2.7d, `actions.rs` → `crate::desktop::act`) focus, invoke, set fields, click, type,
//!   press keys and launch. Each is approved on its own (never "for this run"), waits until you let go of the
//!   mouse and keyboard, shows `agentActing` and the acting glow, and stops at once if you touch either.
//! - Hotkeys (`hotkey.rs`) open the command window or stop everything. Everything is logged (`audit.rs`).
//!
//! Rust owns runs, approvals and timeouts; React only renders `agent:runs`, `agent:output` and the activities.

mod actions;
mod approvals;
mod audit;
mod bridge;
mod describe;
mod folder;
mod hotkey;
mod installs;
mod job;
mod lifecycle;
mod mcp;
mod pipe;
mod run;
mod settings;
mod stream;
mod tools;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, State};
use windows::Win32::Foundation::WAIT_TIMEOUT;
use windows::Win32::System::Threading::{SetEvent, WaitForSingleObject, INFINITE};

use crate::activity::{approval_key, Activities, Signal, Watcher, ACTING_KEY};
use crate::desktop::{self, refs::Refs};
use crate::error::{AppError, AppResult};
use crate::ipc::{
    events, Activity, AgentActingActivity, AgentActingPhase, AgentEntry, AgentEvent,
    AgentFinishedActivity, AgentOptions, AgentOutputPayload, AgentRunsPayload, AgentStarted,
    AgentStatus, ClaudeInstall,
};
use crate::win32::Handle;
use crate::window::{self, glow};
use approvals::{Approvals, Asked, Pending, Purpose, ReplyTo, Request};
use audit::{now_ms, Audit};
use lifecycle::Run;
use settings::AgentSettings;

/// A bridge gets this long to say hello after connecting.
const HELLO_TIMEOUT_MS: u32 = 2000;
/// Ended runs kept for the panel and the command window.
const MAX_ENDED: usize = 20;
const MAX_PROMPT: usize = 32 * 1024;

/// `topbar.exe mcp`: `main.rs` calls this before anything else starts.
pub fn mcp_bridge() -> i32 {
    bridge::run()
}

struct Slot {
    run: Run,
    token: String,
    launch: run::Launch,
    process: Option<run::Process>,
    /// Bumped per process, so a late line or exit from an earlier process is ignored.
    generation: u32,
    dir: PathBuf,
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
    runs_changed: bool,
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

    /// Kills every run's process tree. Called on exit; the jobs would also die with TopBar.
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

    /// Answers and withdraws approvals (denied unless `allow`), noting the decision in the transcript.
    fn settle(&mut self, settled: Vec<Pending>, allow: bool, by: &'static str, after: &mut After) {
        for p in settled {
            let message = match by {
                "timeout" => "Nobody answered in TopBar within the time limit, so this was denied.",
                "runEnded" => "The run ended before this was answered.",
                "cancelled" => "The call was cancelled before it was answered.",
                _ => "The user denied this in TopBar. Don't retry it; adapt or explain what you needed.",
            };
            if let Some(conn) = self.conns.get(&p.reply.conn) {
                let rpc_id = p.reply.rpc_id.clone();
                match p.purpose {
                    Purpose::Prompt => {
                        let reply = mcp::decision(rpc_id, allow, p.input.clone(), message);
                        after.replies.push((conn.pipe.clone(), reply));
                    }
                    // TopBar's own check before a look: allowed means look now, denied is the tool's error.
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
        let fields = json!({ "status": status, "reason": reason, "turns": slot.run.ipc(vec![]).turns,
                             "costUsd": slot.run.ipc(vec![]).cost_usd, "error": slot.run.ipc(vec![]).error });
        self.audit(run_id, "result", fields);
        if status != AgentStatus::Stopped {
            after.signals.push(Signal::Offer(Activity::AgentFinished(
                AgentFinishedActivity {
                    run_id: run_id.to_string(),
                    project,
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
    }
}

/// Starts the server, timer and hotkey threads. Logged and skipped if the pipe can't be created.
pub fn spawn(app: &AppHandle, activities: &Activities) {
    let settings = settings::load(app);
    let dir = app
        .path()
        .app_data_dir()
        .map(|d| d.join("agent"))
        .unwrap_or_else(|_| std::env::temp_dir().join("topbar-agent"));
    let audit = Arc::new(Audit::new(dir));
    audit.prune();
    let stop = match Handle::event() {
        Ok(stop) => Arc::new(stop),
        Err(e) => {
            log::warn!("agent unavailable: {e}");
            return;
        }
    };
    {
        let agents = app.state::<Agents>();
        let mut inner = agents.lock();
        inner.settings = settings.clone();
        inner.audit = Some(audit);
        inner.signals = Some(activities.sender());
        inner.stop = Some(stop.clone());
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
    match start_timer(app.clone(), stop) {
        Ok(watcher) => activities.add_watcher(watcher),
        Err(e) => log::warn!("agent timer unavailable: {e}"),
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
    let first = pipe::Pipe::create(true)?;
    let signal = stop.clone();
    Watcher::spawn(
        "agent-server",
        move || {
            log::info!("agent pipe ready");
            let mut next = Some(first);
            loop {
                let instance = match next.take().map_or_else(|| pipe::Pipe::create(false), Ok) {
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
            h.topbar == mcp::HELLO_VERSION
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
        inner.next_conn += 1;
        let id = inner.next_conn;
        inner.conns.insert(
            id,
            Conn {
                pipe: pipe.clone(),
                run_id,
            },
        );
        id
    };
    loop {
        let message = match pipe.read(&[stop.0], INFINITE) {
            Ok(pipe::Read::Message(m)) => m,
            Ok(pipe::Read::TooLong) => continue,
            _ => break,
        };
        let Ok(message) = serde_json::from_slice::<Value>(&message) else {
            continue;
        };
        match mcp::handle(&message) {
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
            json!({ "tool": tool, "input": audit::input_for_log(&actions::redact(&tool, &input)) }),
        );
        let element = actions::element_for(&tool, &input, &run_id, &inner.refs, Instant::now());
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

/// One of the desktop tools was called: it runs if its prompt was approved (a grant) or its kind is allowed for
/// this run; otherwise TopBar asks in the island itself first.
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
        } else {
            log::info!("agent look {tool} arrived without an approved prompt; asking in TopBar");
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
/// prompt left (actions are never remembered for the run); without one, TopBar asks in the island itself.
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
        } else {
            log::info!("agent action {tool} arrived without an approved prompt; asking in TopBar");
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
            let by = if a.purpose == Purpose::Look {
                "topbar"
            } else {
                "claudeCode"
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

fn start_timer(app: AppHandle, stop: Arc<Handle>) -> AppResult<Watcher> {
    let signal = stop.clone();
    Watcher::spawn(
        "agent-timer",
        move || loop {
            // SAFETY: waits on our own event.
            if unsafe { WaitForSingleObject(stop.0, 1000) } != WAIT_TIMEOUT {
                return;
            }
            let agents = app.state::<Agents>();
            let mut after = After::default();
            {
                let mut inner = agents.lock();
                let expired = inner.approvals.expired(Instant::now());
                if !expired.is_empty() {
                    inner.settle(expired, false, "timeout", &mut after);
                }
            }
            after.run(&app, &agents);
        },
        move || {
            // SAFETY: as in `start_server`.
            let _ = unsafe { SetEvent(signal.0) };
        },
    )
}

/// Starts `slot`'s next process (the first, or a `--resume` follow-up) and writes `text` to it.
fn start_process(
    app: &AppHandle,
    inner: &mut Inner,
    run_id: &str,
    text: &str,
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
    let config = slot.dir.join("mcp.json");
    let mcp = json!({ "mcpServers": { mcp::SERVER_NAME: {
        "type": "stdio",
        "command": exe_path.to_string_lossy(),
        "args": ["mcp"],
        "env": { bridge::TOKEN_ENV: slot.token, bridge::RUN_ENV: slot.run.id },
    } } });
    std::fs::write(&config, mcp.to_string()).map_err(|e| AppError::AgentStart(e.to_string()))?;
    slot.launch.resume = slot.run.session.clone().filter(|_| slot.generation > 1);

    let (line_app, exit_app) = (app.clone(), app.clone());
    let (line_id, exit_id) = (run_id.to_string(), run_id.to_string());
    let mut process = run::spawn(
        &slot.launch,
        &config,
        move |line| on_line(&line_app, &line_id, generation, line),
        move |code, stderr| on_exit(&exit_app, &exit_id, generation, code, &stderr),
    )
    .map_err(|e| AppError::AgentStart(format!("Couldn't start Claude Code: {e}")))?;
    let entry = slot.run.send(text, now_ms());
    process
        .write(&stream::user_message(text))
        .map_err(|e| AppError::AgentStart(e.to_string()))?;
    slot.process = Some(process);
    after.outputs.push(AgentOutputPayload {
        run_id: run_id.to_string(),
        entry,
    });
    after.runs_changed = true;
    Ok(())
}

fn on_line(app: &AppHandle, run_id: &str, generation: u32, line: &[u8]) {
    let events = stream::parse(line);
    if events.is_empty() {
        return;
    }
    let agents = app.state::<Agents>();
    let mut after = After::default();
    {
        let mut inner = agents.lock();
        let mut finished = false;
        let mut tools = Vec::new();
        let Some(slot) = inner.slot(run_id).filter(|s| s.generation == generation) else {
            return;
        };
        for event in events {
            if let stream::StreamEvent::Tool { name, input, .. } = &event {
                tools.push(json!({ "tool": name, "input": audit::input_for_log(input) }));
            }
            let at = now_ms();
            let applied = slot.run.apply(event, at);
            if applied.close_stdin {
                if let Some(process) = &mut slot.process {
                    process.stdin = None;
                }
            }
            finished |= applied.finished;
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
        if finished {
            inner.finished(run_id, &mut after);
        }
        after.runs_changed = true;
    }
    after.run(app, &agents);
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
}

#[tauri::command]
pub async fn agent_start(
    app: AppHandle,
    agents: State<'_, Agents>,
    request: AgentStartRequest,
) -> AppResult<AgentStarted> {
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
    let found = installs(&agents);
    let settings = agents.lock().settings.clone();
    let configured = request.claude_path.or(settings.claude_path.clone());
    let exe = installs::pick(&found, configured.as_deref()).ok_or(AppError::AgentNoClaude)?;
    let at = now_ms();
    let run_id = audit::run_id(at);
    let model = request
        .model
        .or(settings.model.clone())
        .filter(|m| !m.is_empty());
    let launch = run::Launch {
        exe: exe.clone(),
        cwd: request.cwd.clone(),
        model: model.clone(),
        max_turns: request.max_turns.unwrap_or(settings.max_turns),
        max_budget_usd: request.max_budget_usd.or(settings.max_budget_usd),
        resume: None,
        strict_mcp_config: settings.strict_mcp_config,
        user_settings: settings.user_settings,
    };
    let mut after = After::default();
    let result = {
        let mut inner = agents.lock();
        let dir = inner
            .audit
            .as_ref()
            .map(|a| a.dir().join("runs").join(&run_id))
            .unwrap_or_else(|| std::env::temp_dir().join(&run_id));
        inner.audit(
            &run_id,
            "start",
            json!({
                "prompt": prompt, "cwd": request.cwd, "exe": exe, "model": model,
                "maxTurns": launch.max_turns, "maxBudgetUsd": launch.max_budget_usd,
            }),
        );
        inner.slots.push(Slot {
            run: Run::new(run_id.clone(), request.cwd, model, at),
            token: audit::random_hex(16),
            launch,
            process: None,
            generation: 0,
            dir,
        });
        let result = start_process(&app, &mut inner, &run_id, &prompt, &mut after);
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
    after.run(&app, &agents);
    result?;
    log::info!("agent run {run_id} started");
    Ok(AgentStarted { run_id })
}

#[tauri::command]
/// A follow-up: queued into the running process, or resumes the finished session in a new one.
pub async fn agent_send(
    app: AppHandle,
    agents: State<'_, Agents>,
    run_id: String,
    text: String,
) -> AppResult<()> {
    let text = text.trim().to_string();
    if text.is_empty() || text.len() > MAX_PROMPT {
        return Err(AppError::AgentInvalid("Type a message first.".into()));
    }
    let mut after = After::default();
    let result = {
        let mut inner = agents.lock();
        inner.audit(&run_id, "followUp", json!({ "text": text }));
        let slot = inner.slot(&run_id).ok_or(AppError::AgentRunGone)?;
        let writable =
            slot.run.is_live() && slot.process.as_ref().is_some_and(|p| p.stdin.is_some());
        if writable {
            let entry = slot.run.send(&text, now_ms());
            let written = slot
                .process
                .as_mut()
                .map(|p| p.write(&stream::user_message(&text)));
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
            start_process(&app, &mut inner, &run_id, &text, &mut after)
        }
    };
    after.run(&app, &agents);
    result
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
pub fn agent_runs(agents: State<'_, Agents>) -> AgentRunsPayload {
    agents.payload()
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
pub async fn command_window_hide(app: AppHandle) -> AppResult<()> {
    window::command::hide(&app)
}

/// Debug builds only: drive the agent from outside for verification, through a second launch that the
/// single-instance plugin forwards here. `topbar.exe agent-test start <cwd> <prompt> [model] [maxTurns]`,
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
                agents,
                AgentStartRequest {
                    cwd: arg(1),
                    prompt: arg(2),
                    model: args.get(3).cloned().filter(|m| !m.is_empty()),
                    max_turns: args.get(4).and_then(|n| n.parse().ok()),
                    max_budget_usd: None,
                    claude_path: None,
                },
            )
            .await
            .map(|s| s.run_id),
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
            "send" => agent_send(app.clone(), agents, arg(1), arg(2))
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
            // A clean quit (tray "Quit TopBar" does the same), to check runs stop with it.
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
