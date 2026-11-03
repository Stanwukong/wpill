//! Which version of an agent CLI you'd run (2.9a): Connect refuses one older than what Cay was checked with.
//! `parse` is pure and unit-tested; `installed` looks at PATH as a shell would.

use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;
use windows::Win32::System::Threading::CREATE_NO_WINDOW;

pub type Version = (u32, u32, u32);

const RUN_TIMEOUT: Duration = Duration::from_secs(5);

/// `codex-cli 0.166.0` (or a bare `0.166.0`) → `(0, 166, 0)`.
pub fn parse(text: &str) -> Option<Version> {
    let word = text
        .split_whitespace()
        .find(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit()))?;
    let core = word.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u32>().ok());
    Some((
        parts.next()??,
        parts.next()??,
        parts.next().flatten().unwrap_or(0),
    ))
}

pub fn text((a, b, c): Version) -> String {
    format!("{a}.{b}.{c}")
}

/// The first `name` on PATH: a native `<name>.exe` is asked with `--version`; npm's shim (`<name>.cmd`, `.ps1` or
/// bare) is read from its package's `package.json` (`package`, relative to the shim's folder, e.g.
/// `node_modules\@openai\codex`). `None` if there's none, or it didn't say.
pub fn installed(name: &str, package: &str) -> Option<Version> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let exe = dir.join(format!("{name}.exe"));
        if exe.is_file() {
            return run(&exe);
        }
        let shim = [
            format!("{name}.cmd"),
            format!("{name}.ps1"),
            name.to_string(),
        ]
        .iter()
        .any(|n| dir.join(n).is_file());
        if shim {
            return std::fs::read_to_string(dir.join(package).join("package.json"))
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .and_then(|v| v.get("version")?.as_str().and_then(parse));
        }
    }
    None
}

fn run(exe: &Path) -> Option<Version> {
    let mut child = Command::new(exe)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW.0)
        .spawn()
        .ok()?;
    let deadline = Instant::now() + RUN_TIMEOUT;
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
    let output = child.wait_with_output().ok()?;
    parse(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(parse("codex-cli 0.166.0"), Some((0, 166, 0)));
        assert_eq!(parse("1.19.5\n"), Some((1, 19, 5)));
        assert_eq!(parse("0.167.2-alpha.1\n"), Some((0, 167, 2)));
        assert_eq!(parse("codex-cli 1.2"), Some((1, 2, 0)));
        assert_eq!(parse("codex-cli"), None);
        assert!(parse("codex-cli 0.130.0").unwrap() < (0, 166, 0));
        assert_eq!(text((1, 19, 5)), "1.19.5");
    }
}
