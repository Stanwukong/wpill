//! What a tool call is, in a few words: its risk class, the approval summary, the step label in the panel, and
//! what "allow for this run" remembers. Pure; unit-tested.

use serde_json::Value;

use super::{actions, tools};
use crate::desktop::plan;
use crate::ipc::AgentRisk;

const MAX_SUMMARY: usize = 160;
const MAX_LABEL: usize = 64;

/// Tools that only look. Claude Code rarely asks about these in default mode, but when it does (reads outside
/// the folder, network fetches) they're still only reads.
const READ_TOOLS: &[&str] = &[
    "Read",
    "Glob",
    "Grep",
    "LS",
    "WebFetch",
    "WebSearch",
    "TodoWrite",
    "NotebookRead",
];
const EDIT_TOOLS: &[&str] = &["Edit", "MultiEdit", "Write", "NotebookEdit"];
const SHELL_TOOLS: &[&str] = &["Bash", "PowerShell"];
/// Anything here in a shell command makes it compound: remembering its first word would allow far more.
const SHELL_OPERATORS: &[&str] = &["&", "|", ";", "`", "$(", ">", "<", "\n", "\r"];

/// Unknown tools (other MCP servers, new built-ins) count as changes. TopBar's desktop looks are reads.
pub fn risk(tool: &str) -> AgentRisk {
    if is_action(tool) {
        AgentRisk::Input
    } else if READ_TOOLS.contains(&tool) || is_desktop(tool) {
        AgentRisk::Read
    } else {
        AgentRisk::Change
    }
}

/// One of TopBar's own desktop looks (2.7c): `list_windows`, `focused_window`, `screenshot`, `ui_tree`.
pub fn is_desktop(tool: &str) -> bool {
    tools::short_name(tool).is_some()
}

/// One of TopBar's own desktop actions (2.7d): input into other apps, risk `input`, never remembered.
pub fn is_action(tool: &str) -> bool {
    actions::short_name(tool).is_some()
}

/// Text in an approval: quoted on one line (newlines as ⏎), cut to `max`.
fn quote(text: &str, max: usize) -> String {
    let one: String = text
        .replace("\r\n", "\n")
        .chars()
        .map(|c| if c == '\n' { '⏎' } else { c })
        .collect();
    format!("\"{}\"", truncate(&one, max))
}

/// The approval line for a desktop action: exactly what will happen, where. `target` names the window
/// ("Notepad · notes.txt"), `element` the control (`"Save"`). Typed text is shown short; when the audit asks
/// with the text redacted (`{ chars }`), only its length.
pub fn action_summary(
    tool: &str,
    input: &Value,
    target: Option<&str>,
    element: Option<&str>,
) -> String {
    let window = target.unwrap_or("a window that has closed");
    let thing = element.unwrap_or("an element that's no longer there");
    let button = str_field(input, "button");
    let click = match button {
        "right" => "Right-click",
        "double" => "Double-click",
        _ => "Click",
    };
    let text = match tools_text(input) {
        TextArg::Short(t) => format!("{} into {window}", quote(&t, 60)),
        TextArg::Long(t, n) => format!("{n} characters into {window}: {}", quote(&t, 40)),
        TextArg::Count(n) => format!("{n} characters into {window}"),
    };
    let summary = match actions::short_name(tool) {
        Some(actions::FOCUS_WINDOW) => format!("Bring {window} to the front"),
        Some(actions::INVOKE) => {
            let verb = match str_field(input, "action") {
                "toggle" => "Toggle",
                "select" => "Select",
                "expand" => "Expand",
                "collapse" => "Collapse",
                "scrollIntoView" => "Scroll to",
                "focus" => "Focus",
                _ => "Press",
            };
            format!("{verb} {thing} in {window}")
        }
        Some(actions::SET_VALUE) => {
            let value = match input.get("value") {
                Some(Value::String(v)) if v.chars().count() <= 60 && !v.contains('\n') => {
                    quote(v, 60)
                }
                Some(Value::String(v)) => format!("{} characters", v.chars().count()),
                Some(v) => format!("{} characters", v["chars"].as_u64().unwrap_or(0)),
                None => "nothing".into(),
            };
            format!("Set {thing} to {value} in {window}")
        }
        Some(actions::CLICK) if input.get("element").is_some_and(|v| !v.is_null()) => {
            format!("{click} {thing} in {window}")
        }
        Some(actions::CLICK) => {
            let (x, y) = (
                input["x"].as_i64().unwrap_or(0),
                input["y"].as_i64().unwrap_or(0),
            );
            let unit = if str_field(input, "from") == "screenshot" {
                " (screenshot px)"
            } else {
                ""
            };
            format!("{click} at {x}, {y}{unit} in {window}")
        }
        Some(actions::TYPE_TEXT) => format!("Type {text}"),
        Some(actions::PRESS_KEYS) => {
            let chords: Vec<String> = keys_field(input)
                .iter()
                .map(|k| match plan::parse_chord(k) {
                    Ok(c) => match plan::chord_note(&c) {
                        Some(note) => format!("{} ({note})", c.label()),
                        None => c.label(),
                    },
                    Err(_) => k.to_string(),
                })
                .collect();
            format!("Press {} in {window}", chords.join(", then "))
        }
        Some(actions::LAUNCH) => match (
            input.get("url").and_then(Value::as_str),
            str_field(input, "app"),
        ) {
            (Some(url), _) if url.trim().to_ascii_lowercase().starts_with("ms-settings:") => {
                format!("Open Settings ({})", url.trim())
            }
            (Some(url), _) => format!("Open {} in your browser", url.trim()),
            (None, app) => format!("Open the {} app", app.trim()),
        },
        _ => display_tool(tool),
    };
    truncate(&summary, MAX_SUMMARY)
}

enum TextArg {
    Short(String),
    Long(String, usize),
    Count(u64),
}

fn tools_text(input: &Value) -> TextArg {
    match input.get("text") {
        Some(Value::String(t)) if t.chars().count() <= 60 && !t.trim_end().contains('\n') => {
            TextArg::Short(t.clone())
        }
        Some(Value::String(t)) => TextArg::Long(t.clone(), t.chars().count()),
        Some(v) => TextArg::Count(v["chars"].as_u64().unwrap_or(0)),
        None => TextArg::Count(0),
    }
}

fn keys_field(input: &Value) -> Vec<&str> {
    match input.get("keys") {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect(),
        Some(Value::String(s)) => vec![s.as_str()],
        _ => vec![],
    }
}

/// What the island's acting activity says: "Clicking "Save"", "Typing into Notepad", "Pressing Ctrl+S".
pub fn acting_step(
    tool: &str,
    input: &Value,
    target: Option<&str>,
    element: Option<&str>,
) -> String {
    let window = target
        .map(|t| t.split(" · ").next().unwrap_or(t).to_string())
        .unwrap_or_else(|| "a window".into());
    let thing = element.map_or_else(|| window.clone(), str::to_string);
    let step = match actions::short_name(tool) {
        Some(actions::FOCUS_WINDOW) => format!("Bringing {window} forward"),
        Some(actions::INVOKE) => format!("Pressing {thing}"),
        Some(actions::SET_VALUE) => format!("Setting {thing}"),
        Some(actions::CLICK) => match element {
            Some(e) => format!("Clicking {e}"),
            None => format!("Clicking in {window}"),
        },
        Some(actions::TYPE_TEXT) => format!("Typing into {window}"),
        Some(actions::PRESS_KEYS) => {
            let keys: Vec<String> = keys_field(input)
                .iter()
                .map(|k| plan::parse_chord(k).map_or(k.to_string(), |c| c.label()))
                .collect();
            format!("Pressing {}", keys.join(", "))
        }
        Some(actions::LAUNCH) => "Opening".to_string(),
        _ => "Acting".to_string(),
    };
    truncate(&step, MAX_LABEL)
}

/// The approval line for a desktop look; `target` names the window ("Chrome · GitHub").
pub fn desktop_summary(tool: &str, input: &Value, target: Option<&str>) -> String {
    let window = target.unwrap_or("a window that has closed");
    let text = match tools::short_name(tool) {
        Some(tools::LIST_WINDOWS) => "List open windows".to_string(),
        Some(tools::FOCUSED_WINDOW) => "See which window is in front".to_string(),
        Some(tools::SCREENSHOT) if input.get("hwnd").is_none_or(Value::is_null) => {
            "Screenshot of the whole screen".to_string()
        }
        Some(tools::SCREENSHOT) => format!("Screenshot of {window}"),
        Some(tools::UI_TREE) => format!("Read the UI of {window}"),
        _ => display_tool(tool),
    };
    truncate(&text, MAX_SUMMARY)
}

/// `mcp__github__create_issue` → `github · create_issue`; built-ins stay as they are. TopBar's desktop tools
/// read as `desktop · screenshot`.
pub fn display_tool(tool: &str) -> String {
    if let Some(name) = tools::short_name(tool).or_else(|| actions::short_name(tool)) {
        return format!("desktop · {name}");
    }
    match tool
        .strip_prefix("mcp__")
        .and_then(|rest| rest.split_once("__"))
    {
        Some((server, name)) => format!("{server} · {name}"),
        None => tool.to_string(),
    }
}

/// The approval's one line: the command, the file, the host, the query.
pub fn summary(tool: &str, input: &Value, cwd: &str) -> String {
    let text = if SHELL_TOOLS.contains(&tool) {
        one_line(str_field(input, "command"))
    } else if let Some(path) = path_field(input) {
        relative(path, cwd)
    } else {
        match tool {
            "WebFetch" => host(str_field(input, "url")).to_string(),
            "WebSearch" => one_line(str_field(input, "query")),
            "Glob" | "Grep" => one_line(str_field(input, "pattern")),
            "Task" | "Agent" => one_line(str_field(input, "description")),
            _ => String::new(),
        }
    };
    if text.is_empty() {
        display_tool(tool)
    } else {
        truncate(&text, MAX_SUMMARY)
    }
}

/// What the run is doing, for the panel row: "Running git status", "Editing main.rs".
pub fn step_label(tool: &str, input: &Value, cwd: &str) -> String {
    let file = || {
        path_field(input)
            .map(|p| file_name(p).to_string())
            .unwrap_or_default()
    };
    let label = match tool {
        "Bash" | "PowerShell" => format!("Running {}", one_line(str_field(input, "command"))),
        "Edit" | "MultiEdit" | "NotebookEdit" => format!("Editing {}", file()),
        "Write" => format!("Writing {}", file()),
        "Read" | "NotebookRead" => format!("Reading {}", file()),
        "Glob" | "Grep" | "LS" => "Searching".to_string(),
        "WebFetch" => format!("Fetching {}", host(str_field(input, "url"))),
        "WebSearch" => "Searching the web".to_string(),
        "Task" | "Agent" => "Delegating".to_string(),
        "TodoWrite" => "Planning".to_string(),
        "mcp__topbar__list_windows" => "Listing windows".to_string(),
        "mcp__topbar__focused_window" => "Checking the front window".to_string(),
        "mcp__topbar__screenshot" => "Taking a screenshot".to_string(),
        "mcp__topbar__ui_tree" => "Reading a window's UI".to_string(),
        "mcp__topbar__focus_window" => "Bringing a window forward".to_string(),
        "mcp__topbar__invoke" => "Using a control".to_string(),
        "mcp__topbar__set_value" => "Setting a field".to_string(),
        "mcp__topbar__click" => "Clicking".to_string(),
        "mcp__topbar__type_text" => "Typing".to_string(),
        "mcp__topbar__press_keys" => "Pressing keys".to_string(),
        "mcp__topbar__launch" => "Opening".to_string(),
        _ => summary(tool, input, cwd),
    };
    truncate(label.trim(), MAX_LABEL)
}

/// What "allow for this run" covers, or `None` if it can't be offered: `input` actions, and shell commands with
/// operators (remembering `cd` would also allow `cd x && rm -rf y`). Shell commands are remembered by their
/// first word (`Bash:git`), edits as one group, fetches per host, everything else per tool.
pub fn remember_key(tool: &str, input: &Value) -> Option<String> {
    if risk(tool) == AgentRisk::Input {
        return None;
    }
    if SHELL_TOOLS.contains(&tool) {
        let command = str_field(input, "command").trim();
        if command.is_empty() || SHELL_OPERATORS.iter().any(|op| command.contains(op)) {
            return None;
        }
        let first = command.split_whitespace().next()?;
        let program = file_name(first.trim_matches(['"', '\'']));
        let program = program
            .strip_suffix(".exe")
            .or_else(|| program.strip_suffix(".EXE"))
            .unwrap_or(program);
        return Some(format!("{tool}:{}", program.to_ascii_lowercase()));
    }
    if EDIT_TOOLS.contains(&tool) {
        return Some("edit".into());
    }
    // One kind per key: a remembered screenshot covers other windows and the screen too, so the glow and the
    // approval's wording say so ("Allow screenshots for this run").
    match tools::short_name(tool) {
        Some(tools::LIST_WINDOWS | tools::FOCUSED_WINDOW) => return Some("desktop:windows".into()),
        Some(tools::SCREENSHOT) => return Some("desktop:screenshots".into()),
        Some(tools::UI_TREE) => return Some("desktop:ui".into()),
        _ => {}
    }
    if tool == "WebFetch" {
        return Some(format!(
            "WebFetch:{}",
            host(str_field(input, "url")).to_ascii_lowercase()
        ));
    }
    Some(tool.to_string())
}

/// What "for this run" reads as in the island, given a remember key.
pub fn remember_scope(key: &str) -> String {
    match key {
        "desktop:windows" => return "window lists".into(),
        "desktop:screenshots" => return "screenshots".into(),
        "desktop:ui" => return "reading UIs".into(),
        _ => {}
    }
    match key.split_once(':') {
        Some((_, program)) if !program.is_empty() => program.to_string(),
        _ if key == "edit" => "edits".to_string(),
        _ => display_tool(key),
    }
}

fn str_field<'a>(input: &'a Value, key: &str) -> &'a str {
    input.get(key).and_then(Value::as_str).unwrap_or("")
}

fn path_field(input: &Value) -> Option<&str> {
    ["file_path", "notebook_path", "path"]
        .iter()
        .find_map(|k| input.get(*k).and_then(Value::as_str))
        .filter(|p| !p.is_empty())
}

/// `path` relative to `cwd` when it's inside it (either slash, any case), else as given.
fn relative(path: &str, cwd: &str) -> String {
    let norm = |s: &str| s.replace('\\', "/").to_lowercase();
    let base = norm(cwd.trim_end_matches(['\\', '/']));
    let lower = norm(path);
    if !base.is_empty() && lower.starts_with(&format!("{base}/")) {
        return path[base.len() + 1..].replace('\\', "/");
    }
    path.to_string()
}

fn file_name(path: &str) -> &str {
    path.trim_end_matches(['\\', '/'])
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(path)
}

fn host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.split(['/', '?', '#']).next().unwrap_or(rest)
}

/// First line, whitespace collapsed.
fn one_line(text: &str) -> String {
    let first = text.trim().lines().next().unwrap_or("");
    let collapsed = first.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.trim().lines().nth(1).is_some() {
        format!("{collapsed} …")
    } else {
        collapsed
    }
}

pub fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max - 1).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const CWD: &str = r"C:\Users\me\GitHub\wpill";

    #[test]
    fn risk_classes() {
        assert_eq!(risk("Read"), AgentRisk::Read);
        assert_eq!(risk("WebFetch"), AgentRisk::Read);
        assert_eq!(risk("Bash"), AgentRisk::Change);
        assert_eq!(risk("Edit"), AgentRisk::Change);
        assert_eq!(risk("mcp__github__create_issue"), AgentRisk::Change);
        assert_eq!(risk("SomethingNew"), AgentRisk::Change);
        assert_eq!(risk("mcp__topbar__click"), AgentRisk::Input);
        assert_eq!(risk("mcp__topbar__screenshot"), AgentRisk::Read);
        assert_eq!(risk("mcp__topbar__list_windows"), AgentRisk::Read);
        assert_eq!(risk("mcp__topbar__approve"), AgentRisk::Change);
        assert_eq!(risk("mcp__other__screenshot"), AgentRisk::Change);
    }

    #[test]
    fn desktop_looks() {
        let shot = "mcp__topbar__screenshot";
        assert!(is_desktop(shot));
        assert!(!is_desktop("mcp__topbar__click"));
        assert_eq!(display_tool(shot), "desktop · screenshot");
        assert_eq!(
            desktop_summary(shot, &json!({ "hwnd": 5 }), Some("Chrome · GitHub")),
            "Screenshot of Chrome · GitHub"
        );
        assert_eq!(
            desktop_summary(shot, &json!({}), None),
            "Screenshot of the whole screen"
        );
        assert_eq!(
            desktop_summary("mcp__topbar__ui_tree", &json!({ "hwnd": 5 }), None),
            "Read the UI of a window that has closed"
        );
        assert_eq!(
            desktop_summary("mcp__topbar__focused_window", &json!({}), None),
            "See which window is in front"
        );
        assert_eq!(step_label(shot, &json!({}), CWD), "Taking a screenshot");
        let key = |tool: &str| remember_key(tool, &json!({ "hwnd": 1 })).unwrap();
        assert_eq!(
            key("mcp__topbar__list_windows"),
            key("mcp__topbar__focused_window")
        );
        assert_ne!(key(shot), key("mcp__topbar__ui_tree"));
        assert_eq!(remember_scope(&key(shot)), "screenshots");
        assert_eq!(
            remember_scope(&key("mcp__topbar__list_windows")),
            "window lists"
        );
        assert_eq!(remember_scope(&key("mcp__topbar__ui_tree")), "reading UIs");
    }

    #[test]
    fn action_summaries() {
        let np = Some("Notepad · notes.txt");
        let s = |tool: &str, input: Value, element: Option<&str>| {
            action_summary(&format!("mcp__topbar__{tool}"), &input, np, element)
        };
        assert_eq!(
            s(
                "invoke",
                json!({ "hwnd": 1, "element": 3 }),
                Some("\"Save\"")
            ),
            "Press \"Save\" in Notepad · notes.txt"
        );
        assert_eq!(
            s(
                "invoke",
                json!({ "element": 3, "action": "toggle" }),
                Some("\"Word wrap\"")
            ),
            "Toggle \"Word wrap\" in Notepad · notes.txt"
        );
        assert_eq!(
            s("click", json!({ "element": 3 }), Some("\"Save\"")),
            "Click \"Save\" in Notepad · notes.txt"
        );
        assert_eq!(
            s(
                "click",
                json!({ "x": 812, "y": 40, "button": "right" }),
                None
            ),
            "Right-click at 812, 40 in Notepad · notes.txt"
        );
        assert_eq!(
            s(
                "click",
                json!({ "x": 8, "y": 4, "from": "screenshot" }),
                None
            ),
            "Click at 8, 4 (screenshot px) in Notepad · notes.txt"
        );
        assert_eq!(
            s("type_text", json!({ "text": "hello world" }), None),
            "Type \"hello world\" into Notepad · notes.txt"
        );
        assert_eq!(
            s("type_text", json!({ "text": "hi\n" }), None),
            "Type \"hi⏎\" into Notepad · notes.txt"
        );
        let long = s(
            "type_text",
            json!({ "text": "Dear team,\nthe release is out." }),
            None,
        );
        assert!(
            long.starts_with("Type 30 characters into Notepad · notes.txt: \"Dear team,⏎"),
            "{long}"
        );
        assert_eq!(
            s(
                "type_text",
                json!({ "text": { "chars": 42, "lines": 1 } }),
                None
            ),
            "Type 42 characters into Notepad · notes.txt",
            "redacted for the audit"
        );
        assert_eq!(
            s(
                "set_value",
                json!({ "element": 2, "value": "weather" }),
                Some("\"Search\"")
            ),
            "Set \"Search\" to \"weather\" in Notepad · notes.txt"
        );
        assert_eq!(
            s("press_keys", json!({ "keys": ["ctrl+s"] }), None),
            "Press Ctrl+S in Notepad · notes.txt"
        );
        assert_eq!(
            s(
                "press_keys",
                json!({ "keys": ["Ctrl+A", "Delete", "alt+f4"] }),
                None
            ),
            "Press Ctrl+A, then Delete, then Alt+F4 (closes the window) in Notepad · notes.txt"
        );
        assert_eq!(
            s("focus_window", json!({ "hwnd": 1 }), None),
            "Bring Notepad · notes.txt to the front"
        );
        assert_eq!(
            s("launch", json!({ "url": "https://github.com" }), None),
            "Open https://github.com in your browser"
        );
        assert_eq!(
            s("launch", json!({ "app": "Calculator" }), None),
            "Open the Calculator app"
        );
        assert_eq!(
            s("launch", json!({ "url": "ms-settings:display" }), None),
            "Open Settings (ms-settings:display)"
        );
        assert_eq!(
            acting_step("mcp__topbar__type_text", &json!({}), np, None),
            "Typing into Notepad"
        );
        assert_eq!(
            acting_step("mcp__topbar__click", &json!({}), np, Some("\"Save\"")),
            "Clicking \"Save\""
        );
        assert_eq!(
            acting_step(
                "mcp__topbar__press_keys",
                &json!({ "keys": ["ctrl+s"] }),
                np,
                None
            ),
            "Pressing Ctrl+S"
        );
        assert_eq!(display_tool("mcp__topbar__click"), "desktop · click");
        assert_eq!(
            step_label("mcp__topbar__type_text", &json!({}), CWD),
            "Typing"
        );
    }

    #[test]
    fn summaries() {
        let bash = json!({ "command": "git   push origin main", "description": "Push" });
        assert_eq!(summary("Bash", &bash, CWD), "git push origin main");
        let multi = json!({ "command": "echo a\necho b" });
        assert_eq!(summary("Bash", &multi, CWD), "echo a …");
        let edit =
            json!({ "file_path": r"C:\Users\me\GitHub\wpill\src\main.rs", "old_string": "a" });
        assert_eq!(summary("Edit", &edit, CWD), "src/main.rs");
        let outside = json!({ "file_path": r"D:\other\notes.txt" });
        assert_eq!(summary("Write", &outside, CWD), r"D:\other\notes.txt");
        let fetch = json!({ "url": "https://docs.rs/windows/latest?x=1" });
        assert_eq!(summary("WebFetch", &fetch, CWD), "docs.rs");
        assert_eq!(
            summary("mcp__github__create_issue", &json!({}), CWD),
            "github · create_issue"
        );
        let long = json!({ "command": "x".repeat(400) });
        assert_eq!(summary("Bash", &long, CWD).chars().count(), MAX_SUMMARY);
    }

    #[test]
    fn step_labels() {
        let bash = json!({ "command": "git status" });
        assert_eq!(step_label("Bash", &bash, CWD), "Running git status");
        let edit = json!({ "file_path": r"C:\Users\me\GitHub\wpill\src\main.rs" });
        assert_eq!(step_label("Edit", &edit, CWD), "Editing main.rs");
        assert_eq!(
            step_label("Grep", &json!({ "pattern": "x" }), CWD),
            "Searching"
        );
        assert_eq!(step_label("mcp__x__y", &json!({}), CWD), "x · y");
    }

    #[test]
    fn remember_keys() {
        let key = |tool: &str, input: Value| remember_key(tool, &input);
        assert_eq!(
            key("Bash", json!({ "command": "git push" })),
            Some("Bash:git".into())
        );
        assert_eq!(
            key(
                "PowerShell",
                json!({ "command": r"C:\tools\Node.EXE x.js" })
            ),
            Some("PowerShell:node".into())
        );
        assert_eq!(key("Bash", json!({ "command": "cd x && rm -rf y" })), None);
        assert_eq!(key("Bash", json!({ "command": "ls | wc -l" })), None);
        assert_eq!(key("Bash", json!({ "command": "echo $(whoami)" })), None);
        assert_eq!(key("Bash", json!({ "command": "a\nb" })), None);
        assert_eq!(
            key("Write", json!({ "file_path": "a" })),
            Some("edit".into())
        );
        assert_eq!(key("Edit", json!({})), Some("edit".into()));
        assert_eq!(
            key("WebFetch", json!({ "url": "https://Docs.rs/x" })),
            Some("WebFetch:docs.rs".into())
        );
        assert_eq!(key("mcp__topbar__type_text", json!({})), None);
        for name in actions::NAMES {
            let tool = format!("mcp__topbar__{name}");
            assert_eq!(risk(&tool), AgentRisk::Input);
            assert_eq!(
                key(&tool, json!({ "hwnd": 1 })),
                None,
                "{name} is never remembered"
            );
        }
        assert_eq!(remember_scope("Bash:git"), "git");
        assert_eq!(remember_scope("edit"), "edits");
    }
}
