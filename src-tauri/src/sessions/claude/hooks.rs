//! Adds Cay's hooks to Claude Code's user settings (`~/.claude/settings.json`) and removes them again.
//!
//! Merge, never replace: other hooks and every other key stay as they are (key order included, via
//! serde_json's `preserve_order`). Ours are recognised by their command (`…cay.exe' hook`), so installing
//! twice is a no-op and removing touches nothing else. The file is backed up before every write, and a
//! file we can't parse is left alone.
//!
//! The entry is shell form with an explicit `shell`: Claude Code before 2.1.15x has no `args` exec form and
//! would run a bare `cay.exe`, starting a second Cay instead of the forwarder.
//!
//! The status line (2.8f, D41) is chained, never replaced: `statusLine.command` becomes `cay.exe statusline`, which
//! runs the user's own command after reading the usage limits. Their original `statusLine` is kept by the caller
//! (Cay's `settings.json`, `claude.statusLine`) and put back exactly, in place, on Disconnect.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};

/// How Claude Code runs one of our hooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Run {
    /// In the background (`async`); Claude Code doesn't wait.
    Background,
    /// Claude Code waits, briefly.
    Wait,
    /// Claude Code waits for your answer in the island (2.8a): longer than the forwarder's own bounds
    /// (`forward::answer_timeout_ms`, up to ~10 minutes for a question or a plan, 2.8c), so the forwarder always ends
    /// first and never shows as a hook error. The terminal prompts meanwhile, so a long hold doesn't block it.
    Hold,
}

impl Run {
    fn timeout_secs(self) -> u64 {
        match self {
            Run::Background | Run::Wait => 5,
            Run::Hold => 660,
        }
    }
}

/// The hooks we register, and how each runs.
pub const EVENTS: [(&str, Run); 9] = [
    ("SessionStart", Run::Background),
    ("UserPromptSubmit", Run::Background),
    // The live step line (2.8d): in the background, so it never slows a tool or decides anything.
    ("PreToolUse", Run::Background),
    ("PostToolUse", Run::Background),
    ("Notification", Run::Background),
    ("Stop", Run::Background),
    ("StopFailure", Run::Background),
    // At exit, in case Claude Code doesn't wait for background hooks.
    ("SessionEnd", Run::Wait),
    ("PermissionRequest", Run::Hold),
];
const MARKER: &str = "cay.exe' hook";
/// Cay's status line command (2.8f).
const STATUS_MARKER: &str = "cay.exe' statusline";
/// Cay was called TopBar until Phase 4a-3 (D32). Its entries are still ours to replace and remove, but they don't
/// count as connected: they point at the old exe and pipe, so the tray offers Connect again.
const OLD_MARKER: &str = "topbar.exe' hook";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    /// Git Bash, Claude Code's default on Windows when it's installed.
    Bash,
    PowerShell,
}

/// The hook command for `exe`, quoted for `shell` (paths with spaces must survive).
pub fn command(exe: &Path, shell: Shell) -> String {
    command_for(exe, shell, "hook")
}

/// The status line command for `exe` (2.8f).
pub fn status_command(exe: &Path, shell: Shell) -> String {
    command_for(exe, shell, "statusline")
}

fn command_for(exe: &Path, shell: Shell, verb: &str) -> String {
    let path = exe.to_string_lossy();
    match shell {
        // Forward slashes: Git Bash doesn't treat them as escapes. `'` is closed, escaped and reopened.
        Shell::Bash => format!(
            "'{}' {verb}",
            path.replace('\\', "/").replace('\'', r"'\''")
        ),
        Shell::PowerShell => format!("& '{}' {verb}", path.replace('\'', "''")),
    }
}

/// One matcher group (no matcher: every occurrence) holding our hook.
pub fn group(command: &str, shell: Shell, run: Run) -> Value {
    let mut hook = Map::new();
    hook.insert("type".into(), json!("command"));
    hook.insert("command".into(), json!(command));
    hook.insert(
        "shell".into(),
        json!(match shell {
            Shell::Bash => "bash",
            Shell::PowerShell => "powershell",
        }),
    );
    if run == Run::Background {
        hook.insert("async".into(), json!(true));
    }
    hook.insert("timeout".into(), json!(run.timeout_secs()));
    json!({ "hooks": [Value::Object(hook)] })
}

/// The hook's command runs `…\<exe>' hook` (or `…/<exe>' hook`) for `marker`'s exe; `decay.exe` doesn't count.
fn runs(hook: &Value, marker: &str) -> bool {
    hook.get("command")
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase)
        .is_some_and(|c| {
            c.strip_suffix(marker)
                .is_some_and(|rest| rest.ends_with(['/', '\\']))
        })
}

/// Ours to replace or remove: Cay's entries and TopBar's from before the rename.
fn is_ours(hook: &Value) -> bool {
    runs(hook, MARKER) || runs(hook, OLD_MARKER)
}

/// Where Cay stands in a settings file: connected as this version writes it, connected by an earlier version (or
/// TopBar) and needing an update, or not at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Connected,
    OutOfDate,
    NotConnected,
}

pub fn state(settings: &Value) -> State {
    let status_done = has_our_status_line(settings) || !chainable(settings.get("statusLine"));
    if installed(settings) && status_done {
        return State::Connected;
    }
    let any_hook = settings
        .get("hooks")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|events| events.values())
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|g| g.get("hooks").and_then(Value::as_array))
        .flatten()
        .any(is_ours);
    if any_hook || has_our_status_line(settings) {
        State::OutOfDate
    } else {
        State::NotConnected
    }
}

/// `statusLine` runs Cay's status line command.
pub fn has_our_status_line(settings: &Value) -> bool {
    settings
        .get("statusLine")
        .is_some_and(|s| runs(s, STATUS_MARKER))
}

/// A status line Cay can chain: none at all, or a `command` one with a command string. Anything else is left alone
/// (and Cay shows no usage limits).
fn chainable(status: Option<&Value>) -> bool {
    match status {
        None => true,
        Some(s) => {
            s.get("type").and_then(Value::as_str) == Some("command")
                && s.get("command").and_then(Value::as_str).is_some()
        }
    }
}

/// `settings` with Cay's status line chained in front of the user's (2.8f). Returns the new settings and the
/// user's original `statusLine` for the caller to keep (`None` if there was none). `saved` is what the caller kept
/// from an earlier Connect; it's put back first, so connecting twice never chains Cay to itself. The new entry keeps
/// the original's other fields (`padding`, `refreshInterval`) and its place in the file.
pub fn with_status_line(
    settings: Value,
    status_command: &str,
    saved: Option<&Value>,
) -> Result<(Value, Option<Value>), String> {
    let mut settings = without_status_line(settings, saved)?;
    let root = settings
        .as_object_mut()
        .ok_or("settings.json isn't a JSON object")?;
    let original = root.get("statusLine").cloned();
    if !chainable(original.as_ref()) {
        return Ok((settings, original));
    }
    match root.get_mut("statusLine").and_then(Value::as_object_mut) {
        Some(ours) => {
            ours.insert("command".into(), json!(status_command));
        }
        None => {
            root.insert(
                "statusLine".into(),
                json!({ "type": "command", "command": status_command }),
            );
        }
    }
    Ok((settings, original))
}

/// `settings` with the user's own status line back (`saved`, in place) if Cay's is there; without one if they had
/// none. A status line that isn't Cay's is never touched.
pub fn without_status_line(mut settings: Value, saved: Option<&Value>) -> Result<Value, String> {
    if !has_our_status_line(&settings) {
        return Ok(settings);
    }
    let root = settings
        .as_object_mut()
        .ok_or("settings.json isn't a JSON object")?;
    match saved {
        Some(original) => {
            if let Some(slot) = root.get_mut("statusLine") {
                *slot = original.clone();
            }
        }
        None => {
            root.remove("statusLine");
        }
    }
    Ok(settings)
}

/// Connected: every event we register has an entry that runs Cay, as this version writes it (timeout, background).
/// Old TopBar entries don't count, and nor does an install from before an event or a timeout changed (2.8a's
/// `PermissionRequest`, 2.8c's longer hold), so the tray offers Connect to bring it up to date.
pub fn installed(settings: &Value) -> bool {
    let Some(events) = settings.get("hooks").and_then(Value::as_object) else {
        return false;
    };
    EVENTS.iter().all(|(event, run)| {
        events
            .get(*event)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|g| g.get("hooks").and_then(Value::as_array))
            .flatten()
            .any(|h| {
                runs(h, MARKER)
                    && h.get("timeout").and_then(Value::as_u64) == Some(run.timeout_secs())
                    && h.get("async").and_then(Value::as_bool).unwrap_or(false)
                        == (*run == Run::Background)
            })
    })
}

/// `settings` with any previous Cay hooks replaced by fresh ones.
pub fn with_hooks(settings: Value, command: &str, shell: Shell) -> Result<Value, String> {
    let mut settings = without_hooks(settings)?;
    let root = settings
        .as_object_mut()
        .ok_or("settings.json isn't a JSON object")?;
    let events = root
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or("\"hooks\" isn't an object")?;
    for (event, run) in EVENTS {
        events
            .entry(event)
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or_else(|| format!("\"hooks.{event}\" isn't a list"))?
            .push(group(command, shell, run));
    }
    Ok(settings)
}

/// `settings` without Cay's hooks. Groups, events and `hooks` that only held ours go too.
pub fn without_hooks(mut settings: Value) -> Result<Value, String> {
    let root = settings
        .as_object_mut()
        .ok_or("settings.json isn't a JSON object")?;
    let Some(hooks) = root.get_mut("hooks") else {
        return Ok(settings);
    };
    let events = hooks.as_object_mut().ok_or("\"hooks\" isn't an object")?;
    let mut emptied = Vec::new();
    for (event, groups) in events.iter_mut() {
        let Some(groups) = groups.as_array_mut() else {
            continue;
        };
        let had = groups.len();
        groups.retain_mut(|g| {
            let Some(list) = g.get_mut("hooks").and_then(Value::as_array_mut) else {
                return true;
            };
            let before = list.len();
            list.retain(|h| !is_ours(h));
            !(list.is_empty() && before > 0)
        });
        if had > 0 && groups.is_empty() {
            emptied.push(event.clone());
        }
    }
    for event in &emptied {
        events.remove(event);
    }
    if !emptied.is_empty() && events.is_empty() {
        root.remove("hooks");
    }
    Ok(settings)
}

/// What the confirmation dialog says about the status line: the entry written, and what happens to the user's own.
pub fn status_preview(settings: &Value, status_command: &str, saved: Option<&Value>) -> String {
    let pretty = |v: &Value| serde_json::to_string_pretty(v).unwrap_or_default();
    let Ok((after, original)) = with_status_line(settings.clone(), status_command, saved) else {
        return String::new();
    };
    if !has_our_status_line(&after) {
        return "statusLine: yours isn't a command, so it's left as it is (no usage limits in Cay).".into();
    }
    let what = match &original {
        Some(o) => format!(
            "Your status line still shows: Cay runs your command\n{}\nafter reading the usage limits, and puts it \
             back when you disconnect.",
            o.get("command").and_then(Value::as_str).unwrap_or_default()
        ),
        None => "You have no status line; Cay's prints nothing (Claude Code hides some footer hints while one \
                 is set). Disconnect removes it."
            .to_string(),
    };
    format!("statusLine:\n{}\n{what}", pretty(&after["statusLine"]))
}

/// What the confirmation dialog shows: exactly what gets added.
pub fn preview(command: &str, shell: Shell) -> String {
    let pretty = |v: &Value| serde_json::to_string_pretty(v).unwrap_or_default();
    [Run::Background, Run::Wait, Run::Hold]
        .into_iter()
        .map(|run| {
            let events: Vec<&str> = EVENTS.iter().filter(|e| e.1 == run).map(|e| e.0).collect();
            format!(
                "{}:\n{}",
                events.join(", "),
                pretty(&group(command, shell, run))
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// `%CLAUDE_CONFIG_DIR%\settings.json`, else `%USERPROFILE%\.claude\settings.json`.
pub fn settings_path() -> Option<PathBuf> {
    let dir = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|home| Path::new(&home).join(".claude")))?;
    Some(dir.join("settings.json"))
}

/// The shell Claude Code will run hooks and the status line in: Git Bash if it finds it (same lookup order), else
/// PowerShell.
pub fn detect_shell() -> Shell {
    if bash_path().is_some() {
        Shell::Bash
    } else {
        Shell::PowerShell
    }
}

/// Git Bash's `bash.exe`, found the way Claude Code finds it.
pub fn bash_path() -> Option<PathBuf> {
    let from_env = std::env::var_os("CLAUDE_CODE_GIT_BASH_PATH").map(PathBuf::from);
    let from_git = std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join("git.exe"))
            .find(|git| git.is_file())
            // …\Git\cmd\git.exe → …\Git\bin\bash.exe
            .and_then(|git| Some(git.parent()?.parent()?.join("bin").join("bash.exe")))
    });
    let default = std::env::var_os("ProgramFiles")
        .map(|pf| Path::new(&pf).join("Git").join("bin").join("bash.exe"));
    [from_env, from_git, default]
        .into_iter()
        .flatten()
        .find(|bash| bash.is_file())
}

/// The parsed file, `Ok(None)` if it doesn't exist yet.
pub fn read(path: &Path) -> Result<Option<Value>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => Ok(Some(json!({}))),
        Ok(text) => serde_json::from_str(&text).map(Some).map_err(|e| {
            format!(
                "{} isn't valid JSON ({e}); it was left unchanged",
                path.display()
            )
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("couldn't read {}: {e}", path.display())),
    }
}

/// Backs up the current file (if any) next to it, then replaces it atomically. Returns the backup path.
pub fn write(path: &Path, settings: &Value) -> Result<Option<PathBuf>, String> {
    let fail = |what: &str, e: std::io::Error| format!("couldn't {what} {}: {e}", path.display());
    let backup = if path.exists() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let backup = path.with_extension(format!("json.cay-{stamp}.bak"));
        std::fs::copy(path, &backup).map_err(|e| fail("back up", e))?;
        Some(backup)
    } else {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| fail("create the folder for", e))?;
        }
        None
    };
    let mut text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    text.push('\n');
    let temp = path.with_extension("json.cay-tmp");
    std::fs::write(&temp, text).map_err(|e| fail("write", e))?;
    std::fs::rename(&temp, path).map_err(|e| fail("replace", e))?;
    Ok(backup)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = r"C:\Users\DELL 7300\AppData\Local\Cay\cay.exe";

    fn cmd() -> String {
        command(Path::new(EXE), Shell::Bash)
    }

    fn ours_in(settings: &Value, event: &str) -> usize {
        settings["hooks"][event].as_array().map_or(0, |groups| {
            groups
                .iter()
                .flat_map(|g| g["hooks"].as_array().cloned().unwrap_or_default())
                .filter(is_ours)
                .count()
        })
    }

    #[test]
    fn commands_quote_the_path_for_each_shell() {
        assert_eq!(cmd(), "'C:/Users/DELL 7300/AppData/Local/Cay/cay.exe' hook");
        assert_eq!(
            command(Path::new(EXE), Shell::PowerShell),
            r"& 'C:\Users\DELL 7300\AppData\Local\Cay\cay.exe' hook"
        );
        assert_eq!(
            command(Path::new(r"C:\O'Neil\cay.exe"), Shell::Bash),
            r"'C:/O'\''Neil/cay.exe' hook"
        );
        assert_eq!(
            command(Path::new(r"C:\O'Neil\cay.exe"), Shell::PowerShell),
            r"& 'C:\O''Neil\cay.exe' hook"
        );
        assert!(is_ours(
            &json!({ "command": command(Path::new(EXE), Shell::PowerShell) })
        ));
    }

    #[test]
    fn install_merges_with_existing_hooks_and_keys() {
        let settings = json!({
            "model": "opus",
            "hooks": {
                "Stop": [{ "hooks": [{ "type": "command", "command": "say done" }] }],
                "PreToolUse": [{ "matcher": "Bash", "hooks": [{ "type": "command", "command": "lint" }] }]
            },
            "theme": "dark"
        });
        let merged = with_hooks(settings, &cmd(), Shell::Bash).unwrap();
        // Order and other keys survive.
        let keys: Vec<&String> = merged.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["model", "hooks", "theme"]);
        assert_eq!(
            merged["hooks"]["Stop"][0]["hooks"][0]["command"],
            "say done"
        );
        assert_eq!(merged["hooks"]["PreToolUse"][0]["matcher"], "Bash");
        for (event, run) in EVENTS {
            assert_eq!(ours_in(&merged, event), 1, "{event}");
            let ours = merged["hooks"][event].as_array().unwrap().last().unwrap();
            assert_eq!(ours["hooks"][0]["shell"], "bash");
            assert_eq!(ours["hooks"][0]["timeout"], run.timeout_secs());
            assert_eq!(
                ours["hooks"][0].get("async").is_some(),
                run == Run::Background
            );
            assert!(ours.get("matcher").is_none());
        }
        assert!(installed(&merged));
    }

    #[test]
    fn an_install_without_permission_requests_is_out_of_date() {
        let mut settings = with_hooks(json!({}), &cmd(), Shell::Bash).unwrap();
        assert_eq!(
            settings["hooks"]["PermissionRequest"][0]["hooks"][0]["timeout"],
            660
        );
        let mut short = settings.clone();
        short["hooks"]["PermissionRequest"][0]["hooks"][0]["timeout"] = json!(75);
        assert!(!installed(&short), "2.8a's 75 s hold is out of date");
        settings["hooks"]
            .as_object_mut()
            .unwrap()
            .remove("PermissionRequest");
        assert!(!installed(&settings));
        assert!(installed(
            &with_hooks(settings, &cmd(), Shell::Bash).unwrap()
        ));
    }

    #[test]
    fn installing_twice_keeps_one_copy_and_updates_the_path() {
        let once = with_hooks(json!({}), &cmd(), Shell::Bash).unwrap();
        let moved = command(Path::new(r"D:\Apps\cay.exe"), Shell::Bash);
        let twice = with_hooks(once, &moved, Shell::Bash).unwrap();
        assert_eq!(ours_in(&twice, "Stop"), 1);
        assert_eq!(twice["hooks"]["Stop"][0]["hooks"][0]["command"], moved);
    }

    fn status() -> String {
        status_command(Path::new(EXE), Shell::Bash)
    }

    #[test]
    fn three_states() {
        assert_eq!(state(&json!({})), State::NotConnected);
        let (s, _) = with_status_line(
            with_hooks(json!({}), &cmd(), Shell::Bash).unwrap(),
            &status(),
            None,
        )
        .unwrap();
        assert_eq!(state(&s), State::Connected);
        let mut old = s.clone();
        old["hooks"]["PermissionRequest"][0]["hooks"][0]["timeout"] = json!(75);
        assert_eq!(state(&old), State::OutOfDate, "an earlier version's entry");
        let hooks_only = with_hooks(json!({}), &cmd(), Shell::Bash).unwrap();
        assert_eq!(
            state(&hooks_only),
            State::OutOfDate,
            "from before the status line"
        );
        let topbar = json!({ "hooks": { "Stop": [{ "hooks": [{ "command": "'C:/x/topbar.exe' hook" }] }] } });
        assert_eq!(state(&topbar), State::OutOfDate);
        let mut other = hooks_only.clone();
        other["statusLine"] = json!({ "type": "static", "text": "hi" });
        assert_eq!(
            state(&other),
            State::Connected,
            "an unchainable status line is left alone"
        );
    }

    #[test]
    fn status_line_is_chained_and_restored_in_place() {
        let theirs = json!({ "type": "command", "command": "node hud.mjs", "padding": 0 });
        let settings = json!({ "model": "opus", "statusLine": theirs, "theme": "dark" });
        let (chained, original) = with_status_line(settings.clone(), &status(), None).unwrap();
        assert_eq!(original.as_ref(), Some(&theirs));
        assert_eq!(chained["statusLine"]["command"], status());
        assert_eq!(
            chained["statusLine"]["padding"], 0,
            "their other fields stay"
        );
        let keys: Vec<&String> = chained.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["model", "statusLine", "theme"]);
        assert!(has_our_status_line(&chained));

        // Connecting again chains to the saved original, never to Cay itself.
        let (again, original2) =
            with_status_line(chained.clone(), &status(), original.as_ref()).unwrap();
        assert_eq!(again, chained);
        assert_eq!(original2.as_ref(), Some(&theirs));

        let restored = without_status_line(chained, original.as_ref()).unwrap();
        assert_eq!(restored, settings);
        let keys: Vec<&String> = restored.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["model", "statusLine", "theme"], "back in its place");

        // None before: one is added, and removed again.
        let (added, none) = with_status_line(json!({ "a": 1 }), &status(), None).unwrap();
        assert_eq!(none, None);
        assert_eq!(
            added["statusLine"],
            json!({ "type": "command", "command": status() })
        );
        assert_eq!(without_status_line(added, None).unwrap(), json!({ "a": 1 }));

        // Someone else's status line is never touched by Disconnect.
        let mine = json!({ "statusLine": theirs });
        assert_eq!(without_status_line(mine.clone(), None).unwrap(), mine);
        let odd = json!({ "statusLine": { "type": "static" } });
        assert_eq!(
            with_status_line(odd.clone(), &status(), None).unwrap().0,
            odd
        );
    }

    #[test]
    fn previews_match_what_is_written() {
        let merged = with_hooks(json!({}), &cmd(), Shell::Bash).unwrap();
        let text = preview(&cmd(), Shell::Bash);
        for (event, run) in EVENTS {
            let written = merged["hooks"][event][0].clone();
            assert_eq!(written, group(&cmd(), Shell::Bash, run), "{event}");
            assert!(text.contains(event), "{event} is named");
            assert!(
                text.contains(&serde_json::to_string_pretty(&written).unwrap()),
                "{event}'s entry is shown as written"
            );
        }
        let theirs = json!({ "statusLine": { "type": "command", "command": "node hud.mjs" } });
        let shown = status_preview(&theirs, &status(), None);
        let (after, _) = with_status_line(theirs, &status(), None).unwrap();
        assert!(shown.contains(&serde_json::to_string_pretty(&after["statusLine"]).unwrap()));
        assert!(shown.contains("node hud.mjs"));
    }

    #[test]
    fn remove_takes_out_only_ours() {
        let settings = json!({
            "hooks": {
                "Stop": [
                    { "hooks": [{ "type": "command", "command": "say done" }] },
                    { "hooks": [{ "type": "command", "command": "say also" }, { "type": "command", "command": cmd() }] }
                ],
                "Empty": []
            },
            "model": "opus"
        });
        let merged = with_hooks(settings, &cmd(), Shell::Bash).unwrap();
        let removed = without_hooks(merged).unwrap();
        assert!(!installed(&removed));
        let stop = removed["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2);
        assert_eq!(
            stop[1]["hooks"],
            json!([{ "type": "command", "command": "say also" }])
        );
        // Events that only had ours are gone; a list that was already empty stays.
        assert!(removed["hooks"].get("SessionEnd").is_none());
        assert_eq!(removed["hooks"]["Empty"], json!([]));
        assert_eq!(removed["model"], "opus");

        let only_ours = with_hooks(json!({ "model": "x" }), &cmd(), Shell::Bash).unwrap();
        assert_eq!(without_hooks(only_ours).unwrap(), json!({ "model": "x" }));
    }

    #[test]
    fn topbar_entries_are_replaced_but_do_not_count_as_connected() {
        let old = "'C:/Users/DELL 7300/AppData/Local/TopBar/topbar.exe' hook";
        let old_ps = r"& 'C:\Users\DELL 7300\AppData\Local\TopBar\topbar.exe' hook";
        let settings = json!({ "hooks": {
            "Stop": [{ "hooks": [{ "type": "command", "command": old }] }],
            "Notification": [{ "hooks": [{ "type": "command", "command": old_ps }] }]
        } });
        assert!(!installed(&settings));
        assert_eq!(ours_in(&settings, "Stop"), 1);

        let merged = with_hooks(settings.clone(), &cmd(), Shell::Bash).unwrap();
        assert!(installed(&merged));
        assert_eq!(ours_in(&merged, "Stop"), 1);
        assert_eq!(merged["hooks"]["Stop"][0]["hooks"][0]["command"], cmd());
        assert_eq!(without_hooks(settings).unwrap(), json!({}));
    }

    #[test]
    fn other_exes_ending_in_the_name_are_not_ours() {
        for command in [
            "'C:/Tools/decay.exe' hook",
            r"& 'C:\Tools\mytopbar.exe' hook",
            "cay.exe' hook",
        ] {
            assert!(!is_ours(&json!({ "command": command })), "{command}");
        }
    }

    #[test]
    fn refuses_shapes_it_does_not_understand() {
        assert!(with_hooks(json!([1, 2]), &cmd(), Shell::Bash).is_err());
        assert!(with_hooks(json!({ "hooks": "nope" }), &cmd(), Shell::Bash).is_err());
        assert!(with_hooks(json!({ "hooks": { "Stop": {} } }), &cmd(), Shell::Bash).is_err());
        assert!(!installed(&json!({ "hooks": 3 })));
    }

    #[test]
    fn files_are_backed_up_and_bad_json_is_left_alone() {
        let dir = std::env::temp_dir().join(format!("cay-hooks-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("settings.json");
        assert_eq!(read(&path), Ok(None));
        assert_eq!(write(&path, &json!({ "a": 1 })), Ok(None));
        let backup = write(&path, &json!({ "a": 2 })).unwrap().unwrap();
        assert_eq!(read(&backup).unwrap().unwrap()["a"], 1);
        assert_eq!(read(&path).unwrap().unwrap()["a"], 2);
        std::fs::write(&path, "{ not json").unwrap();
        assert!(read(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
