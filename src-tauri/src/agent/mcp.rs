//! TopBar's MCP server, message by message: JSON-RPC 2.0 over the bridge (one message per pipe message).
//! Pure; unit-tested. `approve` is what Claude Code calls through `--permission-prompt-tool mcp__topbar__approve`;
//! the desktop tools (2.7c, `tools.rs`) are what Claude itself calls. Its input is `{ tool_name, input, tool_use_id }` and its
//! reply is a text block holding `{"behavior":"allow","updatedInput":…}` or `{"behavior":"deny","message":…}`
//! (checked against 2.1.123 and 2.1.312; the docs don't spell it out).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const SERVER_NAME: &str = "topbar";
pub const APPROVE_TOOL: &str = "approve";
/// Newest first; an unknown client version gets the newest.
const PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// The bridge's first message on a new connection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hello {
    pub topbar: u32,
    pub run: String,
    pub token: String,
}

pub const HELLO_VERSION: u32 = 1;

#[derive(Debug, PartialEq)]
pub enum Action {
    /// Send this back as it is.
    Reply(Value),
    /// Ask the user; answer later with `decision`.
    Approve {
        rpc_id: Value,
        tool_name: String,
        input: Value,
    },
    /// One of the desktop tools (`tools.rs`): check its approval, then look.
    Look {
        rpc_id: Value,
        name: &'static str,
        args: Value,
    },
    /// One of the desktop actions (2.7d, `actions.rs`): check its approval, then act.
    Act {
        rpc_id: Value,
        name: &'static str,
        args: Value,
    },
    /// Claude Code gave up on a call (`notifications/cancelled`): withdraw its approval or stop its action.
    Cancel { request_id: Value },
    /// A notification or response: nothing to send.
    Ignore,
}

pub fn handle(msg: &Value) -> Action {
    let Some(method) = msg.get("method").and_then(Value::as_str) else {
        return Action::Ignore;
    };
    if method == "notifications/cancelled" {
        return match msg.pointer("/params/requestId") {
            Some(id) if !id.is_null() => Action::Cancel {
                request_id: id.clone(),
            },
            _ => Action::Ignore,
        };
    }
    // Requests have an id; notifications (`notifications/initialized`, `…/cancelled`) don't.
    let Some(id) = msg.get("id").cloned() else {
        return Action::Ignore;
    };
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str);
            let version = asked
                .filter(|v| PROTOCOL_VERSIONS.contains(v))
                .unwrap_or(PROTOCOL_VERSIONS[0]);
            Action::Reply(result(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION") },
                }),
            ))
        }
        "ping" => Action::Reply(result(id, json!({}))),
        "tools/list" => {
            let mut tools = vec![approve_tool()];
            tools.extend(super::tools::definitions());
            tools.extend(super::actions::definitions());
            Action::Reply(result(id, json!({ "tools": tools })))
        }
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str);
            let args = params.get("arguments");
            let tool_name = args
                .and_then(|a| a.get("tool_name"))
                .and_then(Value::as_str);
            match (name, tool_name) {
                (Some(APPROVE_TOOL), Some(tool_name)) => Action::Approve {
                    rpc_id: id,
                    tool_name: tool_name.to_string(),
                    input: args
                        .and_then(|a| a.get("input"))
                        .cloned()
                        .unwrap_or(json!({})),
                },
                (Some(APPROVE_TOOL), None) => {
                    Action::Reply(error(id, -32602, "approve needs tool_name and input"))
                }
                (Some(name), _) if super::tools::NAMES.contains(&name) => Action::Look {
                    rpc_id: id,
                    name: super::tools::NAMES
                        .iter()
                        .copied()
                        .find(|n| *n == name)
                        .unwrap_or_default(),
                    args: args.cloned().unwrap_or(json!({})),
                },
                (Some(name), _) if super::actions::NAMES.contains(&name) => Action::Act {
                    rpc_id: id,
                    name: super::actions::NAMES
                        .iter()
                        .copied()
                        .find(|n| *n == name)
                        .unwrap_or_default(),
                    args: args.cloned().unwrap_or(json!({})),
                },
                _ => Action::Reply(error(id, -32602, "unknown tool")),
            }
        }
        _ => Action::Reply(error(id, -32601, "method not found")),
    }
}

/// The reply to an `approve` call.
pub fn decision(rpc_id: Value, allow: bool, input: Value, message: &str) -> Value {
    let answer = if allow {
        json!({ "behavior": "allow", "updatedInput": input })
    } else {
        json!({ "behavior": "deny", "message": message })
    };
    result(
        rpc_id,
        json!({ "content": [{ "type": "text", "text": answer.to_string() }] }),
    )
}

pub fn result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

pub fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn approve_tool() -> Value {
    json!({
        "name": APPROVE_TOOL,
        "description": "Asks the user in TopBar's island to allow or deny a tool call. Used by Claude Code as its \
                        permission prompt tool; not for direct use.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "tool_name": { "type": "string" },
                "input": { "type": "object" },
                "tool_use_id": { "type": "string" },
            },
            "required": ["tool_name", "input"],
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake_and_list() {
        let init = json!({ "jsonrpc": "2.0", "id": 0, "method": "initialize",
            "params": { "protocolVersion": "2025-06-18", "capabilities": {} } });
        let Action::Reply(r) = handle(&init) else {
            panic!()
        };
        assert_eq!(r["id"], 0);
        assert_eq!(r["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(r["result"]["serverInfo"]["name"], "topbar");

        let future = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": "2099-01-01" } });
        let Action::Reply(r) = handle(&future) else {
            panic!()
        };
        assert_eq!(r["result"]["protocolVersion"], PROTOCOL_VERSIONS[0]);

        let list = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" });
        let Action::Reply(r) = handle(&list) else {
            panic!()
        };
        assert_eq!(r["result"]["tools"][0]["name"], "approve");
        assert_eq!(r["result"]["tools"][3]["name"], "screenshot");

        let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert_eq!(handle(&note), Action::Ignore);
        let unknown = json!({ "jsonrpc": "2.0", "id": "x", "method": "resources/list" });
        let Action::Reply(r) = handle(&unknown) else {
            panic!()
        };
        assert_eq!(r["error"]["code"], -32601);
    }

    #[test]
    fn approve_call_and_replies() {
        // As sent by Claude Code 2.1.123.
        let call = json!({ "method": "tools/call", "jsonrpc": "2.0", "id": 2, "params": {
            "name": "approve",
            "arguments": { "tool_name": "Bash", "input": { "command": "touch probe.txt" },
                           "tool_use_id": "toolu_013" },
            "_meta": { "progressToken": 2 } } });
        assert_eq!(
            handle(&call),
            Action::Approve {
                rpc_id: json!(2),
                tool_name: "Bash".into(),
                input: json!({ "command": "touch probe.txt" }),
            }
        );
        let allow = decision(json!(2), true, json!({ "command": "x" }), "");
        let text = allow["result"]["content"][0]["text"].as_str().unwrap();
        let answer: Value = serde_json::from_str(text).unwrap();
        assert_eq!(
            answer,
            json!({ "behavior": "allow", "updatedInput": { "command": "x" } })
        );
        let deny = decision(json!(2), false, Value::Null, "no");
        let text = deny["result"]["content"][0]["text"].as_str().unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(text).unwrap(),
            json!({ "behavior": "deny", "message": "no" })
        );

        let look = json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": { "name": "ui_tree", "arguments": { "hwnd": 5 } } });
        assert_eq!(
            handle(&look),
            Action::Look {
                rpc_id: json!(4),
                name: "ui_tree",
                args: json!({ "hwnd": 5 }),
            }
        );

        let act = json!({ "jsonrpc": "2.0", "id": 5, "method": "tools/call",
            "params": { "name": "click", "arguments": { "hwnd": 5, "element": 2 } } });
        assert!(matches!(handle(&act), Action::Act { name: "click", .. }));
        let cancel = json!({ "jsonrpc": "2.0", "method": "notifications/cancelled",
            "params": { "requestId": 5, "reason": "AbortError" } });
        assert_eq!(
            handle(&cancel),
            Action::Cancel {
                request_id: json!(5)
            }
        );

        let other = json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": "shell", "arguments": {} } });
        let Action::Reply(r) = handle(&other) else {
            panic!()
        };
        assert_eq!(r["error"]["code"], -32602);
    }
}
