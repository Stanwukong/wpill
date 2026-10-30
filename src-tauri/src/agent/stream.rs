//! Claude Code's `--output-format stream-json` lines → the few events a run needs. Pure; unit-tested.
//!
//! Verified against 2.1.123, 2.1.159 and 2.1.312 (PLAN.md Phase 2.7): `system/init` opens every turn with the
//! session id, `assistant` messages carry `text` / `tool_use` blocks, `user` messages carry `tool_result` blocks,
//! and `result` closes each turn. Everything else (hooks, rate limits, retries, thinking) is ignored, and so are
//! lines that don't parse: a newer Claude Code may add fields or types at any time.

use serde_json::Value;

/// Longest text kept per transcript entry; the command window shows the rest as cut.
pub const MAX_TEXT: usize = 20_000;
pub const MAX_TOOL_RESULT: usize = 2_000;

#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    Init {
        session_id: String,
        model: Option<String>,
    },
    /// Claude's text in the main conversation (subagents' text is left out).
    Text(String),
    Tool {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        id: String,
        is_error: bool,
        text: String,
    },
    Result(TurnResult),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TurnResult {
    /// `success`, `error_max_turns`, `error_max_budget_usd`, `error_during_execution`, …
    pub subtype: String,
    pub is_error: bool,
    pub num_turns: u32,
    /// The session's total so far (Claude Code's estimate).
    pub total_cost_usd: Option<f64>,
    /// Claude's final text, or the failure (e.g. missing login) on success-shaped errors.
    pub result: Option<String>,
    pub errors: Vec<String>,
}

pub fn parse(line: &[u8]) -> Vec<StreamEvent> {
    let Ok(m) = serde_json::from_slice::<Value>(line) else {
        return Vec::new();
    };
    let str_of = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    let main = m.get("parent_tool_use_id").is_none_or(Value::is_null);
    match m.get("type").and_then(Value::as_str) {
        Some("system") if m.get("subtype").and_then(Value::as_str) == Some("init") => {
            match str_of(&m, "session_id") {
                Some(session_id) => vec![StreamEvent::Init {
                    session_id,
                    model: str_of(&m, "model"),
                }],
                None => Vec::new(),
            }
        }
        Some("assistant") => blocks(&m)
            .filter_map(|b| match b.get("type").and_then(Value::as_str)? {
                "text" if main => {
                    let text = b.get("text").and_then(Value::as_str)?.trim();
                    (!text.is_empty()).then(|| StreamEvent::Text(cut(text, MAX_TEXT)))
                }
                "tool_use" => Some(StreamEvent::Tool {
                    id: str_of(b, "id").unwrap_or_default(),
                    name: str_of(b, "name")?,
                    input: b.get("input").cloned().unwrap_or(Value::Null),
                }),
                _ => None,
            })
            .collect(),
        Some("user") => blocks(&m)
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
            .map(|b| StreamEvent::ToolResult {
                id: str_of(b, "tool_use_id").unwrap_or_default(),
                is_error: b.get("is_error").and_then(Value::as_bool).unwrap_or(false),
                text: cut(&content_text(b.get("content")), MAX_TOOL_RESULT),
            })
            .collect(),
        Some("result") => vec![StreamEvent::Result(TurnResult {
            subtype: str_of(&m, "subtype").unwrap_or_default(),
            is_error: m.get("is_error").and_then(Value::as_bool).unwrap_or(false),
            num_turns: m
                .get("num_turns")
                .and_then(Value::as_u64)
                .map_or(0, |n| n.min(u64::from(u32::MAX)) as u32),
            total_cost_usd: m.get("total_cost_usd").and_then(Value::as_f64),
            result: str_of(&m, "result").map(|r| cut(&r, MAX_TEXT)),
            errors: m
                .get("errors")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
        })],
        _ => Vec::new(),
    }
}

/// The message's content blocks (a plain string content has none).
fn blocks(m: &Value) -> impl Iterator<Item = &Value> {
    m.pointer("/message/content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

/// A tool result's content: a string, or text blocks (images and the rest are skipped).
fn content_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// At most `max` chars, marked when cut.
pub fn cut(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((i, _)) => format!("{}\n… (cut)", &text[..i]),
        None => text.to_string(),
    }
}

/// A user message for `--input-format stream-json`, one line with its newline.
pub fn user_message(text: &str) -> Vec<u8> {
    let message = serde_json::json!({
        "type": "user",
        "message": { "role": "user", "content": text },
        "parent_tool_use_id": null,
        "session_id": "",
    });
    let mut line = serde_json::to_vec(&message).unwrap_or_default();
    line.push(b'\n');
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    // Trimmed from real 2.1.123 / 2.1.312 output.
    const INIT: &str = r#"{"type":"system","subtype":"init","cwd":"C:\\x","session_id":"432c70f9-8329","tools":["Bash"],"mcp_servers":[{"name":"topbar","status":"connected"}],"model":"claude-haiku-4-5-20251001","permissionMode":"default"}"#;
    const TOOL: &str = r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"..."},{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"touch probe.txt"}}]},"parent_tool_use_id":null,"session_id":"s"}"#;
    const DENIED: &str = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"The user denied this from TopBar.","is_error":true,"tool_use_id":"toolu_1"}]},"parent_tool_use_id":null}"#;
    const RESULT: &str = r#"{"type":"result","subtype":"error_max_turns","is_error":true,"num_turns":2,"total_cost_usd":0.0325,"errors":["Reached maximum number of turns (1)"],"terminal_reason":"max_turns"}"#;

    #[test]
    fn init_tool_and_denied_result() {
        assert_eq!(
            parse(INIT.as_bytes()),
            vec![StreamEvent::Init {
                session_id: "432c70f9-8329".into(),
                model: Some("claude-haiku-4-5-20251001".into()),
            }]
        );
        assert_eq!(
            parse(TOOL.as_bytes()),
            vec![StreamEvent::Tool {
                id: "toolu_1".into(),
                name: "Bash".into(),
                input: serde_json::json!({ "command": "touch probe.txt" }),
            }]
        );
        assert_eq!(
            parse(DENIED.as_bytes()),
            vec![StreamEvent::ToolResult {
                id: "toolu_1".into(),
                is_error: true,
                text: "The user denied this from TopBar.".into(),
            }]
        );
    }

    #[test]
    fn result_with_errors() {
        let [StreamEvent::Result(r)] = &parse(RESULT.as_bytes())[..] else {
            panic!("expected one result");
        };
        assert_eq!(r.subtype, "error_max_turns");
        assert!(r.is_error);
        assert_eq!(r.num_turns, 2);
        assert_eq!(r.total_cost_usd, Some(0.0325));
        assert_eq!(r.errors, vec!["Reached maximum number of turns (1)"]);
    }

    #[test]
    fn subagent_text_hooks_and_garbage_are_skipped() {
        let sub = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"inner"}]},"parent_tool_use_id":"toolu_9"}"#;
        assert!(parse(sub.as_bytes()).is_empty());
        let hook = r#"{"type":"system","subtype":"hook_started","hook_name":"SessionStart"}"#;
        assert!(parse(hook.as_bytes()).is_empty());
        assert!(parse(b"not json").is_empty());
        assert!(parse(br#"{"type":"stream_event"}"#).is_empty());
        let blocks = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t","content":[{"type":"text","text":"a"},{"type":"image"},{"type":"text","text":"b"}]}]}}"#;
        assert_eq!(
            parse(blocks.as_bytes()),
            vec![StreamEvent::ToolResult {
                id: "t".into(),
                is_error: false,
                text: "a\nb".into()
            }]
        );
    }

    #[test]
    fn cut_and_user_message() {
        assert_eq!(cut("abc", 5), "abc");
        assert_eq!(cut("abcdef", 3), "abc\n… (cut)");
        let line = user_message("hi \"there\"");
        assert!(line.ends_with(b"\n"));
        let v: Value = serde_json::from_slice(&line).unwrap();
        assert_eq!(v["message"]["content"], "hi \"there\"");
        assert_eq!(v["type"], "user");
    }
}
