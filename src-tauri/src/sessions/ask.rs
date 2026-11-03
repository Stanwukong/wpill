//! A terminal session's permission request, held for the island (Phase 2.8a, D34): what `cay.exe hook` sends for a
//! `PermissionRequest`, Cay's reply, and the hook output the forwarder prints for it. Pure; unit-tested.
//!
//! As with `protocol.rs`, any local process of this user can write to the pipe, so every field is bounded and
//! checked. The tool input is kept (the card shows the command or the file), with long strings cut.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::protocol::{origin_ok, Proc};
use crate::agent::interact::{self, PLAN_TOOL, QUESTION_TOOL};
use crate::ipc::AgentKind;

pub const VERSION: u32 = 1;
/// Largest request the forwarder sends and Cay reads.
pub const MAX_REQUEST: usize = 512 * 1024;
const MAX_TOOL: usize = 128;
const MAX_MODE: usize = 32;
const MAX_FIELDS: usize = 32;
/// Per string field of the tool input, in bytes (a `Write` holds the whole file).
const MAX_STRING: usize = 16 * 1024;
/// All strings of the tool input together, in bytes (a `MultiEdit` holds many).
const MAX_STRINGS: usize = 384 * 1024;
/// Items kept per array of the tool input.
const MAX_ITEMS: usize = 64;
/// Nesting kept in the tool input; anything deeper is kept whole if it's small, else dropped.
const MAX_DEPTH: usize = 3;
/// A value nested deeper than `MAX_DEPTH`, serialized; larger ones are dropped.
const MAX_VALUE: usize = 16 * 1024;
/// What Claude is told when you deny from the island.
const DENY_MESSAGE: &str = "The user denied this in Cay.";
const NO_ANSWER_MESSAGE: &str = "The user chose not to answer this in Cay.";
const PLAN_REJECTED_MESSAGE: &str =
    "The user rejected this plan in Cay. Ask what they would like changed.";

/// On the wire, forwarder → Cay, one per connection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub v: u32,
    /// Which agent asks (2.9a); left out for Claude Code.
    #[serde(default, skip_serializing_if = "AgentKind::is_claude")]
    pub agent: AgentKind,
    pub at_ms: i64,
    pub session_id: String,
    pub cwd: String,
    /// The session's permission mode (`default`, `acceptEdits`, `plan`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    pub tool_name: String,
    /// An object, with long strings cut (`trim_input`).
    pub tool_input: Value,
    /// Top-level fields of `tool_input` that were shortened or dropped, so the card never shows a wrong count.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cut: Vec<String>,
    /// Nearest parent first.
    #[serde(default)]
    pub chain: Vec<Proc>,
}

/// The stdin fields we use. Serde skips the rest (transcript path, permission suggestions).
#[derive(Deserialize)]
struct HookInput {
    session_id: String,
    #[serde(default)]
    cwd: String,
    hook_event_name: String,
    #[serde(default)]
    tool_name: String,
    #[serde(default)]
    tool_input: Value,
    #[serde(default)]
    permission_mode: Option<String>,
}

/// Cay → forwarder. `Pass`: no decision, so Claude Code's own prompt decides (timeout, Cay's own runs, a tool the
/// island can't answer, shutdown). `AskUserQuestion` and `ExitPlanMode` (2.8c) are allowed only with their answer:
/// `Answers` or `ApprovePlan`; a plain `Allow` for them is no decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Decision {
    Allow,
    Deny,
    Pass,
    /// Question text → the chosen label (multi-select labels joined by ", "), or your own words.
    Answers(BTreeMap<String, String>),
    ApprovePlan,
    /// Keep planning: a deny whose message is your feedback.
    Feedback(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reply {
    pub v: u32,
    pub decision: Decision,
}

impl Reply {
    pub fn new(decision: Decision) -> Self {
        Self {
            v: VERSION,
            decision,
        }
    }
}

/// Forwarder side: the request for a hook's stdin, or `None` if it isn't a `PermissionRequest` the island can answer.
pub fn request(stdin: &[u8], agent: AgentKind, at_ms: i64, chain: Vec<Proc>) -> Option<Request> {
    let input: HookInput = serde_json::from_slice(stdin).ok()?;
    if input.hook_event_name != "PermissionRequest" {
        return None;
    }
    let (tool_input, cut) = trim_input(input.tool_input);
    let request = Request {
        v: VERSION,
        agent,
        at_ms,
        session_id: input.session_id,
        cwd: input.cwd,
        permission_mode: input.permission_mode,
        tool_name: input.tool_name,
        tool_input,
        cut,
        chain,
    };
    check(&request).then_some(request)
}

/// Cay side: a validated request, or `None` for anything malformed, oversized or not ours to answer.
pub fn decode(bytes: &[u8]) -> Option<Request> {
    if bytes.len() > MAX_REQUEST {
        return None;
    }
    let request: Request = serde_json::from_slice(bytes).ok()?;
    check(&request).then_some(request)
}

/// Forwarder side: Cay's reply, or `None` (treated as `Pass`).
pub fn decode_reply(bytes: &[u8]) -> Option<Decision> {
    let reply: Reply = serde_json::from_slice(bytes).ok()?;
    (reply.v == VERSION).then_some(reply.decision)
}

/// What the forwarder prints for a decision on `tool`, whose input as Claude Code sent it is `original`; nothing for
/// `Pass` or for a decision that doesn't fit the tool, which leaves the prompt to Claude Code. A question's allow
/// echoes the original questions with your answers; a plan's echoes the original input.
pub fn hook_output(decision: &Decision, tool: &str, original: &Value) -> Option<String> {
    let decision = match (tool, decision) {
        (QUESTION_TOOL, Decision::Answers(answers)) => json!({
            "behavior": "allow",
            "updatedInput": interact::answered_input(original, answers)?,
        }),
        (QUESTION_TOOL, Decision::Deny) => {
            json!({ "behavior": "deny", "message": NO_ANSWER_MESSAGE })
        }
        (PLAN_TOOL, Decision::ApprovePlan) if original.is_object() => {
            json!({ "behavior": "allow", "updatedInput": original })
        }
        (PLAN_TOOL, Decision::Feedback(text)) => json!({
            "behavior": "deny",
            "message": format!(
                "The user sent this plan back from Cay with feedback:\n\n{}",
                interact::feedback(text)?
            ),
        }),
        (PLAN_TOOL, Decision::Deny) => {
            json!({ "behavior": "deny", "message": PLAN_REJECTED_MESSAGE })
        }
        (QUESTION_TOOL | PLAN_TOOL, _) => return None,
        (_, Decision::Allow) => json!({ "behavior": "allow" }),
        (_, Decision::Deny) => json!({ "behavior": "deny", "message": DENY_MESSAGE }),
        _ => return None,
    };
    Some(
        json!({
            "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": decision }
        })
        .to_string(),
    )
}

fn check(r: &Request) -> bool {
    let tool_ok = !r.tool_name.is_empty()
        && r.tool_name.len() <= MAX_TOOL
        && r.tool_name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.');
    let mode_ok = r
        .permission_mode
        .as_deref()
        .is_none_or(|m| m.len() <= MAX_MODE && m.chars().all(|c| c.is_ascii_alphanumeric()));
    let input_ok = r
        .tool_input
        .as_object()
        .is_some_and(|o| o.len() <= MAX_FIELDS);
    r.v == VERSION
        && origin_ok(&r.session_id, &r.cwd, &r.chain, r.at_ms)
        && tool_ok
        && mode_ok
        && input_ok
}

/// The input as an object of at most `MAX_FIELDS` fields: each string cut to `MAX_STRING` bytes (and all of them to
/// `MAX_STRINGS`), arrays to `MAX_ITEMS`, nesting to `MAX_DEPTH`. Also returns the top-level fields that lost
/// anything. Anything that isn't an object becomes an empty one.
fn trim_input(input: Value) -> (Value, Vec<String>) {
    let Value::Object(fields) = input else {
        return (Value::Object(Map::new()), Vec::new());
    };
    let mut budget = MAX_STRINGS;
    let mut cut_fields = Vec::new();
    let kept = fields
        .into_iter()
        .take(MAX_FIELDS)
        .filter_map(|(key, value)| {
            let mut lost = false;
            let value = trim_value(value, 0, &mut budget, &mut lost);
            if lost {
                cut_fields.push(key.clone());
            }
            Some((key, value?))
        })
        .collect();
    (Value::Object(kept), cut_fields)
}

fn trim_value(value: Value, depth: usize, budget: &mut usize, lost: &mut bool) -> Option<Value> {
    match value {
        Value::String(s) => {
            let len = s.len();
            let s = cut(s, MAX_STRING.min(*budget));
            *lost |= s.len() < len;
            *budget -= s.len();
            Some(Value::String(s))
        }
        Value::Array(items) if depth < MAX_DEPTH => {
            *lost |= items.len() > MAX_ITEMS;
            let items = items
                .into_iter()
                .take(MAX_ITEMS)
                .filter_map(|v| trim_value(v, depth + 1, budget, lost))
                .collect();
            Some(Value::Array(items))
        }
        Value::Object(fields) if depth < MAX_DEPTH => {
            *lost |= fields.len() > MAX_FIELDS;
            let fields = fields
                .into_iter()
                .take(MAX_FIELDS)
                .filter_map(|(k, v)| Some((k, trim_value(v, depth + 1, budget, lost)?)))
                .collect();
            Some(Value::Object(fields))
        }
        other if other.to_string().len() <= MAX_VALUE => Some(other),
        _ => {
            *lost = true;
            None
        }
    }
}

fn cut(mut s: String, max: usize) -> String {
    if s.len() > max {
        let mut at = max;
        while !s.is_char_boundary(at) {
            at -= 1;
        }
        s.truncate(at);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const AT: i64 = 1_793_000_000_000;

    fn stdin(tool: &str, input: &str) -> String {
        format!(
            r#"{{"session_id":"abc-123","transcript_path":"C:\\t.jsonl","cwd":"C:\\Users\\me\\wpill",
            "permission_mode":"default","hook_event_name":"PermissionRequest","tool_name":"{tool}",
            "tool_input":{input},"permission_suggestions":[{{"type":"addRules","rules":[],"behavior":"allow",
            "destination":"localSettings"}}]}}"#
        )
    }

    #[test]
    fn round_trips_a_shell_request() {
        let r = request(
            stdin(
                "Bash",
                r#"{"command":"rm -rf node_modules","description":"d"}"#,
            )
            .as_bytes(),
            AgentKind::Claude,
            AT,
            vec![],
        )
        .unwrap();
        let wire = serde_json::to_vec(&r).unwrap();
        assert!(!String::from_utf8_lossy(&wire).contains("t.jsonl"));
        let back = decode(&wire).unwrap();
        assert_eq!(back.tool_name, "Bash");
        assert_eq!(back.tool_input["command"], "rm -rf node_modules");
        assert_eq!(back.permission_mode.as_deref(), Some("default"));
        assert_eq!(back.session_id, "abc-123");
    }

    #[test]
    fn only_permission_requests_the_island_can_answer() {
        let other =
            r#"{"session_id":"s","cwd":"C:\\p","hook_event_name":"PreToolUse","tool_name":"Bash"}"#;
        assert_eq!(
            request(other.as_bytes(), AgentKind::Claude, AT, vec![]),
            None
        );
        assert!(request(
            stdin("AskUserQuestion", "{}").as_bytes(),
            AgentKind::Claude,
            AT,
            vec![]
        )
        .is_some());
        assert!(request(
            stdin("ExitPlanMode", "{}").as_bytes(),
            AgentKind::Claude,
            AT,
            vec![]
        )
        .is_some());
        assert!(request(
            stdin("mcp__github__create_issue", "{}").as_bytes(),
            AgentKind::Claude,
            AT,
            vec![]
        )
        .is_some());
    }

    #[test]
    fn cuts_long_input() {
        let long = "é".repeat(MAX_STRING);
        let big: Vec<u32> = (0..10_000).collect();
        let input = json!({ "file_path": "C:\\a.txt", "content": long, "lines": big });
        let r = request(
            stdin("Write", &input.to_string()).as_bytes(),
            AgentKind::Claude,
            AT,
            vec![],
        )
        .unwrap();
        let content = r.tool_input["content"].as_str().unwrap();
        assert!(content.len() <= MAX_STRING && content.ends_with('é'));
        assert_eq!(r.tool_input["lines"].as_array().unwrap().len(), MAX_ITEMS);
        assert_eq!(r.tool_input["file_path"], "C:\\a.txt");
        assert_eq!(r.cut, ["content", "lines"]);
        assert!(serde_json::to_vec(&r).unwrap().len() < MAX_REQUEST);

        // A MultiEdit keeps its edits, each string cut, and all of them within the budget.
        let edit = json!({ "old_string": "a".repeat(MAX_STRING + 1), "new_string": "b" });
        let edits: Vec<Value> = (0..40).map(|_| edit.clone()).collect();
        let input = json!({ "file_path": "C:\\a.txt", "edits": edits });
        let r = request(
            stdin("MultiEdit", &input.to_string()).as_bytes(),
            AgentKind::Claude,
            AT,
            vec![],
        )
        .unwrap();
        assert_eq!(r.cut, ["edits"]);
        assert_eq!(r.tool_input["edits"].as_array().unwrap().len(), 40);
        assert_eq!(
            r.tool_input["edits"][0]["old_string"]
                .as_str()
                .unwrap()
                .len(),
            MAX_STRING
        );
        assert_eq!(
            r.tool_input["edits"][39]["old_string"], "",
            "past the budget"
        );
        assert!(serde_json::to_vec(&r).unwrap().len() < MAX_REQUEST);

        let small = request(
            stdin("Bash", r#"{"command":"ls"}"#).as_bytes(),
            AgentKind::Claude,
            AT,
            vec![],
        )
        .unwrap();
        assert!(small.cut.is_empty());
    }

    #[test]
    fn rejects_malformed_and_hostile_requests() {
        let good = request(
            stdin("Bash", r#"{"command":"ls"}"#).as_bytes(),
            AgentKind::Claude,
            AT,
            vec![],
        )
        .unwrap();
        let wire = |r: &Request| serde_json::to_vec(r).unwrap();
        assert!(decode(&wire(&good)).is_some());
        let bad = [
            Request {
                v: 2,
                ..good.clone()
            },
            Request {
                tool_name: "Bash; rm".into(),
                ..good.clone()
            },
            Request {
                tool_input: json!("ls"),
                ..good.clone()
            },
            Request {
                permission_mode: Some("by pass".into()),
                ..good.clone()
            },
            Request {
                session_id: "../x".into(),
                ..good.clone()
            },
            Request {
                at_ms: 0,
                ..good.clone()
            },
        ];
        for r in &bad {
            assert_eq!(decode(&wire(r)), None, "{r:?}");
        }
        let mut oversized = wire(&good);
        oversized.resize(MAX_REQUEST + 1, b' ');
        assert_eq!(decode(&oversized), None);
        assert_eq!(decode(b"not json"), None);
    }

    #[test]
    fn questions_and_plans_need_their_answer() {
        let original = json!({ "questions": [{ "question": "Which?", "header": "H", "multiSelect": false,
            "options": [{ "label": "A", "description": "" }, { "label": "B", "description": "" }] }] });
        let out = |d: &Decision, tool: &str, input: &Value| {
            hook_output(d, tool, input).map(|s| serde_json::from_str::<Value>(&s).unwrap())
        };
        assert_eq!(
            out(&Decision::Allow, QUESTION_TOOL, &original),
            None,
            "never a bare allow"
        );
        let mut answers = BTreeMap::new();
        answers.insert("Which?".to_string(), "B".to_string());
        let allow = out(&Decision::Answers(answers), QUESTION_TOOL, &original).unwrap();
        let decision = &allow["hookSpecificOutput"]["decision"];
        assert_eq!(decision["behavior"], "allow");
        assert_eq!(decision["updatedInput"]["questions"], original["questions"]);
        assert_eq!(decision["updatedInput"]["answers"]["Which?"], "B");
        assert_eq!(
            out(
                &Decision::Answers(BTreeMap::new()),
                QUESTION_TOOL,
                &original
            ),
            None,
            "unanswered"
        );
        let skip = out(&Decision::Deny, QUESTION_TOOL, &original).unwrap();
        assert_eq!(skip["hookSpecificOutput"]["decision"]["behavior"], "deny");

        let plan = json!({ "plan": "# P", "planFilePath": "C:\\p.md" });
        assert_eq!(out(&Decision::Allow, PLAN_TOOL, &plan), None);
        let approve = out(&Decision::ApprovePlan, PLAN_TOOL, &plan).unwrap();
        assert_eq!(
            approve["hookSpecificOutput"]["decision"]["updatedInput"],
            plan
        );
        let back = out(&Decision::Feedback(" add tests ".into()), PLAN_TOOL, &plan).unwrap();
        let message = back["hookSpecificOutput"]["decision"]["message"]
            .as_str()
            .unwrap();
        assert!(message.ends_with("\n\nadd tests"), "{message}");
        assert_eq!(
            out(&Decision::Feedback("  ".into()), PLAN_TOOL, &plan),
            None
        );
        assert_eq!(
            out(&Decision::ApprovePlan, "Bash", &plan),
            None,
            "only for its tool"
        );
    }

    #[test]
    fn replies_and_hook_output() {
        let wire = |d| serde_json::to_vec(&Reply::new(d)).unwrap();
        assert_eq!(decode_reply(&wire(Decision::Allow)), Some(Decision::Allow));
        let plan = Decision::Feedback("more tests".into());
        assert_eq!(decode_reply(&wire(plan.clone())), Some(plan));
        assert_eq!(decode_reply(br#"{"v":9,"decision":"allow"}"#), None);
        assert_eq!(decode_reply(b"junk"), None);

        let allow: Value =
            serde_json::from_str(&hook_output(&Decision::Allow, "Bash", &json!({})).unwrap())
                .unwrap();
        assert_eq!(
            allow["hookSpecificOutput"]["hookEventName"],
            "PermissionRequest"
        );
        assert_eq!(
            allow["hookSpecificOutput"]["decision"],
            json!({ "behavior": "allow" })
        );
        let deny: Value =
            serde_json::from_str(&hook_output(&Decision::Deny, "Bash", &json!({})).unwrap())
                .unwrap();
        assert_eq!(deny["hookSpecificOutput"]["decision"]["behavior"], "deny");
        assert_eq!(
            deny["hookSpecificOutput"]["decision"]["message"],
            DENY_MESSAGE
        );
        assert_eq!(hook_output(&Decision::Pass, "Bash", &json!({})), None);
    }
}
