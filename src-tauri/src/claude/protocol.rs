//! The pipe message: what the forwarder keeps from a hook's stdin, and the listener's defensive parsing.
//! Pure; unit-tested.
//!
//! Any local process of this user can write to the pipe, so every field is bounded and checked. The
//! forwarder only copies the fields below: prompt, transcript and assistant text never reach the pipe.

use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;
/// Largest message the listener reads; anything longer is dropped whole.
pub const MAX_MESSAGE: usize = 16 * 1024;
const MAX_SESSION_ID: usize = 128;
const MAX_CWD: usize = 1024;
const MAX_CHAIN: usize = 16;
const MAX_EXE: usize = 260;

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// `SessionStart`; `compact` keeps the current status (compaction can happen mid-task).
    SessionStart {
        compact: bool,
    },
    /// `UserPromptSubmit`.
    Prompt,
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
}

/// Forwarder side: the envelope for a hook's stdin, or `None` if it isn't one we track.
pub fn envelope(stdin: &[u8], at_ms: i64, chain: Vec<Proc>) -> Option<Envelope> {
    let input: HookInput = serde_json::from_slice(stdin).ok()?;
    let envelope = Envelope {
        v: VERSION,
        at_ms,
        event: input.hook_event_name,
        session_id: input.session_id,
        cwd: input.cwd,
        notification_type: input.notification_type,
        source: input.source,
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
    })
}

fn check(e: &Envelope) -> Option<Event> {
    let id_ok = !e.session_id.is_empty()
        && e.session_id.len() <= MAX_SESSION_ID
        && e.session_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    let cwd_ok = e.cwd.len() <= MAX_CWD && !e.cwd.chars().any(char::is_control);
    let chain_ok = e.chain.len() <= MAX_CHAIN
        && e.chain
            .iter()
            .all(|p| !p.exe.is_empty() && p.exe.len() <= MAX_EXE);
    if e.v != VERSION || !id_ok || !cwd_ok || !chain_ok || e.at_ms <= 0 {
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
        assert_eq!(ev("PreToolUse", ""), None);
        assert_eq!(ev("SubagentStop", ""), None);
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
