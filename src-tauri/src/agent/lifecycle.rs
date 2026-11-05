//! One run's state: status, step, turns, cost, transcript. Pure; unit-tested. `run.rs` owns the process and
//! feeds this what happens to it.
//!
//! A run is one task, possibly over several processes: stdin stays open while a turn runs (a follow-up sent
//! meanwhile is queued by Claude Code), and is closed once every message sent has its `result`, so the process
//! exits. A later follow-up starts a new process with `--resume`.

use std::collections::VecDeque;

use super::stream::{StreamEvent, TurnResult};
use super::{describe, preview};
use crate::ipc::{
    AgentEndReason, AgentEntry, AgentEvent, AgentMode, AgentRun, AgentRunRoutine, AgentStatus,
};

/// Entries kept per run for the command window (oldest dropped first).
const MAX_ENTRIES: usize = 400;
const MAX_PROMPT: usize = 200;

#[derive(Debug)]
pub struct Run {
    pub id: String,
    pub backend: crate::ipc::AgentBackend,
    pub project: String,
    pub cwd: String,
    prompt: String,
    pub model: Option<String>,
    pub status: AgentStatus,
    pub reason: Option<AgentEndReason>,
    step: Option<String>,
    /// The latest `TodoWrite`'s progress (2.8d).
    todos: Option<crate::ipc::TodoCount>,
    /// The permission mode shown (2.8e): Claude Code's latest report, or a switch Cay just sent.
    mode: AgentMode,
    /// Claude Code's latest report, to fall back to if a switch is refused.
    reported: AgentMode,
    /// Chosen while no process runs; used by the next follow-up.
    next_mode: Option<AgentMode>,
    turns: u32,
    cost_usd: Option<f64>,
    /// Claude Code's session id, from `system/init`; needed to resume.
    pub session: Option<String>,
    error: Option<String>,
    started_at_ms: i64,
    updated_at_ms: i64,
    /// Messages written to the current process that have no `result` yet.
    queued: u32,
    /// When the first of the queued messages was sent: the start of the stretch a `Result` entry times (2.10a).
    busy_since: Option<i64>,
    /// Set by `stop`: the exit that follows is the user's doing.
    stopping: bool,
    transcript: VecDeque<AgentEntry>,
    /// The routine that started it (2.7e).
    pub routine: Option<AgentRunRoutine>,
}

/// What `apply` asks the owner to do.
#[derive(Debug, Default, PartialEq)]
pub struct Applied {
    pub entries: Vec<AgentEntry>,
    /// Every message has its answer: close stdin so the process exits.
    pub close_stdin: bool,
    /// The run just ended (offer `agentFinished`).
    pub finished: bool,
}

impl Run {
    pub fn new(id: String, cwd: String, model: Option<String>, at_ms: i64) -> Self {
        Self {
            id,
            backend: crate::ipc::AgentBackend::Claude,
            project: crate::sessions::project_name(&cwd),
            cwd,
            prompt: String::new(),
            model,
            status: AgentStatus::Starting,
            reason: None,
            step: None,
            todos: None,
            mode: AgentMode::Auto,
            reported: AgentMode::Auto,
            next_mode: None,
            turns: 0,
            cost_usd: None,
            session: None,
            error: None,
            started_at_ms: at_ms,
            updated_at_ms: at_ms,
            queued: 0,
            busy_since: None,
            stopping: false,
            transcript: VecDeque::new(),
            routine: None,
        }
    }

    /// The mode the run starts in.
    pub fn start_in(&mut self, mode: AgentMode) {
        self.mode = mode;
        self.reported = mode;
    }

    pub fn mode(&self) -> AgentMode {
        self.mode
    }

    /// Cay sent a switch to the running process; shown at once, confirmed by Claude Code's `system/status`.
    pub fn switching(&mut self, to: AgentMode, at_ms: i64) {
        self.mode = to;
        self.next_mode = None;
        self.updated_at_ms = at_ms;
    }

    /// Chosen between messages: the next follow-up starts in it.
    pub fn choose_next(&mut self, to: AgentMode, at_ms: i64) {
        self.next_mode = (to != self.mode).then_some(to);
        self.updated_at_ms = at_ms;
    }

    /// The mode a new process (a follow-up) starts in; a mode chosen meanwhile becomes the run's.
    pub fn take_mode(&mut self) -> AgentMode {
        if let Some(next) = self.next_mode.take() {
            self.mode = next;
            self.reported = next;
        }
        self.mode
    }

    pub fn is_live(&self) -> bool {
        matches!(
            self.status,
            AgentStatus::Starting | AgentStatus::Working | AgentStatus::Waiting
        )
    }

    /// A prompt or follow-up is being written to the process.
    pub fn send(&mut self, text: &str, at_ms: i64) -> AgentEntry {
        if self.prompt.is_empty() {
            self.prompt = describe::truncate(text.trim(), MAX_PROMPT);
        }
        if !self.is_live() {
            // A follow-up after the end resumes in a new process.
            self.status = AgentStatus::Starting;
            self.reason = None;
            self.error = None;
            self.stopping = false;
        }
        if self.queued == 0 || self.busy_since.is_none() {
            self.busy_since = Some(at_ms);
        }
        self.queued += 1;
        self.updated_at_ms = at_ms;
        self.push(
            at_ms,
            AgentEvent::User {
                text: text.to_string(),
            },
        )
    }

    pub fn apply(&mut self, event: StreamEvent, at_ms: i64) -> Applied {
        let mut applied = Applied::default();
        if !self.is_live() {
            return applied;
        }
        self.updated_at_ms = at_ms;
        match event {
            StreamEvent::Init { session_id, model } => {
                self.session = Some(session_id);
                if model.is_some() {
                    self.model = model;
                }
                if self.status == AgentStatus::Starting {
                    self.status = AgentStatus::Working;
                }
            }
            StreamEvent::Mode(mode) => {
                self.mode = mode;
                self.reported = mode;
            }
            StreamEvent::ControlError(why) => {
                self.mode = self.reported;
                applied.entries.push(self.push(
                    at_ms,
                    AgentEvent::Error {
                        message: format!("The mode didn't change: {why}"),
                    },
                ));
            }
            StreamEvent::Text(text) => {
                applied
                    .entries
                    .push(self.push(at_ms, AgentEvent::Text { text }));
            }
            StreamEvent::Tool { id, name, input } => {
                let label = describe::step_label(&name, &input, &self.cwd);
                self.step = Some(describe::step_line(&name, &input, &self.cwd));
                if let Some(todos) = describe::todo_count(&name, &input) {
                    self.todos = Some(todos);
                }
                let preview = preview::for_transcript(&name, &input, &self.cwd);
                let name = describe::display_tool(&name);
                applied.entries.push(self.push(
                    at_ms,
                    AgentEvent::Tool {
                        id,
                        name,
                        label,
                        preview,
                    },
                ));
            }
            StreamEvent::ToolResult { id, is_error, text } => {
                applied
                    .entries
                    .push(self.push(at_ms, AgentEvent::ToolResult { id, is_error, text }));
            }
            StreamEvent::Result(result) => {
                self.turns = self.turns.saturating_add(result.num_turns);
                self.cost_usd = result.total_cost_usd.or(self.cost_usd);
                self.queued = self.queued.saturating_sub(1);
                self.step = None;
                self.todos = None;
                let (status, reason, error) = outcome(&result);
                if let Some(error) = error {
                    applied.entries.push(self.push(
                        at_ms,
                        AgentEvent::Error {
                            message: error.clone(),
                        },
                    ));
                    self.error = Some(error);
                }
                if self.queued == 0 {
                    applied.close_stdin = true;
                    applied.finished = true;
                    self.status = status;
                    self.reason = Some(reason);
                    let turns = self.turns;
                    let cost_usd = self.cost_usd;
                    let duration_ms = self.took(at_ms);
                    applied.entries.push(self.push(
                        at_ms,
                        AgentEvent::Result {
                            status,
                            reason,
                            turns,
                            cost_usd,
                            duration_ms,
                        },
                    ));
                } else if self.status == AgentStatus::Starting {
                    self.status = AgentStatus::Working;
                }
            }
        }
        applied
    }

    /// Approvals started or stopped waiting.
    pub fn set_waiting(&mut self, waiting: bool, at_ms: i64) {
        let next = match (self.status, waiting) {
            (AgentStatus::Working | AgentStatus::Starting, true) => AgentStatus::Waiting,
            (AgentStatus::Waiting, false) => AgentStatus::Working,
            _ => return,
        };
        self.status = next;
        self.updated_at_ms = at_ms;
    }

    pub fn stop(&mut self) {
        self.stopping = true;
    }

    /// The process exited. Returns the entries to show and whether the run ended just now. A run that already
    /// ended with its `result` keeps that outcome.
    pub fn exited(&mut self, code: Option<i32>, stderr: &str, at_ms: i64) -> Applied {
        let mut applied = Applied::default();
        if !self.is_live() {
            return applied;
        }
        self.updated_at_ms = at_ms;
        self.step = None;
        self.todos = None;
        self.queued = 0;
        let (status, reason) = if self.stopping {
            (AgentStatus::Stopped, AgentEndReason::Stopped)
        } else {
            let tail = stderr.trim();
            let message = if tail.is_empty() {
                match code {
                    Some(code) => format!("Claude Code exited unexpectedly (code {code})."),
                    None => "Claude Code exited unexpectedly.".to_string(),
                }
            } else {
                tail.to_string()
            };
            applied.entries.push(self.push(
                at_ms,
                AgentEvent::Error {
                    message: message.clone(),
                },
            ));
            self.error = Some(message);
            (AgentStatus::Failed, AgentEndReason::Error)
        };
        self.status = status;
        self.reason = Some(reason);
        let (turns, cost_usd) = (self.turns, self.cost_usd);
        let duration_ms = self.took(at_ms);
        applied.entries.push(self.push(
            at_ms,
            AgentEvent::Result {
                status,
                reason,
                turns,
                cost_usd,
                duration_ms,
            },
        ));
        applied.finished = true;
        applied
    }

    /// How long the stretch of work that just ended took, from its first message; ends the stretch.
    fn took(&mut self, at_ms: i64) -> Option<i64> {
        self.busy_since.take().map(|since| (at_ms - since).max(0))
    }

    /// A line for the transcript that isn't from the stream (approvals, decisions, start failures).
    pub fn note(&mut self, event: AgentEvent, at_ms: i64) -> AgentEntry {
        self.push(at_ms, event)
    }

    pub fn transcript(&self) -> Vec<AgentEntry> {
        self.transcript.iter().cloned().collect()
    }

    pub fn ipc(&self, pending: Vec<crate::ipc::AgentApprovalActivity>) -> AgentRun {
        AgentRun {
            run_id: self.id.clone(),
            backend: self.backend,
            project: self.project.clone(),
            cwd: self.cwd.clone(),
            prompt: self.prompt.clone(),
            status: self.status,
            reason: self.reason,
            step: self.is_live().then(|| self.step.clone()).flatten(),
            todos: self.is_live().then_some(self.todos).flatten(),
            mode: self.mode,
            next_mode: self.next_mode,
            turns: self.turns,
            cost_usd: self.cost_usd,
            model: self.model.clone(),
            pending,
            error: self.error.clone(),
            started_at_ms: self.started_at_ms,
            updated_at_ms: self.updated_at_ms,
            routine: self.routine.clone(),
        }
    }

    fn push(&mut self, at_ms: i64, event: AgentEvent) -> AgentEntry {
        let entry = AgentEntry { at_ms, event };
        if self.transcript.len() >= MAX_ENTRIES {
            self.transcript.pop_front();
        }
        self.transcript.push_back(entry.clone());
        entry
    }
}

/// A turn's `result` → how the run ends if nothing else is queued, plus an error to show.
fn outcome(r: &TurnResult) -> (AgentStatus, AgentEndReason, Option<String>) {
    let detail = || {
        let text = if r.errors.is_empty() {
            r.result.clone().unwrap_or_default()
        } else {
            r.errors.join("\n")
        };
        (!text.trim().is_empty()).then(|| text.trim().to_string())
    };
    match r.subtype.as_str() {
        "success" if !r.is_error => (AgentStatus::Done, AgentEndReason::Completed, None),
        "error_max_turns" => (AgentStatus::Failed, AgentEndReason::MaxTurns, detail()),
        "error_max_budget_usd" => (AgentStatus::Failed, AgentEndReason::MaxBudget, detail()),
        _ => (
            AgentStatus::Failed,
            AgentEndReason::Error,
            detail().or_else(|| Some("The agent reported an error.".into())),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run() -> Run {
        Run::new("r1".into(), r"C:\work\wpill".into(), None, 0)
    }

    fn init() -> StreamEvent {
        StreamEvent::Init {
            session_id: "sid".into(),
            model: Some("haiku".into()),
        }
    }

    fn result(subtype: &str, turns: u32, cost: f64) -> StreamEvent {
        StreamEvent::Result(TurnResult {
            subtype: subtype.into(),
            is_error: subtype != "success",
            num_turns: turns,
            total_cost_usd: Some(cost),
            result: Some("ok".into()),
            errors: if subtype == "success" {
                vec![]
            } else {
                vec!["Reached maximum number of turns (2)".into()]
            },
        })
    }

    #[test]
    fn modes_follow_claude_codes_reports() {
        let mut r = run();
        r.start_in(AgentMode::Auto);
        r.send("go", 1);
        r.apply(init(), 2);
        r.apply(StreamEvent::Mode(AgentMode::Auto), 2);
        r.switching(AgentMode::AcceptEdits, 3);
        assert_eq!(r.ipc(vec![]).mode, AgentMode::AcceptEdits, "shown at once");
        let applied = r.apply(StreamEvent::ControlError("no".into()), 4);
        assert_eq!(
            r.ipc(vec![]).mode,
            AgentMode::Auto,
            "back to what Claude Code reported"
        );
        assert_eq!(applied.entries.len(), 1);
        r.apply(StreamEvent::Mode(AgentMode::Plan), 5);
        assert_eq!(r.mode(), AgentMode::Plan);
        r.apply(result("success", 1, 0.01), 6);
        // Between messages: kept for the next one.
        r.choose_next(AgentMode::Default, 7);
        assert_eq!(r.ipc(vec![]).next_mode, Some(AgentMode::Default));
        assert_eq!(r.ipc(vec![]).mode, AgentMode::Plan);
        assert_eq!(r.take_mode(), AgentMode::Default);
        assert_eq!(r.ipc(vec![]).next_mode, None);
        r.choose_next(AgentMode::Default, 8);
        assert_eq!(r.ipc(vec![]).next_mode, None, "already that mode");
    }

    #[test]
    fn a_task_runs_to_done() {
        let mut r = run();
        assert_eq!(r.project, "wpill");
        r.send("list files", 1);
        assert_eq!(r.status, AgentStatus::Starting);
        r.apply(init(), 2);
        assert_eq!(r.status, AgentStatus::Working);
        assert_eq!(r.session.as_deref(), Some("sid"));
        r.apply(
            StreamEvent::Tool {
                id: "t".into(),
                name: "Bash".into(),
                input: json!({ "command": "dir" }),
            },
            3,
        );
        assert_eq!(r.ipc(vec![]).step.as_deref(), Some("Bash · dir"));
        let a = r.apply(result("success", 2, 0.01), 4);
        assert!(a.close_stdin && a.finished);
        assert_eq!(r.status, AgentStatus::Done);
        assert_eq!(r.reason, Some(AgentEndReason::Completed));
        assert_eq!(r.ipc(vec![]).step, None);
        // The exit after closing stdin doesn't change the outcome.
        assert_eq!(r.exited(Some(0), "", 5), Applied::default());
        assert_eq!(r.status, AgentStatus::Done);
    }

    #[test]
    fn a_follow_up_mid_turn_keeps_stdin_open() {
        let mut r = run();
        r.send("one", 1);
        r.apply(init(), 2);
        r.send("two", 3);
        let a = r.apply(result("success", 1, 0.01), 4);
        assert!(!a.close_stdin && !a.finished);
        assert_eq!(r.status, AgentStatus::Working);
        let a = r.apply(result("success", 1, 0.02), 5);
        assert!(a.close_stdin);
        assert_eq!(r.ipc(vec![]).turns, 2);
        assert_eq!(r.ipc(vec![]).cost_usd, Some(0.02));
    }

    #[test]
    fn results_are_timed_from_the_first_message_and_edits_carry_their_diff() {
        let mut r = run();
        r.send("one", 1_000);
        r.apply(init(), 1_100);
        let a = r.apply(
            StreamEvent::Tool {
                id: "e".into(),
                name: "Edit".into(),
                input: json!({ "file_path": "a.txt", "old_string": "a", "new_string": "b" }),
            },
            1_200,
        );
        let AgentEvent::Tool { preview, .. } = &a.entries[0].event else {
            panic!("a tool entry");
        };
        let Some(crate::ipc::ApprovalPreview::Diff(d)) = preview else {
            panic!("an edit has its diff");
        };
        assert_eq!(
            (d.file.as_str(), d.added, d.removed),
            ("a.txt", Some(1), Some(1))
        );
        let a = r.apply(
            StreamEvent::Tool {
                id: "b".into(),
                name: "Bash".into(),
                input: json!({ "command": "dir" }),
            },
            1_300,
        );
        assert!(matches!(
            &a.entries[0].event,
            AgentEvent::Tool { preview: None, .. }
        ));
        r.send("two", 2_000);
        r.apply(result("success", 1, 0.01), 3_000);
        let a = r.apply(result("success", 1, 0.02), 4_500);
        let duration = |a: &Applied| match a.entries.last().map(|e| &e.event) {
            Some(AgentEvent::Result { duration_ms, .. }) => *duration_ms,
            other => panic!("a result, got {other:?}"),
        };
        assert_eq!(duration(&a), Some(3_500), "from the first queued message");
        // A follow-up after the end is timed on its own, and so is an exit.
        r.send("three", 10_000);
        let a = r.exited(Some(1), "boom", 10_250);
        assert_eq!(duration(&a), Some(250));
    }

    #[test]
    fn limits_and_errors() {
        let mut r = run();
        r.send("x", 1);
        r.apply(result("error_max_turns", 3, 0.1), 2);
        assert_eq!(r.status, AgentStatus::Failed);
        assert_eq!(r.reason, Some(AgentEndReason::MaxTurns));
        assert_eq!(
            r.ipc(vec![]).error.as_deref(),
            Some("Reached maximum number of turns (2)")
        );

        let mut r = run();
        r.send("x", 1);
        let a = r.exited(Some(1), "Not logged in", 2);
        assert!(a.finished);
        assert_eq!(r.status, AgentStatus::Failed);
        assert_eq!(r.ipc(vec![]).error.as_deref(), Some("Not logged in"));
    }

    #[test]
    fn stop_waiting_and_resume() {
        let mut r = run();
        r.send("x", 1);
        r.apply(init(), 2);
        r.set_waiting(true, 3);
        assert_eq!(r.status, AgentStatus::Waiting);
        r.set_waiting(false, 4);
        assert_eq!(r.status, AgentStatus::Working);
        r.stop();
        r.exited(None, "", 5);
        assert_eq!(r.status, AgentStatus::Stopped);
        assert_eq!(r.reason, Some(AgentEndReason::Stopped));
        // Events from the dead process are ignored.
        assert_eq!(r.apply(init(), 6), Applied::default());
        // A follow-up starts it again with the same session.
        r.send("go on", 7);
        assert_eq!(r.status, AgentStatus::Starting);
        assert_eq!(r.reason, None);
        assert_eq!(r.session.as_deref(), Some("sid"));
        assert_eq!(r.ipc(vec![]).prompt, "x");
    }
}
