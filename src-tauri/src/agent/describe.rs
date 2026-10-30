//! What a tool call is, in a few words: its risk class, the approval summary, the step label in the panel, and
//! what "allow for this run" remembers. Pure; unit-tested.

use serde_json::Value;

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
/// TopBar's own desktop actions (stage 2.7d): input into other apps, never remembered.
const INPUT_TOOLS: &[&str] = &[
    "mcp__topbar__focus_window",
    "mcp__topbar__invoke",
    "mcp__topbar__set_value",
    "mcp__topbar__click",
    "mcp__topbar__type_text",
    "mcp__topbar__press_keys",
    "mcp__topbar__launch",
];
const EDIT_TOOLS: &[&str] = &["Edit", "MultiEdit", "Write", "NotebookEdit"];
const SHELL_TOOLS: &[&str] = &["Bash", "PowerShell"];
/// Anything here in a shell command makes it compound: remembering its first word would allow far more.
const SHELL_OPERATORS: &[&str] = &["&", "|", ";", "`", "$(", ">", "<", "\n", "\r"];

/// Unknown tools (other MCP servers, new built-ins) count as changes.
pub fn risk(tool: &str) -> AgentRisk {
    if INPUT_TOOLS.contains(&tool) {
        AgentRisk::Input
    } else if READ_TOOLS.contains(&tool) {
        AgentRisk::Read
    } else {
        AgentRisk::Change
    }
}

/// `mcp__github__create_issue` → `github · create_issue`; built-ins stay as they are.
pub fn display_tool(tool: &str) -> String {
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
        assert_eq!(remember_scope("Bash:git"), "git");
        assert_eq!(remember_scope("edit"), "edits");
    }
}
