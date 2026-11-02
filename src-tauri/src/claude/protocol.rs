//! The pipe message: what the forwarder keeps from a hook's stdin, and the listener's defensive parsing.
//! Pure; unit-tested.
//!
//! Any local process of this user can write to the pipe, so every field is bounded and checked. The
//! forwarder only copies the fields below: prompt, transcript and assistant text never reach the pipe.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ipc::TodoCount;

pub const VERSION: u32 = 1;
/// Largest message the listener reads; anything longer is dropped whole.
pub const MAX_MESSAGE: usize = 16 * 1024;
const MAX_SESSION_ID: usize = 128;
const MAX_CWD: usize = 1024;
const MAX_CHAIN: usize = 16;
const MAX_EXE: usize = 260;
/// The step line, in characters (`describe::step_line` makes at most 64).
const MAX_STEP: usize = 96;
const MAX_TODOS: u32 = 1000;

/// One ancestor of the forwarder. `created` (FILETIME ticks) tells a live process from a reused pid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proc {
    pub pid: u32,
    /// File name only, e.g. `WindowsTerminal.exe`.
    pub exe: String,
    pub created: u64,
}

/// On the wire, one per connection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Envelope {
    pub v: u32,
    /// When the forwarder started (Unix ms). Async hooks can arrive out of order.
    pub at_ms: i64,
    pub event: String,
    pub session_id: String,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notification_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// `PreToolUse` (2.8d): the tool and its target, built by the forwarder ("Edit · hover/logic.rs"). The tool's
    /// input itself never reaches the pipe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
    /// `PreToolUse` of `TodoWrite`: its progress.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub todos: Option<TodoCount>,
    /// The session's permission mode as the hook reports it (2.8e), shown read-only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    /// Nearest parent first.
    #[serde(default)]
    pub chain: Vec<Proc>,
}

/// The stdin fields we use. Serde skips the rest (prompt, last message) without keeping them.
#[derive(Deserialize)]
struct HookInput {
    session_id: String,
    #[serde(default)]
    cwd: String,
    hook_event_name: String,
    #[serde(default)]
    notification_type: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    permission_mode: Option<String>,
    #[serde(default)]
    tool_name: Option<String>,
    #[serde(default)]
    tool_input: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// `SessionStart`; `compact` keeps the current status (compaction can happen mid-task).
    SessionStart {
        compact: bool,
    },
    /// `UserPromptSubmit`.
    Prompt,
    /// `PreToolUse` (2.8d): a tool is about to run; the message carries its step line.
    ToolStart,
    /// `PostToolUse`: work goes on, so a permission prompt was answered.
    ToolDone,
    /// `Notification` for a permission prompt or a question.
    NeedsInput,
    /// `Notification` `idle_prompt`: Claude finished a while ago (also after an Esc, where `Stop` never fires).
    Idle,
    Stop,
    /// The turn ended on an API error.
    StopFailure,
    SessionEnd,
}

/// Maps a hook to what we track, or `None` for hooks and notification types we ignore.
fn event_of(name: &str, notification_type: Option<&str>, source: Option<&str>) -> Option<Event> {
    Some(match name {
        "SessionStart" => Event::SessionStart {
            compact: source == Some("compact"),
        },
        "UserPromptSubmit" => Event::Prompt,
        "PreToolUse" => Event::ToolStart,
        "PostToolUse" => Event::ToolDone,
        "Notification" => match notification_type? {
            "permission_prompt"
            | "elicitation_dialog"
            | "elicitation_url_dialog"
            | "agent_needs_input" => Event::NeedsInput,
            "idle_prompt" => Event::Idle,
            _ => return None,
        },
        "Stop" => Event::Stop,
        "StopFailure" => Event::StopFailure,
        "SessionEnd" => Event::SessionEnd,
        _ => return None,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub at_ms: i64,
    /// Claude Code's session id.
    pub session: String,
    pub cwd: String,
    pub event: Event,
    pub chain: Vec<Proc>,
    /// For `ToolStart`.
    pub step: Option<String>,
    pub todos: Option<TodoCount>,
    /// `default`, `acceptEdits`, `plan`, `auto`, … when the hook said.
    pub mode: Option<String>,
}

/// Forwarder side: the envelope for a hook's stdin, or `None` if it isn't one we track.
pub fn envelope(stdin: &[u8], at_ms: i64, chain: Vec<Proc>) -> Option<Envelope> {
    let input: HookInput = serde_json::from_slice(stdin).ok()?;
    let (step, todos) = match input.tool_name.as_deref() {
        Some(tool) if input.hook_event_name == "PreToolUse" => {
            let (step, todos) = crate::agent::step_of(tool, &input.tool_input, &input.cwd);
            (Some(step), todos)
        }
        _ => (None, None),
    };
    let envelope = Envelope {
        v: VERSION,
        at_ms,
        event: input.hook_event_name,
        session_id: input.session_id,
        cwd: input.cwd,
        notification_type: input.notification_type,
        source: input.source,
        step,
        todos,
        permission_mode: input.permission_mode,
        chain,
    };
    check(&envelope).map(|_| envelope)
}

/// Listener side: a validated message, or `None` for anything malformed, oversized or untracked.
pub fn decode(bytes: &[u8]) -> Option<Message> {
    if bytes.len() > MAX_MESSAGE {
        return None;
    }
    let envelope: Envelope = serde_json::from_slice(bytes).ok()?;
    let event = check(&envelope)?;
    Some(Message {
        at_ms: envelope.at_ms,
        session: envelope.session_id,
        cwd: envelope.cwd,
        event,
        chain: envelope.chain,
        step: envelope.step,
        todos: envelope.todos,
        mode: envelope.permission_mode,
    })
}

/// The checks every pipe message shares (`ask.rs` too): a plain session id, a one-line folder, a short chain.
pub fn origin_ok(session_id: &str, cwd: &str, chain: &[Proc], at_ms: i64) -> bool {
    let id_ok = !session_id.is_empty()
        && session_id.len() <= MAX_SESSION_ID
        && session_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    let cwd_ok = cwd.len() <= MAX_CWD && !cwd.chars().any(char::is_control);
    let chain_ok = chain.len() <= MAX_CHAIN
        && chain
            .iter()
            .all(|p| !p.exe.is_empty() && p.exe.len() <= MAX_EXE);
    id_ok && cwd_ok && chain_ok && at_ms > 0
}

fn check(e: &Envelope) -> Option<Event> {
    if e.v != VERSION || !origin_ok(&e.session_id, &e.cwd, &e.chain, e.at_ms) {
        return None;
    }
    let step_ok = e
        .step
        .as_deref()
        .is_none_or(|s| s.chars().count() <= MAX_STEP && !s.chars().any(char::is_control));
    let todos_ok = e
        .todos
        .is_none_or(|t| t.total <= MAX_TODOS && t.done <= t.total);
    let mode_ok = e
        .permission_mode
        .as_deref()
        .is_none_or(|m| m.len() <= 32 && m.chars().all(|c| c.is_ascii_alphanumeric()));
    if !step_ok || !todos_ok || !mode_ok {
        return None;
    }
    event_of(
        &e.event,
        e.notification_type.as_deref(),
        e.source.as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const AT: i64 = 1_793_000_000_000;

    fn chain() -> Vec<Proc> {
        vec![Proc {
            pid: 4242,
            exe: "claude.exe".into(),
            created: 133_000_000_000_000_000,
        }]
    }

    fn round_trip(stdin: &str) -> Option<Message> {
        let e = envelope(stdin.as_bytes(), AT, chain())?;
        decode(&serde_json::to_vec(&e).unwrap())
    }

    #[test]
    fn keeps_only_the_fields_it_needs() {
        let stdin = r#"{"session_id":"abc-123","transcript_path":"C:\\t.jsonl","cwd":"C:\\Users\\me\\wpill",
            "hook_event_name":"Stop","stop_hook_active":false,"last_assistant_message":"secret text"}"#;
        let e = envelope(stdin.as_bytes(), AT, chain()).unwrap();
        let wire = String::from_utf8(serde_json::to_vec(&e).unwrap()).unwrap();
        assert!(!wire.contains("secret"));
        assert!(!wire.contains("t.jsonl"));
        let m = decode(wire.as_bytes()).unwrap();
        assert_eq!(m.event, Event::Stop);
        assert_eq!(m.session, "abc-123");
        assert_eq!(m.cwd, r"C:\Users\me\wpill");
        assert_eq!(m.chain, chain());
    }

    #[test]
    fn maps_notification_types() {
        let n = |t: &str| {
            round_trip(&format!(
                r#"{{"session_id":"s","cwd":"C:\\p","hook_event_name":"Notification","notification_type":"{t}","message":"m"}}"#
            ))
            .map(|m| m.event)
        };
        assert_eq!(n("permission_prompt"), Some(Event::NeedsInput));
        assert_eq!(n("elicitation_dialog"), Some(Event::NeedsInput));
        assert_eq!(n("agent_needs_input"), Some(Event::NeedsInput));
        assert_eq!(n("idle_prompt"), Some(Event::Idle));
        assert_eq!(n("auth_success"), None);
        let untyped =
            r#"{"session_id":"s","cwd":"C:\\p","hook_event_name":"Notification","message":"m"}"#;
        assert_eq!(round_trip(untyped), None);
    }

    #[test]
    fn maps_the_other_events() {
        let ev = |name: &str, extra: &str| {
            round_trip(&format!(
                r#"{{"session_id":"s","cwd":"C:\\p","hook_event_name":"{name}"{extra}}}"#
            ))
            .map(|m| m.event)
        };
        assert_eq!(
            ev("SessionStart", r#","source":"startup""#),
            Some(Event::SessionStart { compact: false })
        );
        assert_eq!(
            ev("SessionStart", r#","source":"compact""#),
            Some(Event::SessionStart { compact: true })
        );
        assert_eq!(
            ev("UserPromptSubmit", r#","prompt":"hi""#),
            Some(Event::Prompt)
        );
        assert_eq!(ev("PostToolUse", ""), Some(Event::ToolDone));
        assert_eq!(
            ev("StopFailure", r#","error":"rate_limit""#),
            Some(Event::StopFailure)
        );
        assert_eq!(
            ev("SessionEnd", r#","reason":"other""#),
            Some(Event::SessionEnd)
        );
        assert_eq!(
            ev("PreToolUse", r#","tool_name":"Read""#),
            Some(Event::ToolStart)
        );
        assert_eq!(ev("SubagentStop", ""), None);
    }

    #[test]
    fn tool_starts_carry_a_step_and_never_the_input() {
        let stdin = r#"{"session_id":"s","cwd":"C:\\p","hook_event_name":"PreToolUse","tool_name":"Edit",
            "tool_input":{"file_path":"C:\\p\\src\\main.rs","old_string":"secret","new_string":"x"}}"#;
        let e = envelope(stdin.as_bytes(), AT, chain()).unwrap();
        let wire = String::from_utf8(serde_json::to_vec(&e).unwrap()).unwrap();
        assert!(!wire.contains("secret"));
        let m = decode(wire.as_bytes()).unwrap();
        assert_eq!(m.step.as_deref(), Some("Edit · src/main.rs"));
        assert_eq!(m.mode, None);
        let with_mode = stdin.replace(
            "\"PreToolUse\"",
            "\"PreToolUse\",\"permission_mode\":\"acceptEdits\"",
        );
        assert_eq!(
            round_trip(&with_mode).unwrap().mode.as_deref(),
            Some("acceptEdits")
        );
        assert_eq!(m.todos, None);
        let todo = r#"{"session_id":"s","cwd":"C:\\p","hook_event_name":"PreToolUse","tool_name":"TodoWrite",
            "tool_input":{"todos":[{"content":"a","status":"completed"},{"content":"b","status":"pending"}]}}"#;
        let m = round_trip(todo).unwrap();
        assert_eq!(m.todos, Some(TodoCount { done: 1, total: 2 }));
        let mut bad = e.clone();
        bad.step = Some("a\nb".into());
        assert_eq!(decode(&serde_json::to_vec(&bad).unwrap()), None);
        bad.step = None;
        bad.todos = Some(TodoCount { done: 3, total: 2 });
        assert_eq!(decode(&serde_json::to_vec(&bad).unwrap()), None);
    }

    #[test]
    fn rejects_malformed_and_hostile_input() {
        assert_eq!(envelope(b"not json", AT, vec![]), None);
        assert_eq!(envelope(br#"{"cwd":"x"}"#, AT, vec![]), None);
        let bad_id = r#"{"session_id":"a b/../c","cwd":"C:\\p","hook_event_name":"Stop"}"#;
        assert_eq!(envelope(bad_id.as_bytes(), AT, vec![]), None);
        let newline_cwd = r#"{"session_id":"s","cwd":"C:\\p\nq","hook_event_name":"Stop"}"#;
        assert_eq!(envelope(newline_cwd.as_bytes(), AT, vec![]), None);

        let good = envelope(
            br#"{"session_id":"s","cwd":"C:\\p","hook_event_name":"Stop"}"#,
            AT,
            vec![],
        )
        .unwrap();
        let wire = |e: &Envelope| serde_json::to_vec(e).unwrap();
        assert!(decode(&wire(&good)).is_some());
        assert_eq!(
            decode(&wire(&Envelope {
                v: 2,
                ..good.clone()
            })),
            None
        );
        assert_eq!(
            decode(&wire(&Envelope {
                at_ms: 0,
                ..good.clone()
            })),
            None
        );
        let long_chain = Envelope {
            chain: vec![chain()[0].clone(); MAX_CHAIN + 1],
            ..good.clone()
        };
        assert_eq!(decode(&wire(&long_chain)), None);
        let mut oversized = wire(&good);
        oversized.resize(MAX_MESSAGE + 1, b' ');
        assert_eq!(decode(&oversized), None);
        assert_eq!(decode(b"{\"v\":1"), None);
    }
}
