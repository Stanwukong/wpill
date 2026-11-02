//! `cay.exe hook`: run by Claude Code as a hook command. Reads the hook's JSON from stdin, keeps the few
//! fields Cay uses, adds its ancestor chain, writes one message to the pipe and exits.
//!
//! A `PermissionRequest` (2.8a) goes to the approval pipe instead and waits there for your answer in the island; the
//! only thing the forwarder ever prints is that answer as hook output. With no answer (Cay closed, the request handed
//! back, anything unexpected) it prints nothing and Claude Code asks in the terminal as usual.
//!
//! It runs before Tauri (checked in `main.rs`) and always exits 0: if Cay isn't running the pipe doesn't exist and it
//! returns at once. Every wait is bounded so a hook never hangs Claude Code.

use std::io::{Read, Write};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use windows::core::{Result, HSTRING};
use windows::Win32::Foundation::{ERROR_PIPE_BUSY, GENERIC_WRITE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, WriteFile, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_NONE, OPEN_EXISTING,
};
use windows::Win32::System::Pipes::WaitNamedPipeW;

use super::{ask, asks, pipe, process, protocol};
use crate::duplex;
use crate::win32::Handle;

/// Hook input can carry Claude's whole last message; keep up to this much and discard the rest.
const MAX_STDIN: usize = 8 * 1024 * 1024;
const STDIN_TIMEOUT: Duration = Duration::from_millis(1000);
/// Total time to get past a busy pipe (another forwarder connected).
const CONNECT_BUDGET: Duration = Duration::from_millis(1000);
const BUSY_WAIT_MS: u32 = 200;
/// How long to wait for your answer: a little longer than Cay holds a request (`asks::hold_for`), so its hand-back
/// arrives first: 60 s for a permission, ~10 minutes for a question or a plan (2.8c). The hook entry's own timeout is
/// longer still (`hooks::EVENTS`).
fn answer_timeout_ms(tool: &str) -> u32 {
    let hold = asks::hold_for(tool).as_millis() as u32;
    hold + 5_000
}

pub fn run() {
    let at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    let Some(stdin) = read_stdin() else {
        return;
    };
    let chain = process::ancestors();
    if let Some(request) = ask::request(&stdin, at_ms, chain.clone()) {
        // The answer is built from the input as Claude Code sent it, not Cay's trimmed copy.
        let original = serde_json::from_slice::<serde_json::Value>(&stdin)
            .ok()
            .and_then(|v| v.get("tool_input").cloned())
            .unwrap_or_default();
        let output =
            hold(&request).and_then(|d| ask::hook_output(&d, &request.tool_name, &original));
        if let Some(output) = output {
            let mut stdout = std::io::stdout().lock();
            let _ = stdout.write_all(output.as_bytes());
            let _ = stdout.flush();
        }
        return;
    }
    let Some(envelope) = protocol::envelope(&stdin, at_ms, chain) else {
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

/// Sends the request and waits for Cay's decision; `None` means no decision.
fn hold(request: &ask::Request) -> Option<ask::Decision> {
    let bytes = serde_json::to_vec(request).ok()?;
    if bytes.len() > ask::MAX_REQUEST {
        return None;
    }
    let pipe = duplex::Pipe::open(&asks::name()?).ok()?;
    pipe.write(&bytes).ok()?;
    match pipe.read(&[], answer_timeout_ms(&request.tool_name)).ok()? {
        duplex::Read::Message(reply) => ask::decode_reply(&reply),
        _ => None,
    }
}

/// Stdin to EOF on a helper thread, so a writer that never closes it can't keep us alive.
pub(super) fn read_stdin() -> Option<Vec<u8>> {
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

pub(super) fn send(name: &HSTRING, bytes: &[u8]) -> Result<()> {
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
            // Not found: Cay isn't running.
            Err(e) => return Err(e),
        }
    }
}
