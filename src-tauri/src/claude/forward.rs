//! `topbar.exe hook`: run by Claude Code as a hook command. Reads the hook's JSON from stdin, keeps the few
//! fields TopBar uses, adds its ancestor chain, writes one message to the pipe and exits.
//!
//! It runs before Tauri (checked in `main.rs`), never prints, and always exits 0: if TopBar isn't running
//! the pipe doesn't exist and it returns at once. Every wait is bounded so a hook never hangs Claude Code.

use std::io::Read;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use windows::core::{Result, HSTRING};
use windows::Win32::Foundation::{ERROR_PIPE_BUSY, GENERIC_WRITE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, WriteFile, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_NONE, OPEN_EXISTING,
};
use windows::Win32::System::Pipes::WaitNamedPipeW;

use super::{pipe, process, protocol};
use crate::win32::Handle;

/// Hook input can carry Claude's whole last message; keep up to this much and discard the rest.
const MAX_STDIN: usize = 8 * 1024 * 1024;
const STDIN_TIMEOUT: Duration = Duration::from_millis(1000);
/// Total time to get past a busy pipe (another forwarder connected).
const CONNECT_BUDGET: Duration = Duration::from_millis(1000);
const BUSY_WAIT_MS: u32 = 200;

pub fn run() {
    let at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    let Some(stdin) = read_stdin() else {
        return;
    };
    let Some(envelope) = protocol::envelope(&stdin, at_ms, process::ancestors()) else {
        return;
    };
    let Ok(bytes) = serde_json::to_vec(&envelope) else {
        return;
    };
    if bytes.len() > protocol::MAX_MESSAGE {
        return;
    }
    if let Some(name) = pipe::name() {
        let _ = send(&name, &bytes);
    }
}

/// Stdin to EOF on a helper thread, so a writer that never closes it can't keep us alive.
fn read_stdin() -> Option<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut kept = Vec::new();
        let mut chunk = vec![0u8; 64 * 1024];
        let mut stdin = std::io::stdin().lock();
        loop {
            match stdin.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    let room = MAX_STDIN.saturating_sub(kept.len());
                    kept.extend_from_slice(&chunk[..n.min(room)]);
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        let _ = tx.send(kept);
    });
    rx.recv_timeout(STDIN_TIMEOUT).ok()
}

fn send(name: &HSTRING, bytes: &[u8]) -> Result<()> {
    let deadline = Instant::now() + CONNECT_BUDGET;
    loop {
        // SAFETY: opens the pipe for writing; the handle is closed by the guard.
        let opened = unsafe {
            CreateFileW(
                name,
                GENERIC_WRITE.0,
                FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            )
        };
        match opened {
            Ok(pipe) => {
                let pipe = Handle(pipe);
                let mut written = 0u32;
                // SAFETY: one write of the whole message (message-mode pipe); `bytes` outlives the call.
                return unsafe { WriteFile(pipe.0, Some(bytes), Some(&mut written), None) };
            }
            Err(e) if e.code() == ERROR_PIPE_BUSY.to_hresult() && Instant::now() < deadline => {
                // SAFETY: waits on the pipe name; returns early when an instance frees up.
                let _ = unsafe { WaitNamedPipeW(name, BUSY_WAIT_MS) };
            }
            // Not found: TopBar isn't running.
            Err(e) => return Err(e),
        }
    }
}
