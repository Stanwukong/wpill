//! Which window runs a session: the nearest ancestor of the hook that owns an Alt+Tab window (Windows
//! Terminal, VS Code, conhost). When that process owns several windows (VS Code, Terminal), the one whose
//! title names the project wins, else the most recently used one.

use windows::Win32::Foundation::HWND;

use super::process;
use super::protocol::Proc;
use crate::apps::{self, TopLevel};

pub fn find(chain: &[Proc], project: &str, own: HWND) -> Option<isize> {
    // Only ancestors that are still the same process (pids get reused).
    let live: Vec<u32> = chain
        .iter()
        .filter(|p| process::alive(p))
        .map(|p| p.pid)
        .collect();
    choose(&apps::top_level_windows(own), &live, project)
}

/// `windows` in Z-order; `pids` nearest ancestor first.
fn choose(windows: &[TopLevel], pids: &[u32], project: &str) -> Option<isize> {
    for &pid in pids {
        let owned: Vec<&TopLevel> = windows.iter().filter(|w| w.pid == pid).collect();
        if owned.is_empty() {
            continue;
        }
        let named = owned
            .iter()
            .find(|w| names(&w.title, project))
            .or_else(|| owned.iter().find(|w| mentions(&w.title, project)));
        return named.or(owned.first()).map(|w| w.hwnd);
    }
    None
}

/// A title segment is exactly the project: `main.rs — wpill — Visual Studio Code`.
fn names(title: &str, project: &str) -> bool {
    [" \u{2014} ", " - ", " | "]
        .iter()
        .flat_map(|sep| title.split(sep))
        .any(|s| s.trim().eq_ignore_ascii_case(project))
}

fn mentions(title: &str, project: &str) -> bool {
    title.to_lowercase().contains(&project.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(hwnd: isize, pid: u32, title: &str) -> TopLevel {
        TopLevel {
            hwnd,
            pid,
            title: title.into(),
        }
    }

    #[test]
    fn nearest_ancestor_with_a_window_wins() {
        // bash (1) → claude (2) → pwsh (3) → WindowsTerminal (4)
        let windows = [w(10, 99, "Browser"), w(20, 4, "Claude Code")];
        assert_eq!(choose(&windows, &[1, 2, 3, 4], "wpill"), Some(20));
        assert_eq!(choose(&windows, &[1, 2, 3], "wpill"), None);
    }

    #[test]
    fn several_vs_code_windows_match_the_folder() {
        let windows = [
            w(1, 7, "notes.md — api — Visual Studio Code"),
            w(2, 7, "main.rs — wpill — Visual Studio Code"),
            w(3, 7, "wpill-old — Visual Studio Code"),
        ];
        assert_eq!(choose(&windows, &[7], "wpill"), Some(2));
        assert_eq!(choose(&windows, &[7], "api"), Some(1));
        // Nothing names it: the most recently used one.
        assert_eq!(choose(&windows, &[7], "other"), Some(1));
        // Only a partial match: still better than nothing.
        assert_eq!(choose(&windows, &[7], "old"), Some(3));
    }
}
