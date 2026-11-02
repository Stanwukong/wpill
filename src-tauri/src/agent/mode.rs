//! Permission modes for agent runs (Phase 2.8e, D40): Claude Code's own Shift+Tab cycle, Auto by default. Pure;
//! unit-tested.
//!
//! Only these four are ever passed or set: `bypassPermissions` and `dontAsk` are not offered. Questions
//! (`AskUserQuestion`) and plans reach the island in every mode; what a mode changes is which tool calls Claude Code
//! asks about at all.

use serde_json::json;

use crate::ipc::AgentMode;

/// The mode after `mode` when you click the chip: Ask → Accept edits → Plan → Auto → Ask, like Claude Code.
pub fn next(mode: AgentMode) -> AgentMode {
    match mode {
        AgentMode::Default => AgentMode::AcceptEdits,
        AgentMode::AcceptEdits => AgentMode::Plan,
        AgentMode::Plan => AgentMode::Auto,
        AgentMode::Auto => AgentMode::Default,
    }
}

/// `--permission-mode`'s value, and how stream-json reports it.
pub fn cli(mode: AgentMode) -> &'static str {
    match mode {
        AgentMode::Auto => "auto",
        AgentMode::Default => "default",
        AgentMode::AcceptEdits => "acceptEdits",
        AgentMode::Plan => "plan",
    }
}

/// A mode Claude Code reported (`system/init` or `system/status` `permissionMode`); `None` for anything Cay doesn't
/// offer.
pub fn parse(reported: &str) -> Option<AgentMode> {
    [
        AgentMode::Auto,
        AgentMode::Default,
        AgentMode::AcceptEdits,
        AgentMode::Plan,
    ]
    .into_iter()
    .find(|m| cli(*m) == reported)
}

/// The stream-json line that switches a running session's mode (verified with Claude Code 2.1.314: it answers with a
/// `control_response` and a `system/status` carrying the new `permissionMode`).
pub fn set_request(request_id: &str, mode: AgentMode) -> Vec<u8> {
    let mut line = json!({
        "type": "control_request",
        "request_id": request_id,
        "request": { "subtype": "set_permission_mode", "mode": cli(mode) },
    })
    .to_string()
    .into_bytes();
    line.push(b'\n');
    line
}

/// Whether Cay's own check before a desktop look or action (2.7c/d) is skipped: only in Auto, where Claude Code's
/// classifier decides as it does for every other tool (the author's call, D40). The glow and the input guard stay.
pub fn desktop_follows_claude(mode: AgentMode) -> bool {
    mode == AgentMode::Auto
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn cycles_like_claude_code() {
        let mut m = AgentMode::Default;
        let mut seen = vec![m];
        for _ in 0..4 {
            m = next(m);
            seen.push(m);
        }
        assert_eq!(
            seen,
            [
                AgentMode::Default,
                AgentMode::AcceptEdits,
                AgentMode::Plan,
                AgentMode::Auto,
                AgentMode::Default
            ]
        );
    }

    #[test]
    fn only_the_offered_modes_parse() {
        for m in [
            AgentMode::Auto,
            AgentMode::Default,
            AgentMode::AcceptEdits,
            AgentMode::Plan,
        ] {
            assert_eq!(parse(cli(m)), Some(m));
            assert_eq!(
                serde_json::to_value(m).unwrap(),
                Value::String(cli(m).into()),
                "IPC and Claude Code use the same names"
            );
        }
        assert_eq!(parse("bypassPermissions"), None);
        assert_eq!(parse("dontAsk"), None);
        assert!(serde_json::from_str::<AgentMode>("\"bypassPermissions\"").is_err());
    }

    #[test]
    fn set_request_line() {
        let line = set_request("mode-3", AgentMode::AcceptEdits);
        assert_eq!(line.last(), Some(&b'\n'));
        let v: Value = serde_json::from_slice(&line).unwrap();
        assert_eq!(v["type"], "control_request");
        assert_eq!(v["request_id"], "mode-3");
        assert_eq!(v["request"]["subtype"], "set_permission_mode");
        assert_eq!(v["request"]["mode"], "acceptEdits");
        assert!(desktop_follows_claude(AgentMode::Auto));
        assert!(!desktop_follows_claude(AgentMode::AcceptEdits));
    }
}
