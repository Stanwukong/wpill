//! Ending an app from the panel's Memory section (Phase 3b, D49).
//!
//! `end_process_group` only acts on processes Rust itself listed for that app, checks each one again by
//! `(pid, creation time)` against a fresh snapshot (before the confirm box and again after it), and refuses the whole
//! app if any member is protected. Then it asks the app to close (`WM_CLOSE` to its windows, so it can prompt to save),
//! waits up to 5 s, and ends what's left with `TerminateProcess`, children before parents. An app that is showing a
//! dialog after `WM_CLOSE` (its window disabled: "Save changes?") is left open rather than ended under the prompt.
//!
//! The top half is pure and unit-tested; the bottom half is the Win32 calls.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use windows::core::HSTRING;
use windows::Win32::Foundation::{
    ERROR_INVALID_PARAMETER, E_ACCESSDENIED, FILETIME, LPARAM, STILL_ACTIVE, WAIT_OBJECT_0, WPARAM,
};
use windows::Win32::System::Threading::{
    GetExitCodeProcess, GetProcessTimes, OpenProcess, TerminateProcess, WaitForSingleObject,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled;
use windows::Win32::UI::WindowsAndMessaging::{
    IsWindow, MessageBoxW, PostMessageW, IDOK, MB_DEFBUTTON2, MB_ICONWARNING, MB_OKCANCEL,
    MB_SETFOREGROUND, MB_TOPMOST, WM_CLOSE,
};

use super::logic::{self, Proc};
use super::snapshot::{self, RawProcess};
use crate::apps;
use crate::ipc::EndOutcome;
use crate::win32::{self, Handle};

/// How long an app gets to close its windows before what's left is ended.
const CLOSE_WAIT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(200);
/// How long to wait for a terminated process to be gone before reporting it.
const EXIT_WAIT_MS: u32 = 2000;

/// `(pid, creation time)`: a process, never confused with a later one that reuses the pid.
pub type Id = (u32, u64);

/// What a fresh snapshot says about a request.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Checked {
    /// Still running and part of the app, children before parents.
    pub go: Vec<Id>,
    /// Already settled: gone, changed, or refused.
    pub settled: Vec<(Id, EndOutcome)>,
}

/// Checks `requested` against what Rust listed for the app (`listed`) and a fresh snapshot (`now`).
///
/// - not in the app's list → `Changed` (the page can't add pids of its own);
/// - pid gone → `NotFound`; pid running with another creation time → `Changed` (reused);
/// - any remaining member protected → every remaining one `Protected`, nothing to do.
pub fn check(requested: &[Id], listed: &[Id], now: &[Proc], own_pid: u32) -> Checked {
    let listed: HashSet<Id> = listed.iter().copied().collect();
    let by_pid: HashMap<u32, &Proc> = now.iter().map(|p| (p.pid, p)).collect();
    let mut seen = HashSet::new();
    let mut out = Checked::default();
    let mut alive: Vec<&Proc> = Vec::new();
    for &id in requested {
        if !seen.insert(id) {
            continue;
        }
        if !listed.contains(&id) {
            out.settled.push((id, EndOutcome::Changed));
            continue;
        }
        match by_pid.get(&id.0) {
            None => out.settled.push((id, EndOutcome::NotFound)),
            Some(p) if p.created != id.1 => out.settled.push((id, EndOutcome::Changed)),
            Some(p) => alive.push(p),
        }
    }
    if alive.iter().any(|p| logic::is_protected(p, own_pid)) {
        out.settled.extend(
            alive
                .iter()
                .map(|p| ((p.pid, p.created), EndOutcome::Protected)),
        );
        return out;
    }
    out.go = children_first(&alive);
    out
}

/// Deepest first (a process's parent, if it's in `procs` and older, comes after it); ties by pid.
fn children_first(procs: &[&Proc]) -> Vec<Id> {
    let by_pid: HashMap<u32, &Proc> = procs.iter().map(|p| (p.pid, *p)).collect();
    let depth = |p: &Proc| {
        let mut at = p;
        let mut d = 0usize;
        while let Some(up) = by_pid
            .get(&at.parent)
            .filter(|q| q.pid != at.pid && q.created <= at.created)
        {
            d += 1;
            at = up;
            if d > procs.len() {
                break;
            }
        }
        d
    };
    let mut order: Vec<(usize, Id)> = procs
        .iter()
        .map(|p| (depth(p), (p.pid, p.created)))
        .collect();
    order.sort_by(|a, b| b.0.cmp(&a.0).then(a.1 .0.cmp(&b.1 .0)));
    order.into_iter().map(|(_, id)| id).collect()
}

/// "End WhatsApp?" with the process count and memory, for the confirm box.
pub fn confirm_text(name: &str, processes: usize, bytes: u64) -> String {
    let count = if processes == 1 {
        "1 process".to_string()
    } else {
        format!("{processes} processes")
    };
    format!(
        "End {name}?\n\n{count}, {}. Cay asks it to close first; anything it hasn't saved may be lost.",
        size(bytes)
    )
}

/// 1.2 GB, 340 MB: as the panel shows it (`formatBytes` in `useHeavyProcesses.ts`).
pub fn size(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if mb >= 1024.0 {
        format!("{:.1} GB", mb / 1024.0)
    } else {
        format!("{} MB", mb.round().max(1.0) as u64)
    }
}

/// Counts for the log line (names and counts only).
pub fn tally(outcomes: &[(Id, EndOutcome)]) -> String {
    let mut counts: Vec<(EndOutcome, usize)> = Vec::new();
    for (_, o) in outcomes {
        match counts.iter_mut().find(|(k, _)| k == o) {
            Some((_, n)) => *n += 1,
            None => counts.push((*o, 1)),
        }
    }
    counts
        .iter()
        .map(|(o, n)| format!("{n} {o:?}"))
        .collect::<Vec<_>>()
        .join(", ")
}

// ---- Win32 ----

pub fn as_procs(raw: Vec<RawProcess>) -> Vec<Proc> {
    raw.into_iter()
        .map(|p| Proc {
            pid: p.pid,
            parent: p.parent,
            created: p.created,
            session: p.session,
            name: p.name,
            path: None,
            private_bytes: p.private_bytes,
        })
        .collect()
}

/// A fresh snapshot, as `Proc`s (paths aren't needed to check or protect).
pub fn now() -> Option<Vec<Proc>> {
    snapshot::read().map(as_procs)
}

/// The native confirm box: OK ends, Cancel (the default button) doesn't. No owner window: it never involves the
/// island, which must not take focus.
pub fn confirm(text: &str) -> bool {
    // SAFETY: both strings are valid for the call.
    unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(text),
            &HSTRING::from("Cay"),
            MB_OKCANCEL | MB_ICONWARNING | MB_DEFBUTTON2 | MB_SETFOREGROUND | MB_TOPMOST,
        ) == IDOK
    }
}

/// Asks the app to close, waits, then ends what's left. `go` comes from `check`, children first.
pub fn end(go: &[Id], own_hwnd: isize) -> Vec<(Id, EndOutcome)> {
    let pids: HashSet<u32> = go.iter().map(|id| id.0).collect();
    let windows: Vec<isize> = apps::top_level_windows(win32::from_raw(own_hwnd))
        .into_iter()
        .filter(|w| pids.contains(&w.pid))
        .map(|w| w.hwnd)
        .collect();

    let mut remaining: Vec<Id> = go.to_vec();
    let mut out = Vec::new();
    if !windows.is_empty() {
        for &hwnd in &windows {
            let h = win32::from_raw(hwnd);
            // SAFETY: IsWindow takes any value; the pid check makes sure the handle wasn't reused.
            if unsafe { IsWindow(Some(h)) }.as_bool() && pids.contains(&win32::window_pid(h)) {
                // SAFETY: posting WM_CLOSE to a live window; a failure (an elevated app: UIPI) is harmless.
                let _ = unsafe { PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0)) };
            }
        }
        let deadline = Instant::now() + CLOSE_WAIT;
        loop {
            if let Some(now) = snapshot::read() {
                let running: HashSet<Id> = now.iter().map(|p| (p.pid, p.created)).collect();
                remaining.retain(|id| {
                    let alive = running.contains(id);
                    if !alive {
                        out.push((*id, EndOutcome::Closed));
                    }
                    alive
                });
            }
            if remaining.is_empty() || Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(POLL);
        }
        // A window still there but disabled is showing a dialog ("Save changes?"): leave the app to it.
        let asking = windows.iter().any(|&hwnd| {
            let h = win32::from_raw(hwnd);
            // SAFETY: as above; IsWindowEnabled takes any window handle.
            let disabled = unsafe { IsWindow(Some(h)).as_bool() && !IsWindowEnabled(h).as_bool() };
            disabled && pids.contains(&win32::window_pid(h))
        });
        if asking {
            out.extend(remaining.iter().map(|id| (*id, EndOutcome::Prompted)));
            return out;
        }
    }
    for id in remaining {
        out.push((id, terminate(id)));
    }
    out
}

/// Ends one process, after checking through its own handle that it's still the one listed.
fn terminate((pid, created): Id) -> EndOutcome {
    // SAFETY: the handle is closed by the guard; every out-pointer is a valid local.
    unsafe {
        let access = PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE;
        let process = match OpenProcess(access, false, pid) {
            Ok(h) => Handle(h),
            Err(e) if e.code() == E_ACCESSDENIED => return EndOutcome::AccessDenied,
            // The pid no longer exists: it exited by itself (often a helper after its app closed).
            Err(e) if e.code() == ERROR_INVALID_PARAMETER.to_hresult() => {
                return EndOutcome::Closed
            }
            Err(e) => {
                log::warn!("end: OpenProcess failed: {e}");
                return EndOutcome::Failed;
            }
        };
        // The handle pins this process, so the creation time can't change between the check and the call.
        let (mut creation, mut exit, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        if GetProcessTimes(process.0, &mut creation, &mut exit, &mut kernel, &mut user).is_err() {
            return EndOutcome::Failed;
        }
        if ticks(creation) != created {
            return EndOutcome::Changed;
        }
        let mut code = 0u32;
        if GetExitCodeProcess(process.0, &mut code).is_ok() && code != STILL_ACTIVE.0 as u32 {
            return EndOutcome::Closed;
        }
        match TerminateProcess(process.0, 1) {
            Ok(()) => {
                if WaitForSingleObject(process.0, EXIT_WAIT_MS) != WAIT_OBJECT_0 {
                    log::warn!("end: a terminated process hasn't exited after {EXIT_WAIT_MS} ms");
                }
                EndOutcome::Ended
            }
            Err(e) if e.code() == E_ACCESSDENIED => EndOutcome::AccessDenied,
            Err(e) => {
                log::warn!("end: TerminateProcess failed: {e}");
                EndOutcome::Failed
            }
        }
    }
}

fn ticks(t: FILETIME) -> u64 {
    (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, parent: u32, created: u64, name: &str) -> Proc {
        Proc {
            pid,
            parent,
            created,
            session: 1,
            name: name.into(),
            path: None,
            private_bytes: 1,
        }
    }

    const OWN: u32 = 9999;

    #[test]
    fn only_listed_and_still_running_processes_go_children_first() {
        // chrome 100 → renderer 101 → its child 102; crashpad 103 under 100.
        let now = vec![
            p(100, 1, 10, "chrome.exe"),
            p(101, 100, 11, "chrome.exe"),
            p(102, 101, 12, "chrome.exe"),
            p(103, 100, 13, "crashpad_handler.exe"),
        ];
        let listed = [(100, 10), (101, 11), (102, 12), (103, 13)];
        let c = check(
            &[(100, 10), (103, 13), (101, 11), (102, 12)],
            &listed,
            &now,
            OWN,
        );
        assert!(c.settled.is_empty());
        assert_eq!(c.go, vec![(102, 12), (101, 11), (103, 13), (100, 10)]);
    }

    #[test]
    fn gone_reused_and_unlisted_pids_are_settled_not_ended() {
        let now = vec![p(100, 1, 10, "notepad.exe"), p(200, 1, 99, "notepad.exe")];
        // 200 was listed with creation time 20; pid 200 is now another process (created 99).
        let listed = [(100, 10), (200, 20), (300, 30)];
        let c = check(
            &[(100, 10), (200, 20), (300, 30), (400, 40)],
            &listed,
            &now,
            OWN,
        );
        assert_eq!(c.go, vec![(100, 10)]);
        assert_eq!(
            c.settled,
            vec![
                ((200, 20), EndOutcome::Changed),
                ((300, 30), EndOutcome::NotFound),
                ((400, 40), EndOutcome::Changed),
            ]
        );
    }

    #[test]
    fn duplicates_are_checked_once() {
        let now = vec![p(100, 1, 10, "notepad.exe")];
        let c = check(&[(100, 10), (100, 10)], &[(100, 10)], &now, OWN);
        assert_eq!(c.go, vec![(100, 10)]);
    }

    #[test]
    fn one_protected_member_refuses_the_whole_app() {
        let now = vec![p(100, 1, 10, "app.exe"), p(101, 100, 11, "conhost.exe")];
        let listed = [(100, 10), (101, 11)];
        let c = check(&listed, &listed, &now, OWN);
        assert!(c.go.is_empty());
        assert_eq!(
            c.settled,
            vec![
                ((100, 10), EndOutcome::Protected),
                ((101, 11), EndOutcome::Protected)
            ]
        );
    }

    #[test]
    fn cay_session_zero_and_system_are_never_ended() {
        let mut service = p(500, 1, 50, "someservice.exe");
        service.session = 0;
        for proc in [p(OWN, 1, 5, "cay.exe"), service, p(4, 0, 1, "System")] {
            let id = (proc.pid, proc.created);
            let c = check(&[id], &[id], std::slice::from_ref(&proc), OWN);
            assert!(c.go.is_empty(), "{}", proc.name);
            assert_eq!(c.settled, vec![(id, EndOutcome::Protected)]);
        }
    }

    #[test]
    fn a_reused_parent_pid_does_not_make_a_child() {
        // 100's parent pid 50 is now a newer process: 100 isn't its child, so ties go by pid.
        let now = vec![p(50, 1, 90, "app.exe"), p(100, 50, 10, "app.exe")];
        let listed = [(50, 90), (100, 10)];
        let c = check(&listed, &listed, &now, OWN);
        assert_eq!(c.go, vec![(50, 90), (100, 10)]);
    }

    #[test]
    fn confirm_text_and_sizes() {
        assert_eq!(
            confirm_text("WhatsApp", 3, 1288 * 1024 * 1024),
            "End WhatsApp?\n\n3 processes, 1.3 GB. Cay asks it to close first; anything it hasn't saved may be lost."
        );
        assert!(confirm_text("Notepad", 1, 20 << 20).contains("1 process, 20 MB."));
        assert_eq!(size(1), "1 MB");
        assert_eq!(size(340 << 20), "340 MB");
    }

    #[test]
    fn tally_counts_each_outcome() {
        let o = [
            ((1, 1), EndOutcome::Closed),
            ((2, 2), EndOutcome::Ended),
            ((3, 3), EndOutcome::Closed),
        ];
        assert_eq!(tally(&o), "2 Closed, 1 Ended");
    }

    /// Live: ends a `cmd` → `ping` tree this test started (no window, nobody else's process).
    /// `cargo test --lib -- --ignored end_a_tree`
    #[test]
    #[ignore]
    fn end_a_tree_this_test_started() {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut child = std::process::Command::new("cmd.exe")
            .args(["/c", "ping -n 60 127.0.0.1 >NUL"])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .expect("cmd starts");
        let cmd_pid = child.id();
        // Give cmd a moment to start ping.
        let mut tree = Vec::new();
        for _ in 0..50 {
            let snap = now().expect("snapshot");
            tree = snap
                .iter()
                .filter(|p| p.pid == cmd_pid || p.parent == cmd_pid)
                .cloned()
                .collect::<Vec<_>>();
            if tree.iter().any(|p| p.name.eq_ignore_ascii_case("ping.exe")) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let ping = tree
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case("ping.exe"))
            .expect("ping started under cmd");
        let ids: Vec<Id> = tree
            .iter()
            .filter(|p| p.pid == cmd_pid || p.pid == ping.pid)
            .map(|p| (p.pid, p.created))
            .collect();
        let snap = now().expect("snapshot");
        let checked = check(&ids, &ids, &snap, std::process::id());
        assert!(checked.settled.is_empty(), "{:?}", checked.settled);
        assert_eq!(
            checked.go.first().map(|id| id.0),
            Some(ping.pid),
            "ping first"
        );
        let outcomes = end(&checked.go, 0);
        assert!(
            outcomes.iter().all(|(_, o)| *o == EndOutcome::Ended),
            "{outcomes:?}"
        );
        let status = child.wait().expect("cmd exits");
        assert!(!status.success());
        // A reused-pid check through the handle: the old creation time no longer matches anything.
        // (Closed: the pid is free; Changed: Windows already gave it to another process.)
        let again = terminate((ping.pid, ping.created));
        assert!(
            matches!(again, EndOutcome::Closed | EndOutcome::Changed),
            "{again:?}"
        );
    }
}
