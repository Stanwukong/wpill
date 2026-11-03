//! Adds Cay's hooks to Codex's `hooks.json` (`$CODEX_HOME` or `~/.codex`) and removes them again (2.9a, D47).
//!
//! Merge, never replace, as for Claude Code: other hooks and keys stay, ours are recognised by their command
//! (`…cay.exe hook --agent codex`), and the file is backed up before every write. Codex skips a new or changed hook
//! until you trust it once with `/hooks`, so Connect says so.
//!
//! Codex runs a hook's command through Windows PowerShell (0.166; cmd in older versions), and a quoted path runs
//! nothing there. So the command is the exe's 8.3 short path, unquoted, which both shells run; on a volume without
//! 8.3 names it falls back to PowerShell's `& '…'`, which works from 0.166 on (the minimum, `MIN_VERSION`).

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

/// The oldest Codex checked with these entries (2026-11-03).
pub const MIN_VERSION: (u32, u32, u32) = (0, 166, 0);
const VERB: &str = "hook --agent codex";

/// How Codex runs one of our hooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Run {
    /// In the background (`async`); Codex doesn't wait.
    Background,
    /// Events whose timeout Codex caps at 3 s (`SessionEnd`, `Interrupt`); the forwarder finishes well within it.
    Short,
    /// Codex waits for your answer in the island: longer than the forwarder's own bounds, as for Claude Code.
    Hold,
}

impl Run {
    fn timeout_secs(self) -> u64 {
        match self {
            Run::Background => 5,
            Run::Short => 3,
            Run::Hold => 660,
        }
    }
}

/// The hooks we register, and how each runs. `PermissionRequest` only fires in the TUI (`codex exec` can't ask).
pub const EVENTS: [(&str, Run); 8] = [
    ("SessionStart", Run::Background),
    ("UserPromptSubmit", Run::Background),
    ("PreToolUse", Run::Background),
    ("PostToolUse", Run::Background),
    ("Stop", Run::Background),
    ("Interrupt", Run::Short),
    ("SessionEnd", Run::Short),
    ("PermissionRequest", Run::Hold),
];

/// The hook command: `short` (the exe's 8.3 path) when it has no spaces, else `& '<exe>'` for PowerShell.
pub fn command(exe: &Path, short: Option<&Path>) -> String {
    match short.map(|s| s.to_string_lossy()) {
        Some(short) if !short.contains(' ') && !short.contains('\'') => format!("{short} {VERB}"),
        _ => format!("& '{}' {VERB}", exe.to_string_lossy().replace('\'', "''")),
    }
}

/// Whether a handler runs Cay's forwarder for Codex: its command ends in `…\cay.exe hook --agent codex` (or the
/// `& '…\cay.exe'` form), case-insensitive; `decay.exe` doesn't count.
fn is_ours(handler: &Value) -> bool {
    ["command", "commandWindows"].iter().any(|key| {
        handler
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_ascii_lowercase)
            .and_then(|c| c.strip_suffix(VERB).map(|rest| rest.trim_end().to_string()))
            .is_some_and(|rest| {
                rest.trim_end_matches('\'')
                    .strip_suffix("cay.exe")
                    .is_some_and(|dir| dir.ends_with(['\\', '/']))
            })
    })
}

/// One matcher group (no matcher: every tool) holding our handler. `commandWindows` repeats `command`, so a newer
/// Codex that prefers it runs the same thing.
pub fn group(command: &str, run: Run) -> Value {
    let mut handler = Map::new();
    handler.insert("type".into(), json!("command"));
    handler.insert("command".into(), json!(command));
    handler.insert("commandWindows".into(), json!(command));
    handler.insert("timeout".into(), json!(run.timeout_secs()));
    if matches!(run, Run::Background) {
        handler.insert("async".into(), json!(true));
    }
    json!({ "hooks": [Value::Object(handler)] })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Connected,
    /// Ours, but not as this exe writes them (another path, or an older shape): Connect updates them.
    OutOfDate,
    NotConnected,
}

pub fn state(file: &Value, command: &str) -> State {
    let groups = |event: &str| -> Vec<Value> {
        file.pointer(&format!("/hooks/{event}"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let current = EVENTS.iter().all(|(event, run)| {
        let want = group(command, *run)["hooks"][0].clone();
        let ours: Vec<Value> = groups(event)
            .iter()
            .flat_map(|g| g["hooks"].as_array().cloned().unwrap_or_default())
            .filter(is_ours)
            .collect();
        ours == [want]
    });
    if current {
        return State::Connected;
    }
    let any = file
        .get("hooks")
        .and_then(Value::as_object)
        .is_some_and(|events| {
            events
                .values()
                .filter_map(Value::as_array)
                .flatten()
                .any(|g| {
                    g["hooks"]
                        .as_array()
                        .is_some_and(|hs| hs.iter().any(is_ours))
                })
        });
    if any {
        State::OutOfDate
    } else {
        State::NotConnected
    }
}

/// The file with exactly one of our groups per event, as this `command` writes it.
pub fn with_hooks(file: Value, command: &str) -> Result<Value, String> {
    let mut file = without_hooks(file)?;
    let root = file.as_object_mut().ok_or("hooks.json isn't an object")?;
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("hooks.json's \"hooks\" isn't an object")?;
    for (event, run) in EVENTS {
        let groups = hooks
            .entry(event)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or_else(|| format!("hooks.json's \"{event}\" isn't a list"))?;
        groups.push(group(command, run));
    }
    Ok(file)
}

/// The file without any of our handlers. A group left empty by that goes, and so does an event left without groups;
/// anything that wasn't ours stays as it was.
pub fn without_hooks(mut file: Value) -> Result<Value, String> {
    let root = file.as_object_mut().ok_or("hooks.json isn't an object")?;
    let Some(hooks) = root.get_mut("hooks") else {
        return Ok(file);
    };
    let hooks = hooks
        .as_object_mut()
        .ok_or("hooks.json's \"hooks\" isn't an object")?;
    for groups in hooks.values_mut() {
        let Some(groups) = groups.as_array_mut() else {
            continue;
        };
        groups.retain_mut(|g| {
            let Some(handlers) = g.get_mut("hooks").and_then(Value::as_array_mut) else {
                return true;
            };
            let before = handlers.len();
            handlers.retain(|h| !is_ours(h));
            !(handlers.is_empty() && before > 0)
        });
    }
    hooks.retain(|_, groups| groups.as_array().is_none_or(|g| !g.is_empty()));
    Ok(file)
}

/// What Connect adds, for its dialog.
pub fn preview(command: &str) -> String {
    let pretty = |v: &Value| serde_json::to_string_pretty(v).unwrap_or_default();
    [Run::Background, Run::Short, Run::Hold]
        .into_iter()
        .map(|run| {
            let events: Vec<&str> = EVENTS.iter().filter(|e| e.1 == run).map(|e| e.0).collect();
            format!("{}:\n{}", events.join(", "), pretty(&group(command, run)))
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// `$CODEX_HOME\hooks.json`, else `%USERPROFILE%\.codex\hooks.json`.
pub fn path() -> Option<PathBuf> {
    let dir = std::env::var_os("CODEX_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|home| Path::new(&home).join(".codex")))?;
    Some(dir.join("hooks.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = r"C:\Users\DELL 7300\AppData\Local\Cay\cay.exe";
    const SHORT: &str = r"C:\Users\DELL73~1\AppData\Local\Cay\cay.exe";

    fn cmd() -> String {
        command(Path::new(EXE), Some(Path::new(SHORT)))
    }

    fn handlers(file: &Value, event: &str) -> Vec<Value> {
        file["hooks"][event].as_array().map_or(vec![], |groups| {
            groups
                .iter()
                .flat_map(|g| g["hooks"].as_array().cloned().unwrap_or_default())
                .collect()
        })
    }

    #[test]
    fn the_command_avoids_quotes_and_spaces() {
        assert_eq!(cmd(), format!("{SHORT} hook --agent codex"));
        // No 8.3 name (or one with a space): PowerShell's call operator, quotes doubled.
        assert_eq!(
            command(Path::new(r"C:\Bob's Apps\cay.exe"), None),
            r"& 'C:\Bob''s Apps\cay.exe' hook --agent codex"
        );
        assert_eq!(
            command(Path::new(EXE), Some(Path::new(EXE))),
            format!("& '{EXE}' hook --agent codex")
        );
    }

    #[test]
    fn recognises_only_cays_handlers() {
        let ours = |c: &str| is_ours(&json!({ "type": "command", "command": c }));
        assert!(ours(&cmd()));
        assert!(ours(&command(Path::new(EXE), None)));
        assert!(ours(r"C:\OTHER\CAY.EXE HOOK --AGENT CODEX"));
        assert!(!ours(r"C:\x\decay.exe hook --agent codex"));
        assert!(!ours(r"C:\x\cay.exe hook"));
        assert!(!ours("python audit.py"));
        assert!(is_ours(
            &json!({ "command": "x", "commandWindows": format!("{SHORT} hook --agent codex") })
        ));
    }

    #[test]
    fn connects_into_an_empty_or_missing_file() {
        let file = with_hooks(json!({}), &cmd()).unwrap();
        assert_eq!(state(&file, &cmd()), State::Connected);
        for (event, run) in EVENTS {
            assert_eq!(
                handlers(&file, event),
                vec![group(&cmd(), run)["hooks"][0].clone()]
            );
        }
        let hold = &handlers(&file, "PermissionRequest")[0];
        assert_eq!(
            (hold["timeout"].as_u64(), hold.get("async")),
            (Some(660), None)
        );
        let bg = &handlers(&file, "Stop")[0];
        assert_eq!(bg["async"], json!(true));
        assert_eq!(handlers(&file, "SessionEnd")[0]["timeout"], json!(3));
        assert_eq!(state(&json!({}), &cmd()), State::NotConnected);
    }

    #[test]
    fn keeps_everything_else_and_never_doubles() {
        let theirs = json!({
            "description": "team policy",
            "hooks": {
                "PreToolUse": [{ "matcher": "Bash", "hooks": [{ "type": "command", "command": "py policy.py" }] }],
                "Stop": [{ "hooks": [
                    { "type": "command", "command": "notify.exe" },
                    { "type": "command", "command": format!("{SHORT} hook --agent codex") }
                ] }]
            }
        });
        let once = with_hooks(theirs.clone(), &cmd()).unwrap();
        let twice = with_hooks(once.clone(), &cmd()).unwrap();
        assert_eq!(once, twice);
        assert_eq!(once["description"], json!("team policy"));
        // Theirs first, in place; ours once per event.
        assert_eq!(
            handlers(&once, "PreToolUse")[0]["command"],
            json!("py policy.py")
        );
        assert_eq!(
            handlers(&once, "Stop")
                .iter()
                .filter(|h| is_ours(h))
                .count(),
            1
        );
        assert_eq!(handlers(&once, "Stop")[0]["command"], json!("notify.exe"));

        let removed = without_hooks(once).unwrap();
        assert_eq!(
            removed,
            json!({
                "description": "team policy",
                "hooks": {
                    "PreToolUse": [{ "matcher": "Bash", "hooks": [{ "type": "command", "command": "py policy.py" }] }],
                    "Stop": [{ "hooks": [{ "type": "command", "command": "notify.exe" }] }]
                }
            })
        );
    }

    #[test]
    fn another_exe_or_shape_is_out_of_date() {
        let dev = command(Path::new(r"C:\src\target\debug\cay.exe"), None);
        let file = with_hooks(json!({}), &dev).unwrap();
        assert_eq!(state(&file, &cmd()), State::OutOfDate);
        let mut missing = with_hooks(json!({}), &cmd()).unwrap();
        missing["hooks"]
            .as_object_mut()
            .unwrap()
            .remove("Interrupt");
        assert_eq!(state(&missing, &cmd()), State::OutOfDate);
        let updated = with_hooks(file, &cmd()).unwrap();
        assert_eq!(state(&updated, &cmd()), State::Connected);
    }

    #[test]
    fn refuses_files_it_cant_merge() {
        assert!(with_hooks(json!([]), &cmd()).is_err());
        assert!(with_hooks(json!({ "hooks": [] }), &cmd()).is_err());
        assert!(with_hooks(json!({ "hooks": { "Stop": {} } }), &cmd()).is_err());
        assert!(without_hooks(json!({ "hooks": 3 })).is_err());
    }
}
