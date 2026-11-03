//! Usage limits through Claude Code's status line (Phase 2.8f, D41).
//!
//! Connect points `statusLine.command` at `cay.exe statusline` and keeps the user's own `statusLine` in Cay's
//! `settings.json` (`claude.statusLine`, with the shell it runs in). On each status update, `cay.exe statusline`
//! sends only the `rate_limits` to the hook pipe (bounded, quiet), then runs the user's command with the same input
//! and prints its output unchanged. With no command of theirs it prints nothing. Like `cay.exe hook`, it never
//! panics, never waits unbounded, and prints nothing of its own.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use super::hooks::{self, Shell};
use crate::ipc::UsageLimits;
use crate::sessions::{files, forward, pipe, process, protocol};

/// The user's command gets this long; Claude Code cancels a slow status line on the next update anyway.
const RUN_TIMEOUT: Duration = Duration::from_secs(10);
/// Most of the user's output that's passed on.
const MAX_OUTPUT: usize = 64 * 1024;
/// `CREATE_NO_WINDOW`: the user's command must not flash a console (the release exe has none to share).
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// What Connect kept of the user's own status line.
#[derive(Debug, Clone, PartialEq)]
pub struct Saved {
    /// Their `statusLine` object as it was (`None`: they had none).
    pub status_line: Option<Value>,
    pub shell: Shell,
}

/// `cay.exe statusline`: `main.rs` calls this before anything else starts.
pub fn run() {
    let at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    let Some(stdin) = forward::read_stdin() else {
        return;
    };
    if let Some(envelope) = protocol::limits_envelope(&stdin, at_ms, process::ancestors()) {
        if let (Ok(bytes), Some(name)) = (serde_json::to_vec(&envelope), pipe::name()) {
            if bytes.len() <= protocol::MAX_MESSAGE {
                let _ = forward::send(&name, &bytes);
            }
        }
    }
    let Some(saved) = saved() else {
        return;
    };
    let Some(command) = saved
        .status_line
        .as_ref()
        .and_then(|s| s.get("command"))
        .and_then(Value::as_str)
    else {
        return;
    };
    if let Some(output) = run_theirs(command, saved.shell, &stdin) {
        let mut stdout = std::io::stdout().lock();
        let _ = stdout.write_all(&output);
        let _ = stdout.flush();
    }
}

/// The user's command, run the way Claude Code runs it, with `input` on stdin; its stdout, or `None` on any failure
/// or after `RUN_TIMEOUT` (the process is then killed).
fn run_theirs(command: &str, shell: Shell, input: &[u8]) -> Option<Vec<u8>> {
    use std::os::windows::process::CommandExt;
    let mut cmd = match shell {
        Shell::Bash => {
            let mut c = Command::new(hooks::bash_path()?);
            c.args(["-c", command]);
            c
        }
        Shell::PowerShell => {
            let mut c = Command::new("powershell.exe");
            c.args(["-NoProfile", "-NonInteractive", "-Command", command]);
            c
        }
    };
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take()?;
    let input = input.to_vec();
    std::thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = (&mut stdout).take(MAX_OUTPUT as u64).read_to_end(&mut out);
        let _ = tx.send(out);
    });
    let deadline = Instant::now() + RUN_TIMEOUT;
    let output = rx.recv_timeout(RUN_TIMEOUT).ok();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                return None;
            }
        }
    }
    output
}

/// What Connect kept, or `None` if Cay hasn't chained a status line.
pub fn saved() -> Option<Saved> {
    from_block(&files::cay_block("claude")?)
}

fn from_block(block: &Value) -> Option<Saved> {
    let shell = match block.get("statusShell")?.as_str()? {
        "powershell" => Shell::PowerShell,
        _ => Shell::Bash,
    };
    let status_line = block.get("statusLine").filter(|v| !v.is_null()).cloned();
    Some(Saved { status_line, shell })
}

/// Keeps (or with `None`, forgets) what Connect replaced, in Cay's `settings.json`; other keys stay.
pub fn keep(saved: Option<&Saved>) -> Result<(), String> {
    files::set_cay_block("claude", saved.map(to_block))
}

fn to_block(s: &Saved) -> Value {
    json!({
        "statusLine": s.status_line,
        "statusShell": match s.shell { Shell::Bash => "bash", Shell::PowerShell => "powershell" },
    })
}

/// Marks windows whose reset time has passed (Claude Code drops them then, but a session may not update for a
/// while). Returns whether anything changed.
pub fn mark_stale(limits: &mut UsageLimits, now_ms: i64) -> bool {
    let mut changed = false;
    for w in [&mut limits.five_hour, &mut limits.seven_day]
        .into_iter()
        .flatten()
    {
        let stale = now_ms >= w.resets_at_ms;
        changed |= stale != w.stale;
        w.stale = stale;
    }
    changed
}

/// The newer of two readings.
pub fn newest(current: Option<UsageLimits>, reading: UsageLimits) -> UsageLimits {
    match current {
        Some(c) if c.at_ms > reading.at_ms => c,
        _ => reading,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::LimitWindow;

    #[test]
    fn the_saved_block_round_trips() {
        let theirs = json!({ "type": "command", "command": "node hud.mjs", "padding": 0 });
        let saved = Saved {
            status_line: Some(theirs.clone()),
            shell: Shell::Bash,
        };
        assert_eq!(from_block(&to_block(&saved)), Some(saved));
        let none = Saved {
            status_line: None,
            shell: Shell::PowerShell,
        };
        assert_eq!(from_block(&to_block(&none)), Some(none));
        assert_eq!(from_block(&json!({})), None);
    }

    #[test]
    fn stale_after_reset() {
        let w = |resets_at_ms| LimitWindow {
            used_percent: 50.0,
            resets_at_ms,
            stale: false,
        };
        let mut l = UsageLimits {
            five_hour: Some(w(1_000)),
            seven_day: Some(w(5_000)),
            at_ms: 500,
        };
        assert!(!mark_stale(&mut l, 900));
        assert!(mark_stale(&mut l, 1_000));
        assert!(l.five_hour.unwrap().stale && !l.seven_day.unwrap().stale);
        assert!(!mark_stale(&mut l, 1_100), "already marked");
        let newer = UsageLimits { at_ms: 600, ..l };
        assert_eq!(
            newest(Some(newer), l).at_ms,
            600,
            "an older reading arriving late loses"
        );
        assert_eq!(newest(Some(l), newer).at_ms, 600);
    }

    #[test]
    fn runs_their_command_with_the_same_input() {
        let Some(_) = hooks::bash_path() else {
            return; // No Git Bash on this machine.
        };
        let out = run_theirs("cat", Shell::Bash, b"{\"x\":1}").unwrap();
        assert_eq!(out, b"{\"x\":1}");
        let out = run_theirs("echo hi; exit 3", Shell::Bash, b"").unwrap();
        assert_eq!(out, b"hi\n", "output is passed on whatever the exit code");
    }
}
