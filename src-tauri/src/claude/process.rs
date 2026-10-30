//! Process helpers: the forwarder's ancestor chain, and checking that a recorded process still exists.

use windows::Win32::Foundation::{FILETIME, STILL_ACTIVE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, GetExitCodeProcess, GetProcessTimes, OpenProcess,
    PROCESS_QUERY_LIMITED_INFORMATION,
};

use super::protocol::Proc;
use crate::win32::Handle;

/// Enough to get from a hook's shell past Claude Code to the terminal or editor.
const MAX_DEPTH: usize = 12;

/// The calling process's ancestors, nearest first. Stops at a parent created after its child: that pid
/// was reused by an unrelated process after the real parent exited.
pub fn ancestors() -> Vec<Proc> {
    let table = snapshot();
    // SAFETY: no preconditions.
    let mut pid = unsafe { GetCurrentProcessId() };
    let Some(mut child_created) = created(pid) else {
        return Vec::new();
    };
    let mut chain = Vec::new();
    while chain.len() < MAX_DEPTH {
        let Some(&(parent, _)) = table.iter().find(|(p, _)| *p == pid).map(|(_, e)| e) else {
            break;
        };
        let Some((_, (_, exe))) = table.iter().find(|(p, _)| *p == parent) else {
            break;
        };
        let Some(parent_created) = created(parent).filter(|&c| c <= child_created) else {
            break;
        };
        chain.push(Proc {
            pid: parent,
            exe: exe.clone(),
            created: parent_created,
        });
        pid = parent;
        child_created = parent_created;
    }
    chain
}

/// `(pid, (parent pid, exe file name))` for every process.
fn snapshot() -> Vec<(u32, (u32, String))> {
    let mut out = Vec::new();
    // SAFETY: the snapshot handle is closed by the guard; entry.dwSize is set as the API requires.
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return out;
        };
        let snap = Handle(snap);
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut ok = Process32FirstW(snap.0, &mut entry).is_ok();
        while ok {
            let len = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            let exe = String::from_utf16_lossy(&entry.szExeFile[..len]);
            out.push((entry.th32ProcessID, (entry.th32ParentProcessID, exe)));
            ok = Process32NextW(snap.0, &mut entry).is_ok();
        }
    }
    out
}

/// Creation time in FILETIME ticks, or `None` if the process is gone or can't be opened.
pub fn created(pid: u32) -> Option<u64> {
    // SAFETY: the handle is closed by the guard; the FILETIMEs are valid out-pointers.
    unsafe {
        let process = Handle(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?);
        let (mut creation, mut exit, mut kernel, mut user) = Default::default();
        GetProcessTimes(process.0, &mut creation, &mut exit, &mut kernel, &mut user).ok()?;
        Some(ticks(creation))
    }
}

/// Whether `p` is still running (same pid *and* creation time).
pub fn alive(p: &Proc) -> bool {
    // SAFETY: as in `created`; GetExitCodeProcess writes one u32.
    unsafe {
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, p.pid) else {
            return false;
        };
        let process = Handle(process);
        let mut code = 0u32;
        if GetExitCodeProcess(process.0, &mut code).is_err() || code != STILL_ACTIVE.0 as u32 {
            return false;
        }
        let (mut creation, mut exit, mut kernel, mut user) = Default::default();
        GetProcessTimes(process.0, &mut creation, &mut exit, &mut kernel, &mut user).is_ok()
            && ticks(creation) == p.created
    }
}

fn ticks(t: FILETIME) -> u64 {
    (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime)
}
