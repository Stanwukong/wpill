//! OpenCode's tools as Claude Code's (2.9a): the plugin forwards OpenCode's own tool names and arguments, and the
//! forwarder renames them here, so the cards, previews and step lines read them like any other session's. Pure;
//! unit-tested.

use serde_json::{Map, Value};

/// OpenCode tool → the Claude Code tool Cay already describes. Unknown tools (MCP, custom) keep their name.
fn tool_name(tool: &str) -> &str {
    match tool {
        "bash" => "Bash",
        "edit" => "Edit",
        "multiedit" => "MultiEdit",
        "write" => "Write",
        "read" => "Read",
        "glob" => "Glob",
        "grep" => "Grep",
        "list" => "LS",
        "webfetch" => "WebFetch",
        "websearch" => "WebSearch",
        "todowrite" => "TodoWrite",
        "todoread" => "TodoRead",
        "task" => "Task",
        // Its patch tool takes Codex's `*** Begin Patch` format (`agent/patch.rs`).
        "patch" | "apply_patch" => "apply_patch",
        other => other,
    }
}

/// Argument names: OpenCode's camelCase → Claude Code's snake_case. `filepath` comes from a permission's metadata.
fn key(name: &str) -> &str {
    match name {
        "filePath" | "filepath" => "file_path",
        "oldString" => "old_string",
        "newString" => "new_string",
        "replaceAll" => "replace_all",
        "patchText" => "command",
        other => other,
    }
}

fn rename_keys(input: Value) -> Value {
    match input {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| {
                    let v = match (k.as_str(), v) {
                        // A multiedit's edits use the same names.
                        ("edits", Value::Array(items)) => {
                            Value::Array(items.into_iter().map(rename_keys).collect())
                        }
                        (_, v) => v,
                    };
                    (key(&k).to_string(), v)
                })
                .collect::<Map<_, _>>(),
        ),
        other => other,
    }
}

/// The plugin's hook JSON with Claude Code's tool name and argument names, or `None` if it isn't JSON.
pub fn claude_shaped(stdin: &[u8]) -> Option<Vec<u8>> {
    let mut input: Value = serde_json::from_slice(stdin).ok()?;
    let obj = input.as_object_mut()?;
    if let Some(name) = obj.get("tool_name").and_then(Value::as_str) {
        let name = tool_name(name).to_string();
        obj.insert("tool_name".into(), Value::String(name));
    }
    if let Some(args) = obj.remove("tool_input") {
        obj.insert("tool_input".into(), rename_keys(args));
    }
    serde_json::to_vec(&input).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn shaped(input: Value) -> Value {
        serde_json::from_slice(&claude_shaped(&serde_json::to_vec(&input).unwrap()).unwrap())
            .unwrap()
    }

    #[test]
    fn tools_read_like_claude_codes() {
        let edit = shaped(json!({
            "session_id": "ses_1", "hook_event_name": "PermissionRequest", "tool_name": "edit",
            "tool_input": { "filePath": "C:\\p\\a.rs", "oldString": "x", "newString": "y", "replaceAll": true }
        }));
        assert_eq!(edit["tool_name"], json!("Edit"));
        assert_eq!(
            edit["tool_input"],
            json!({ "file_path": "C:\\p\\a.rs", "old_string": "x", "new_string": "y", "replace_all": true })
        );
        assert_eq!(edit["session_id"], json!("ses_1"));

        let bash = shaped(
            json!({ "tool_name": "bash", "tool_input": { "command": "ls", "description": "list" } }),
        );
        assert_eq!(
            (
                bash["tool_name"].clone(),
                bash["tool_input"]["command"].clone()
            ),
            (json!("Bash"), json!("ls"))
        );

        let multi = shaped(json!({ "tool_name": "multiedit", "tool_input": {
            "filePath": "a.rs", "edits": [{ "oldString": "a", "newString": "b" }] } }));
        assert_eq!(
            multi["tool_input"]["edits"][0],
            json!({ "old_string": "a", "new_string": "b" })
        );

        let patch = shaped(
            json!({ "tool_name": "patch", "tool_input": { "patchText": "*** Begin Patch" } }),
        );
        assert_eq!(patch["tool_name"], json!("apply_patch"));
        assert_eq!(patch["tool_input"]["command"], json!("*** Begin Patch"));

        // A permission's metadata, when the plugin had no arguments for the call.
        let meta = shaped(
            json!({ "tool_name": "edit", "tool_input": { "filepath": "b.rs", "diff": "-a\n+b" } }),
        );
        assert_eq!(meta["tool_input"]["file_path"], json!("b.rs"));

        // MCP and custom tools keep their names; events without a tool pass through.
        assert_eq!(
            shaped(json!({ "tool_name": "github_create_issue" }))["tool_name"],
            json!("github_create_issue")
        );
        assert_eq!(
            shaped(json!({ "hook_event_name": "Stop" })),
            json!({ "hook_event_name": "Stop" })
        );
        assert_eq!(claude_shaped(b"not json"), None);
    }
}
