//! Which `claude.exe` to run, and which folders to offer.
//!
//! The `claude` on PATH is often an npm shim (a shell script plus `.cmd`), which can't be spawned without a
//! shell, so the real exe is looked up next to it. Versions come from `claude.exe --version` (bounded wait,
//! no window). Recent folders are TopBar's own runs first, then the projects Claude Code keeps transcripts for,
//! newest first.

use std::collections::HashSet;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, SystemTime};

use windows::Win32::System::Threading::CREATE_NO_WINDOW;

use crate::ipc::ClaudeInstall;

const VERSION_TIMEOUT: Duration = Duration::from_secs(5);
const NPM_EXE: &str = r"node_modules\@anthropic-ai\claude-code\bin\claude.exe";
const MAX_FOLDERS: usize = 10;

/// Every distinct install found, newest version first.
pub fn find() -> Vec<ClaudeInstall> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (path, source) in candidates() {
        let key = path.to_string_lossy().to_lowercase();
        if !path.is_file() || !seen.insert(key) {
            continue;
        }
        out.push(ClaudeInstall {
            version: version_of(&path),
            path: path.to_string_lossy().into_owned(),
            source,
        });
    }
    out.sort_by(|a, b| compare(b.version.as_deref(), a.version.as_deref()));
    out
}

/// The configured path if it exists, else the newest install.
pub fn pick(installs: &[ClaudeInstall], configured: Option<&str>) -> Option<String> {
    if let Some(path) = configured.filter(|p| Path::new(p).is_file()) {
        return Some(path.to_string());
    }
    installs.first().map(|i| i.path.clone())
}

fn candidates() -> Vec<(PathBuf, &'static str)> {
    let mut out = Vec::new();
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from);
    if let Some(home) = &home {
        out.push((home.join(r".local\bin\claude.exe"), "native"));
    }
    for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        out.push((dir.join("claude.exe"), "PATH"));
        if dir.join("claude.cmd").is_file() || dir.join("claude").is_file() {
            out.push((dir.join(NPM_EXE), "npm"));
        }
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        out.push((PathBuf::from(appdata).join("npm").join(NPM_EXE), "npm"));
    }
    if let Some(home) = &home {
        let extensions = home.join(r".vscode\extensions");
        if let Ok(entries) = std::fs::read_dir(&extensions) {
            let mut dirs: Vec<_> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("anthropic.claude-code-"))
                })
                .collect();
            dirs.sort();
            for dir in dirs.into_iter().rev() {
                out.push((dir.join(r"resources\native-binary\claude.exe"), "VS Code"));
            }
        }
    }
    out
}

fn version_of(exe: &Path) -> Option<String> {
    let mut child = Command::new(exe)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW.0)
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut text = String::new();
        let _ = std::io::Read::read_to_string(&mut stdout, &mut text);
        let _ = tx.send(text);
    });
    let text = rx.recv_timeout(VERSION_TIMEOUT).ok();
    let _ = child.kill();
    let _ = child.wait();
    parse_version(&text?)
}

/// `2.1.312 (Claude Code)` → `2.1.312`.
pub fn parse_version(text: &str) -> Option<String> {
    let first = text.split_whitespace().next()?;
    let valid = !first.is_empty()
        && first.split('.').count() >= 2
        && first
            .split('.')
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
    valid.then(|| first.to_string())
}

/// Numeric, part by part; an unknown version sorts below every known one.
pub fn compare(a: Option<&str>, b: Option<&str>) -> std::cmp::Ordering {
    let parts = |v: &str| -> Vec<u64> { v.split('.').map(|p| p.parse().unwrap_or(0)).collect() };
    match (a, b) {
        (Some(a), Some(b)) => parts(a).cmp(&parts(b)),
        (Some(_), None) => std::cmp::Ordering::Greater,
        (None, Some(_)) => std::cmp::Ordering::Less,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

/// TopBar's recent run folders first, then Claude Code's projects by their last transcript, existing only.
pub fn recent_folders(own: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let push = |folder: String, out: &mut Vec<String>| {
        if out.len() < MAX_FOLDERS
            && !out.iter().any(|f| f.eq_ignore_ascii_case(&folder))
            && Path::new(&folder).is_dir()
        {
            out.push(folder);
        }
    };
    for folder in own {
        push(folder.clone(), &mut out);
    }
    for folder in claude_projects() {
        push(folder, &mut out);
    }
    out
}

/// Project folders from `~/.claude.json`, ordered by the newest file in their `~/.claude/projects/<dir>`.
fn claude_projects() -> Vec<String> {
    let Some(home) = std::env::var_os("USERPROFILE").map(PathBuf::from) else {
        return Vec::new();
    };
    let config = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".claude"));
    let known: Vec<String> = std::fs::read_to_string(home.join(".claude.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| {
            v.get("projects")?
                .as_object()
                .map(|o| o.keys().cloned().collect())
        })
        .unwrap_or_default();
    let mut dated: Vec<(SystemTime, String)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(config.join("projects")) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(folder) = known
                .iter()
                .find(|k| project_dir(k).eq_ignore_ascii_case(&name))
            else {
                continue;
            };
            let modified = entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            dated.push((modified, folder.replace('/', "\\")));
        }
    }
    dated.sort_by_key(|a| std::cmp::Reverse(a.0));
    dated.into_iter().map(|(_, f)| f).collect()
}

/// Claude Code's transcript folder name for a project: every character that isn't a letter or digit becomes
/// `-` (`C:\Users\me\wpill` → `C--Users-me-wpill`).
pub fn project_dir(folder: &str) -> String {
    folder
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn versions() {
        assert_eq!(
            parse_version("2.1.312 (Claude Code)\n"),
            Some("2.1.312".into())
        );
        assert_eq!(parse_version("error: nope"), None);
        assert_eq!(parse_version(""), None);
        assert_eq!(compare(Some("2.1.312"), Some("2.1.159")), Ordering::Greater);
        assert_eq!(compare(Some("2.1.99"), Some("2.1.123")), Ordering::Less);
        assert_eq!(compare(Some("2.1.0"), None), Ordering::Greater);
    }

    #[test]
    fn transcript_folder_names() {
        assert_eq!(
            project_dir(r"C:\Users\DELL 7300\Documents\GitHub\wpill"),
            "C--Users-DELL-7300-Documents-GitHub-wpill"
        );
        assert_eq!(project_dir("C:/x_y.z"), "C--x-y-z");
    }

    #[test]
    fn pick_prefers_configured_then_newest() {
        let installs = vec![ClaudeInstall {
            path: r"C:\a\claude.exe".into(),
            version: Some("2.1.312".into()),
            source: "npm",
        }];
        assert_eq!(pick(&installs, None).as_deref(), Some(r"C:\a\claude.exe"));
        // A configured path that doesn't exist falls back.
        assert_eq!(
            pick(&installs, Some(r"Z:\missing\claude.exe")).as_deref(),
            Some(r"C:\a\claude.exe")
        );
        assert_eq!(pick(&[], None), None);
    }
}
