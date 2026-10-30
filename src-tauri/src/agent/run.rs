//! Starting `claude.exe` for a run: the command line, the process in its job, and the threads that read it.
//!
//! The process gets no console (`CREATE_NO_WINDOW`) and starts suspended so it's in the job before it can start
//! anything. Stdout lines go to `on_line`; when stdout ends, the exit code and the tail of stderr go to `on_exit`.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{ChildStdin, Command, Stdio};

use windows::Win32::System::Threading::{CREATE_NO_WINDOW, CREATE_SUSPENDED};

use super::job::Job;
use super::mcp::{APPROVE_TOOL, SERVER_NAME};

/// A stdout line longer than this (a huge tool result) is skipped.
const MAX_LINE: usize = 16 * 1024 * 1024;
/// Stderr kept for the error message when a run fails.
const STDERR_TAIL: usize = 2000;

/// Appended to Claude Code's system prompt for every agent run.
pub const GUIDANCE: &str = "You are running headless, started from TopBar, a Windows desktop helper. The user \
isn't watching a terminal: they see a one-line status at the top of the screen and approve risky steps with Allow \
or Deny buttons there. Nobody can answer questions while you work, so don't ask; make a sensible choice and state \
your assumptions. If you truly can't go on, stop and say exactly what you need in your final message; the user can \
reply with a follow-up. Prefer the shell and files. Never type, print or send passwords, tokens or other secrets. \
Avoid interactive commands and commands that never end, and don't leave servers or watchers running. End with a \
short summary of what you did and where the results are.

TopBar's desktop tools (mcp__topbar__list_windows, focused_window, ui_tree, screenshot) let you look at the user's \
screen. They only look: you can't click, type or focus anything. Use them only when the task is about what's on \
screen and the shell or files can't answer it. Every look is approved by the user and makes the screen edges glow, \
so before each one say in one sentence what you're looking for. Get a window's hwnd from list_windows (list again \
if a window has closed). Prefer ui_tree (text, cheap, exact) over screenshot; take a screenshot only for visual \
questions (layout, images, charts) or when the tree is empty. Don't repeat sensitive things you happen to see that \
the task doesn't need.";

#[derive(Debug, Clone)]
pub struct Launch {
    pub exe: String,
    pub cwd: String,
    pub model: Option<String>,
    pub max_turns: u32,
    pub max_budget_usd: Option<f64>,
    /// Claude Code's session id, for a follow-up after the process ended.
    pub resume: Option<String>,
    pub strict_mcp_config: bool,
    pub user_settings: bool,
}

/// The arguments after `claude.exe`. Never `bypassPermissions`: every prompt goes to `approve`.
pub fn args(l: &Launch, mcp_config: &Path) -> Vec<String> {
    let mut a: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--permission-mode",
        "default",
    ]
    .map(String::from)
    .into();
    a.push("--permission-prompt-tool".into());
    a.push(format!("mcp__{SERVER_NAME}__{APPROVE_TOOL}"));
    a.push("--mcp-config".into());
    a.push(mcp_config.to_string_lossy().into_owned());
    if l.strict_mcp_config {
        a.push("--strict-mcp-config".into());
    }
    if !l.user_settings {
        a.push("--setting-sources".into());
        a.push("project,local".into());
    }
    // Nobody can type an answer mid-run; Claude asks in its final message instead.
    a.push("--disallowedTools".into());
    a.push("AskUserQuestion".into());
    a.push("--max-turns".into());
    a.push(l.max_turns.max(1).to_string());
    if let Some(budget) = l.max_budget_usd.filter(|b| *b > 0.0) {
        a.push("--max-budget-usd".into());
        a.push(format!("{budget}"));
    }
    if let Some(model) = l.model.as_deref().filter(|m| !m.trim().is_empty()) {
        a.push("--model".into());
        a.push(model.trim().into());
    }
    if let Some(session) = &l.resume {
        a.push("--resume".into());
        a.push(session.clone());
    }
    a.push("--append-system-prompt".into());
    a.push(GUIDANCE.into());
    a
}

pub struct Process {
    pub stdin: Option<ChildStdin>,
    pub job: Job,
}

impl Process {
    /// Writes one stream-json line. Fails if stdin is closed.
    pub fn write(&mut self, line: &[u8]) -> std::io::Result<()> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::BrokenPipe))?;
        stdin.write_all(line)?;
        stdin.flush()
    }
}

pub fn spawn(
    launch: &Launch,
    mcp_config: &Path,
    mut on_line: impl FnMut(&[u8]) + Send + 'static,
    on_exit: impl FnOnce(Option<i32>, String) + Send + 'static,
) -> std::io::Result<Process> {
    let job = Job::new().map_err(std::io::Error::other)?;
    let mut child = Command::new(&launch.exe)
        .args(args(launch, mcp_config))
        .current_dir(&launch.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW.0 | CREATE_SUSPENDED.0)
        .spawn()?;
    if let Err(e) = job.adopt(&child) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(std::io::Error::other(e));
    }
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let tail = std::thread::Builder::new()
        .name("agent-run-err".into())
        .spawn(move || {
            let mut kept = Vec::new();
            if let Some(mut stderr) = stderr {
                let mut chunk = [0u8; 4096];
                while let Ok(n) = stderr.read(&mut chunk) {
                    if n == 0 {
                        break;
                    }
                    kept.extend_from_slice(&chunk[..n]);
                    if kept.len() > STDERR_TAIL * 2 {
                        kept.drain(..kept.len() - STDERR_TAIL);
                    }
                }
            }
            let text = String::from_utf8_lossy(&kept).into_owned();
            let start = text.len().saturating_sub(STDERR_TAIL);
            let start = (start..=text.len())
                .find(|&i| text.is_char_boundary(i))
                .unwrap_or(text.len());
            text[start..].trim().to_string()
        })?;

    std::thread::Builder::new()
        .name("agent-run-out".into())
        .spawn(move || {
            if let Some(stdout) = stdout {
                let mut reader = BufReader::new(stdout);
                let mut line = Vec::new();
                loop {
                    line.clear();
                    match reader.read_until(b'\n', &mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) if line.len() > MAX_LINE => {}
                        Ok(_) => on_line(line.trim_ascii()),
                    }
                }
            }
            let code = child.wait().ok().and_then(|s| s.code());
            let stderr = tail.join().unwrap_or_default();
            on_exit(code, stderr);
        })?;

    Ok(Process { stdin, job })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launch() -> Launch {
        Launch {
            exe: r"C:\claude.exe".into(),
            cwd: r"C:\w".into(),
            model: Some("haiku".into()),
            max_turns: 25,
            max_budget_usd: Some(1.0),
            resume: None,
            strict_mcp_config: true,
            user_settings: true,
        }
    }

    fn has_pair(args: &[String], flag: &str, value: &str) -> bool {
        args.windows(2).any(|w| w[0] == flag && w[1] == value)
    }

    #[test]
    fn command_line() {
        let a = args(&launch(), Path::new(r"C:\runs\r1\mcp.json"));
        assert_eq!(a[0], "-p");
        assert!(has_pair(&a, "--permission-mode", "default"));
        assert!(has_pair(
            &a,
            "--permission-prompt-tool",
            "mcp__topbar__approve"
        ));
        assert!(has_pair(&a, "--mcp-config", r"C:\runs\r1\mcp.json"));
        assert!(has_pair(&a, "--input-format", "stream-json"));
        assert!(has_pair(&a, "--max-turns", "25"));
        assert!(has_pair(&a, "--max-budget-usd", "1"));
        assert!(has_pair(&a, "--model", "haiku"));
        assert!(has_pair(&a, "--disallowedTools", "AskUserQuestion"));
        assert!(a.contains(&"--strict-mcp-config".to_string()));
        assert!(!a.contains(&"--setting-sources".to_string()));
        assert!(!a.contains(&"--resume".to_string()));
        assert!(!a
            .iter()
            .any(|x| x.contains("bypass") || x.contains("dangerously")));
        assert_eq!(a.last().map(String::as_str), Some(GUIDANCE));
    }

    #[test]
    fn options_change_the_command_line() {
        let l = Launch {
            model: None,
            max_budget_usd: None,
            resume: Some("sid".into()),
            strict_mcp_config: false,
            user_settings: false,
            max_turns: 0,
            ..launch()
        };
        let a = args(&l, Path::new("m.json"));
        assert!(!a.contains(&"--model".to_string()));
        assert!(!a.contains(&"--max-budget-usd".to_string()));
        assert!(!a.contains(&"--strict-mcp-config".to_string()));
        assert!(has_pair(&a, "--setting-sources", "project,local"));
        assert!(has_pair(&a, "--resume", "sid"));
        assert!(has_pair(&a, "--max-turns", "1"));
    }
}
