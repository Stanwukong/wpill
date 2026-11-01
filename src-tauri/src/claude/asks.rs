//! Terminal sessions' permission requests, answered in the island (Phase 2.8a, D34).
//!
//! `cay.exe hook` sends each `PermissionRequest` (`ask.rs`) to `\\.\pipe\cay-claude-ask-<user SID>` and waits. The
//! `claude-asks` thread accepts each on its own `duplex` instance and hands it to a thread that shows a
//! `claudeApproval` card and waits for the first of: your answer (`decide`), the hand-back after `HOLD`, the forwarder
//! going away (answered in the terminal, or Claude Code stopped waiting), the session moving on, or shutdown. Only
//! your click allows or denies; everything else replies `Pass`, so Claude Code's own prompt decides. Requests from
//! Cay's own agent runs pass at once (`agent/` answers those through its prompt tool).

use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tauri::{AppHandle, Manager};
use windows::core::{Error, HSTRING};
use windows::Win32::System::Threading::SetEvent;

use super::ask::{self, Decision, Reply};
use super::{process, sessions, Claude};
use crate::activity::{claude_approval_key, Signal, Watcher};
use crate::duplex::{Pipe, Read};
use crate::error::{AppError, AppResult};
use crate::ipc::{Activity, ClaudeApprovalActivity, ClaudeNeedsInputActivity};
use crate::win32::{self, Handle};

/// How long a request waits in the island before the terminal asks instead. Below the forwarder's own wait
/// (`forward::ANSWER_TIMEOUT_MS`) so the hand-back always reaches it. Never ends in an allow or a deny.
pub const HOLD: Duration = Duration::from_secs(55);
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

    fn hold(&self, session: &str) -> Option<(u32, Arc<Handle>)> {
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
                session: session.to_string(),
                decision: None,
                wake: wake.clone(),
            },
        );
        Some((id, wake))
    }

    fn release(&self, id: u32) -> Option<Decision> {
        self.lock().held.remove(&id).and_then(|h| h.decision)
    }

    /// Your answer from the card. `false` if the request is no longer held or already answered.
    pub fn decide(&self, id: u32, allow: bool) -> bool {
        let decision = if allow {
            Decision::Allow
        } else {
            Decision::Deny
        };
        self.lock()
            .held
            .get_mut(&id)
            .is_some_and(|h| h.settle(decision))
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
    let Some((id, wake)) = asks.hold(&request.session_id) else {
        reply(pipe, Decision::Pass);
        return;
    };
    let project = sessions::project(&request.cwd);
    let session_id = app.state::<Claude>().row_of(&request.session_id);
    let (tool, summary, risk) =
        crate::agent::describe_tool(&request.tool_name, &request.tool_input, &request.cwd);
    log::debug!("permission request {id}: {tool} ({project})");
    let _ = out.send(Signal::Offer(Activity::ClaudeApproval(
        ClaudeApprovalActivity {
            request_id: id,
            session_id,
            project: project.clone(),
            tool,
            summary,
            risk,
        },
    )));

    let waited = pipe.read(&[wake.0, stop.0], HOLD.as_millis() as u32);
    // An answer that landed as the wait ended still counts.
    let decision = asks.release(id);
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

fn reply(pipe: &Pipe, decision: Decision) {
    let Ok(bytes) = serde_json::to_vec(&Reply::new(decision)) else {
        return;
    };
    if let Err(e) = pipe.write(&bytes) {
        log::debug!("permission reply not delivered: {e}");
    }
}

/// `claude_decide` and `agent-test session-decide`.
pub fn decide(app: &AppHandle, request_id: u32, allow: bool) -> AppResult<()> {
    if app.state::<Asks>().decide(request_id, allow) {
        Ok(())
    } else {
        Err(AppError::AgentApprovalGone)
    }
}
