//! Terminal sessions' permission requests, answered in the island (Phase 2.8a, D34).
//!
//! `cay.exe hook` sends each `PermissionRequest` (`ask.rs`) to `\\.\pipe\cay-claude-ask-<user SID>` and waits. The
//! `claude-asks` thread accepts each on its own `duplex` instance and hands it to a thread that shows a
//! `claudeApproval` card and waits for the first of: your answer (`decide`), the hand-back after `HOLD`, the forwarder
//! going away (answered in the terminal, or Claude Code stopped waiting), the session moving on, or shutdown. Only
//! your click allows or denies; everything else replies `Pass`, so Claude Code's own prompt decides. Requests from
//! Cay's own agent runs pass at once (`agent/` answers those through its prompt tool).

use std::collections::{BTreeMap, HashMap};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};
use windows::core::{Error, HSTRING};
use windows::Win32::System::Threading::SetEvent;

use super::ask::{self, Decision, Reply};
use super::{process, sessions, Claude};
use crate::activity::{claude_approval_key, Signal, Watcher};
use crate::agent::interact;
use crate::duplex::{Pipe, Read};
use crate::error::{AppError, AppResult};
use crate::ipc::{
    events, Activity, ApprovalPreview, ClaudeApprovalActivity, ClaudeAsksPayload,
    ClaudeNeedsInputActivity,
};
use crate::win32::{self, Handle};

/// How long a permission request waits in the island before it's handed back. Below the forwarder's own wait
/// (`forward::answer_timeout_ms`) so the hand-back always reaches it. Never ends in an allow or a deny.
pub const HOLD: Duration = Duration::from_secs(55);
/// A question or a plan (2.8c) waits longer: reading takes time, and the terminal asks meanwhile, so holding it
/// doesn't block you there.
pub const HOLD_INTERACTIVE: Duration = Duration::from_secs(600);

/// How long a request for `tool` is held.
pub fn hold_for(tool: &str) -> Duration {
    if crate::agent::interact::is_interactive(tool) {
        HOLD_INTERACTIVE
    } else {
        HOLD
    }
}
/// A connected forwarder gets this long to send its request.
const READ_TIMEOUT_MS: u32 = 1000;
/// More requests than this at once go straight to their terminals.
const MAX_HELD: usize = 16;

/// `\\.\pipe\cay-claude-ask-<user SID>`: per user, like the hook pipe.
pub fn name() -> Option<HSTRING> {
    Some(HSTRING::from(format!(
        r"\\.\pipe\cay-claude-ask-{}",
        win32::user_sid()?
    )))
}

/// Tauri-managed: the requests being held, for `claude_decide` and the listener.
#[derive(Default)]
pub struct Asks(Mutex<Inner>);

#[derive(Default)]
struct Inner {
    next_id: u32,
    held: HashMap<u32, Held>,
}

struct Held {
    /// Claude Code's session id.
    session: String,
    /// The tool and its (trimmed) input, for questions and plans (2.8c).
    tool: String,
    input: serde_json::Value,
    /// A question's answers so far (question text → answer); sent once every question has one.
    answers: BTreeMap<String, String>,
    /// The card, for the command window's list; set once it's shown.
    card: Option<ClaudeApprovalActivity>,
    /// The command window's copy of the preview (2.8b).
    full: Option<ApprovalPreview>,
    /// Set once: your answer, or `Pass` when the session moved on.
    decision: Option<Decision>,
    /// Wakes the request's thread.
    wake: Arc<Handle>,
}

impl Held {
    fn settle(&mut self, decision: Decision) -> bool {
        if self.decision.is_some() {
            return false;
        }
        self.decision = Some(decision);
        // SAFETY: signals the request's own event, which it holds through the Arc.
        let _ = unsafe { SetEvent(self.wake.0) };
        true
    }
}

impl Asks {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn hold(&self, request: &ask::Request) -> Option<(u32, Arc<Handle>)> {
        let mut inner = self.lock();
        if inner.held.len() >= MAX_HELD {
            return None;
        }
        let wake = Arc::new(Handle::event().ok()?);
        inner.next_id = inner.next_id.wrapping_add(1);
        let id = inner.next_id;
        inner.held.insert(
            id,
            Held {
                session: request.session_id.clone(),
                tool: request.tool_name.clone(),
                input: request.tool_input.clone(),
                answers: BTreeMap::new(),
                card: None,
                full: None,
                decision: None,
                wake: wake.clone(),
            },
        );
        Some((id, wake))
    }

    fn show(&self, id: u32, card: ClaudeApprovalActivity, full: Option<ApprovalPreview>) {
        if let Some(held) = self.lock().held.get_mut(&id) {
            held.card = Some(card);
            held.full = full;
        }
    }

    /// The cards being held, oldest first (`claude:asks`).
    pub fn list(&self) -> Vec<ClaudeApprovalActivity> {
        let inner = self.lock();
        let mut ids: Vec<&u32> = inner.held.keys().collect();
        ids.sort_by_key(|id| **id);
        ids.into_iter()
            .filter_map(|id| inner.held[id].card.clone())
            .collect()
    }

    /// The whole diff or command of a held request.
    pub fn preview(&self, id: u32) -> Option<ApprovalPreview> {
        self.lock().held.get(&id)?.full.clone()
    }

    fn release(&self, id: u32) -> Option<Decision> {
        self.lock().held.remove(&id).and_then(|h| h.decision)
    }

    /// Your answer from the card. `false` if the request is no longer held or already answered, or if it's a
    /// question or a plan (they're answered with `answer` / `plan`, never a bare allow; a deny skips them).
    pub fn decide(&self, id: u32, allow: bool) -> bool {
        let decision = if allow {
            Decision::Allow
        } else {
            Decision::Deny
        };
        self.lock()
            .held
            .get_mut(&id)
            .is_some_and(|h| !(allow && interact::is_interactive(&h.tool)) && h.settle(decision))
    }

    /// Your answer to one of a held question's questions (2.8c). Once every question has one, the request is
    /// answered; until then the card comes back with the answers so far.
    pub fn answer(
        &self,
        id: u32,
        question: &str,
        labels: &[String],
        other: Option<&str>,
    ) -> Result<Option<ClaudeApprovalActivity>, AppError> {
        let mut inner = self.lock();
        let held = inner
            .held
            .get_mut(&id)
            .filter(|h| h.decision.is_none() && h.tool == interact::QUESTION_TOOL)
            .ok_or(AppError::AgentApprovalGone)?;
        let questions = interact::questions(&held.input).ok_or_else(|| {
            AppError::AgentInvalid("This question can't be answered here.".into())
        })?;
        let (key, value) = interact::answer(&questions, question, labels, other)
            .ok_or_else(|| AppError::AgentInvalid("That isn't one of the answers.".into()))?;
        held.answers.insert(key, value);
        if interact::complete(&questions, &held.answers) {
            let answers = held.answers.clone();
            held.settle(Decision::Answers(answers));
            return Ok(None);
        }
        let card = held.card.as_mut().map(|card| {
            card.interaction =
                interact::interaction(&held.tool, &held.input, &held.answers).map(Box::new);
            card.clone()
        });
        Ok(card)
    }

    /// Approve a held plan, or send it back with your feedback (2.8c).
    pub fn plan(&self, id: u32, approve: bool, feedback: &str) -> Result<(), AppError> {
        let decision = if approve {
            Decision::ApprovePlan
        } else {
            Decision::Feedback(
                interact::feedback(feedback)
                    .ok_or_else(|| AppError::AgentInvalid("Write what to change first.".into()))?,
            )
        };
        let mut inner = self.lock();
        let held = inner
            .held
            .get_mut(&id)
            .filter(|h| h.tool == interact::PLAN_TOOL)
            .ok_or(AppError::AgentApprovalGone)?;
        if held.settle(decision) {
            Ok(())
        } else {
            Err(AppError::AgentApprovalGone)
        }
    }

    /// The session moved on (a new prompt, it stopped or ended): its requests go back to the terminal.
    pub fn release_session(&self, session: &str) {
        for held in self.lock().held.values_mut() {
            if held.session == session {
                held.settle(Decision::Pass);
            }
        }
    }

    /// Whether the island holds a request for this session (its "needs input" would repeat the card).
    pub fn holds_for(&self, session: &str) -> bool {
        self.lock().held.values().any(|h| h.session == session)
    }
}

pub fn start(app: AppHandle, out: Sender<Signal>) -> AppResult<Watcher> {
    let name = name().ok_or_else(Error::from_thread)?;
    let first = Pipe::create(&name, true)?;
    let stop = Arc::new(Handle::event()?);
    let signal = stop.clone();
    Watcher::spawn(
        "claude-asks",
        move || {
            log::info!("listening for Claude Code permission requests");
            let mut next = Some(first);
            loop {
                let instance = match next.take().map_or_else(|| Pipe::create(&name, false), Ok) {
                    Ok(instance) => instance,
                    Err(e) => {
                        log::warn!("permission request pipe stopped: {e}");
                        return;
                    }
                };
                match instance.accept(stop.0) {
                    Ok(true) => {
                        let (app, out, stop) = (app.clone(), out.clone(), stop.clone());
                        let spawned = std::thread::Builder::new()
                            .name("claude-ask".into())
                            .spawn(move || serve(&app, &out, &instance, &stop));
                        if let Err(e) = spawned {
                            log::warn!("permission request thread: {e}");
                        }
                    }
                    Ok(false) => return,
                    Err(e) => log::debug!("permission request accept failed: {e}"),
                }
            }
        },
        move || {
            // SAFETY: signals the event both sides hold through the Arc.
            let _ = unsafe { SetEvent(signal.0) };
        },
    )
}

/// One request, from its arrival until it's answered or handed back.
fn serve(app: &AppHandle, out: &Sender<Signal>, pipe: &Pipe, stop: &Handle) {
    let request = match pipe.read(&[stop.0], READ_TIMEOUT_MS) {
        Ok(Read::Message(m)) => ask::decode(&m),
        _ => None,
    };
    // Dropping the pipe without a reply leaves the prompt to Claude Code.
    let Some(request) = request else {
        log::debug!("ignored a malformed permission request");
        return;
    };
    if request.chain.iter().any(process::is_self) {
        reply(pipe, Decision::Pass);
        return;
    }
    let asks = app.state::<Asks>();
    let Some((id, wake)) = asks.hold(&request) else {
        reply(pipe, Decision::Pass);
        return;
    };
    let project = sessions::project(&request.cwd);
    let session_id = app.state::<Claude>().row_of(&request.session_id);
    let (tool, summary, risk) =
        crate::agent::describe_tool(&request.tool_name, &request.tool_input, &request.cwd);
    let previews = crate::agent::preview_tool(
        &request.tool_name,
        &request.tool_input,
        &request.cwd,
        &request.cut,
    );
    log::debug!("permission request {id}: {tool} ({project})");
    let card = ClaudeApprovalActivity {
        request_id: id,
        session_id,
        project: project.clone(),
        tool,
        summary,
        risk,
        preview: previews.as_ref().and_then(|p| p.0.clone()).map(Box::new),
        interaction: interact::interaction(
            &request.tool_name,
            &request.tool_input,
            &BTreeMap::new(),
        )
        .map(Box::new),
    };
    asks.show(id, card.clone(), previews.map(|p| p.1));
    emit(app, &asks);
    let _ = out.send(Signal::Offer(Activity::ClaudeApproval(card)));

    let waited = pipe.read(
        &[wake.0, stop.0],
        hold_for(&request.tool_name).as_millis() as u32,
    );
    // An answer that landed as the wait ended still counts.
    let decision = asks.release(id);
    emit(app, &asks);
    let _ = out.send(Signal::Retract(claude_approval_key(id)));
    match (decision, waited) {
        (Some(decision), _) => {
            log::debug!("permission request {id}: {decision:?}");
            reply(pipe, decision);
        }
        (None, Ok(Read::Closed)) => {
            log::debug!("permission request {id}: answered in the terminal");
        }
        (None, Ok(Read::Timeout)) => {
            log::debug!("permission request {id}: handed back to the terminal");
            reply(pipe, Decision::Pass);
            // The terminal is asking now; say so like any other prompt.
            if let Some(session_id) = session_id {
                let _ = out.send(Signal::Offer(Activity::ClaudeNeedsInput(
                    ClaudeNeedsInputActivity {
                        session_id,
                        project,
                    },
                )));
            }
        }
        (None, _) => reply(pipe, Decision::Pass),
    }
}

/// `claude:asks`, for the command window.
fn emit(app: &AppHandle, asks: &Asks) {
    let _ = app.emit(events::CLAUDE_ASKS, ClaudeAsksPayload { asks: asks.list() });
}

fn reply(pipe: &Pipe, decision: Decision) {
    let Ok(bytes) = serde_json::to_vec(&Reply::new(decision)) else {
        return;
    };
    if let Err(e) = pipe.write(&bytes) {
        log::debug!("permission reply not delivered: {e}");
    }
}

/// `claude_answer`: a question's card shows the answers so far until every question has one.
pub fn answer(
    app: &AppHandle,
    request_id: u32,
    question: &str,
    labels: &[String],
    other: Option<&str>,
) -> AppResult<()> {
    let asks = app.state::<Asks>();
    if let Some(card) = asks.answer(request_id, question, labels, other)? {
        let _ = app
            .state::<crate::activity::Activities>()
            .sender()
            .send(Signal::Offer(Activity::ClaudeApproval(card)));
        emit(app, &asks);
    }
    Ok(())
}

/// `claude_plan`.
pub fn plan(app: &AppHandle, request_id: u32, approve: bool, feedback: &str) -> AppResult<()> {
    app.state::<Asks>().plan(request_id, approve, feedback)
}

/// `claude_decide` and `agent-test session-decide`.
pub fn decide(app: &AppHandle, request_id: u32, allow: bool) -> AppResult<()> {
    if app.state::<Asks>().decide(request_id, allow) {
        Ok(())
    } else {
        Err(AppError::AgentApprovalGone)
    }
}
