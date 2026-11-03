//! The OpenCode plugin Cay installs (2.9a, D47): `cay.js` in OpenCode's `plugins` folder. Pure; unit-tested.
//!
//! OpenCode loads it at startup and it forwards events to the same `cay.exe hook` Claude Code and Codex use, as
//! Claude-shaped hook JSON. A `permission.asked` waits for the forwarder's decision and answers through OpenCode's
//! own client (`permission.ask` is declared but never called in 1.19.5); with no decision it leaves OpenCode's prompt
//! alone, and an answer there stops the waiting forwarder. Everything fails open: a spawn error, Cay closed, or a
//! reply OpenCode refuses changes nothing.

use std::path::Path;

/// The oldest OpenCode checked with this plugin (2026-11-03).
pub const MIN_VERSION: (u32, u32, u32) = (1, 19, 5);
pub const FILE_NAME: &str = "cay.js";
/// On the plugin's second line; a `cay.js` without it isn't Cay's and is never replaced or removed.
const MARKER: &str = "cay-plugin:";
const VERSION: u32 = 1;

const TEMPLATE: &str = r##"// Cay: shows OpenCode sessions in Cay's island and lets you answer permission requests there.
// cay-plugin: __VERSION__. Written by Cay's tray item "Connect OpenCode…"; "Disconnect OpenCode…" deletes it.
import { spawn } from "node:child_process";

const EXE = __EXE__;
// A little longer than Cay holds a request; Cay always answers or hands back first.
const HOLD_MS = 700000;

function start(input, wait) {
  let child;
  try {
    child = spawn(EXE, ["hook", "--agent", "opencode"], {
      stdio: ["pipe", wait ? "pipe" : "ignore", "ignore"],
      windowsHide: true,
    });
  } catch {
    return { child: null, done: Promise.resolve("") };
  }
  const done = new Promise((resolve) => {
    let out = "";
    const timer = setTimeout(() => {
      try { child.kill(); } catch {}
    }, wait ? HOLD_MS : 15000);
    const finish = () => {
      clearTimeout(timer);
      resolve(out);
    };
    child.on("error", finish);
    child.on("close", finish);
    if (wait) child.stdout.on("data", (d) => { out += d; });
  });
  child.stdin.on("error", () => {});
  try { child.stdin.end(JSON.stringify(input)); } catch {}
  return { child, done };
}

export const Cay = async ({ client, directory }) => {
  const calls = new Map();
  const held = new Map();
  const base = (sessionID, event) => ({ session_id: sessionID, cwd: directory, hook_event_name: event });
  const send = (sessionID, event, extra) => {
    if (typeof sessionID === "string" && sessionID) start({ ...base(sessionID, event), ...extra }, false);
  };

  const ask = async (p) => {
    const call = p.tool && p.tool.callID ? calls.get(p.tool.callID) : undefined;
    const input = {
      ...base(p.sessionID, "PermissionRequest"),
      tool_name: call ? call.tool : p.permission,
      tool_input: (call && call.args) || p.metadata || {},
    };
    const { child, done } = start(input, true);
    if (child) held.set(p.id, child);
    const out = await done;
    held.delete(p.id);
    let behavior;
    try { behavior = JSON.parse(out).hookSpecificOutput.decision.behavior; } catch {}
    if (behavior !== "allow" && behavior !== "deny") return;
    try {
      await client.postSessionIdPermissionsPermissionId({
        path: { id: p.sessionID, permissionID: p.id },
        body: { response: behavior === "allow" ? "once" : "reject" },
      });
    } catch {}
  };

  return {
    "chat.message": async (input) => send(input.sessionID, "UserPromptSubmit"),
    "tool.execute.before": async (input, output) => {
      calls.set(input.callID, { tool: input.tool, args: output.args });
      if (calls.size > 256) calls.delete(calls.keys().next().value);
      send(input.sessionID, "PreToolUse", { tool_name: input.tool, tool_input: output.args || {} });
    },
    "tool.execute.after": async (input) => {
      calls.delete(input.callID);
      send(input.sessionID, "PostToolUse", { tool_name: input.tool });
    },
    event: async ({ event }) => {
      const p = event.properties || {};
      switch (event.type) {
        case "session.created":
          send(p.sessionID || (p.info && p.info.id), "SessionStart", { source: "startup" });
          break;
        case "session.idle":
          send(p.sessionID, "Stop");
          break;
        case "session.error":
          send(p.sessionID, p.error && p.error.name === "MessageAbortedError" ? "Interrupt" : "StopFailure");
          break;
        case "session.deleted":
          send(p.sessionID || (p.info && p.info.id), "SessionEnd");
          break;
        case "permission.asked":
          ask(p);
          break;
        case "permission.replied": {
          const child = held.get(p.requestID);
          if (child) {
            held.delete(p.requestID);
            try { child.kill(); } catch {}
          }
          break;
        }
      }
    },
  };
};
"##;

/// The plugin for this exe. The path goes in as a JSON string, so nothing in it needs quoting by hand.
pub fn source(exe: &Path) -> String {
    let exe = serde_json::to_string(&exe.to_string_lossy()).unwrap_or_else(|_| "\"\"".into());
    TEMPLATE
        .replace("__VERSION__", &VERSION.to_string())
        .replace("__EXE__", &exe)
}

/// Written by Cay (any version, any exe).
pub fn is_ours(text: &str) -> bool {
    text.lines()
        .take(3)
        .any(|l| l.starts_with("//") && l.contains(MARKER))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Connected,
    /// Cay's plugin from another exe or version: Connect replaces it.
    OutOfDate,
    NotConnected,
    /// A `cay.js` that isn't Cay's is in the way; Cay never touches it.
    Taken,
}

/// Where the plugin stands, given the file's text (`None` when it doesn't exist).
pub fn state(existing: Option<&str>, exe: &Path) -> State {
    match existing {
        None => State::NotConnected,
        Some(text) if text == source(exe) => State::Connected,
        Some(text) if is_ours(text) => State::OutOfDate,
        Some(_) => State::Taken,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = r#"C:\Users\DELL 7300\AppData\Local\Cay\cay.exe"#;

    #[test]
    fn the_exe_is_a_json_string() {
        let s = source(Path::new(EXE));
        assert!(s.contains(r#"const EXE = "C:\\Users\\DELL 7300\\AppData\\Local\\Cay\\cay.exe";"#));
        assert!(s.contains("cay-plugin: 1."));
        assert!(!s.contains("__"));
        let odd = source(Path::new(r#"C:\a "b"\cay.exe"#));
        assert!(odd.contains(r#"const EXE = "C:\\a \"b\"\\cay.exe";"#));
    }

    #[test]
    fn states() {
        let exe = Path::new(EXE);
        assert_eq!(state(None, exe), State::NotConnected);
        assert_eq!(state(Some(&source(exe)), exe), State::Connected);
        let dev = source(Path::new(r"C:\src\target\debug\cay.exe"));
        assert_eq!(state(Some(&dev), exe), State::OutOfDate);
        assert_eq!(
            state(Some("export const Mine = async () => ({});"), exe),
            State::Taken
        );
        assert!(is_ours(&dev));
    }
}
