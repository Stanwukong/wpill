//! The agent pipe, `\\.\pipe\topbar-agent-<user SID>`: duplex, message mode, one instance per bridge, locked to
//! the current user like 2.6's (D17/D20). Both ends use overlapped handles so a blocked read never holds up a
//! write (an `approve` answer goes out from a command while the connection thread waits for the next message).

use std::sync::Mutex;

use windows::core::{Error, Result, HSTRING};
use windows::Win32::Foundation::{
    ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_MORE_DATA, ERROR_NO_DATA, ERROR_PIPE_BUSY,
    ERROR_PIPE_CONNECTED, ERROR_PIPE_NOT_CONNECTED, ERROR_TIMEOUT, GENERIC_READ, GENERIC_WRITE,
    HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED,
    FILE_SHARE_NONE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, SetNamedPipeHandleState, WaitNamedPipeW,
    PIPE_READMODE_MESSAGE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE, PIPE_UNLIMITED_INSTANCES,
    PIPE_WAIT,
};
use windows::Win32::System::Threading::{ResetEvent, WaitForMultipleObjects, INFINITE};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

use crate::win32::{self, Handle, UserOnly};

/// Largest message either side accepts. Approval requests carry the tool input (a `Write` holds the file).
pub const MAX_MESSAGE: usize = 4 * 1024 * 1024;
const CHUNK: usize = 64 * 1024;
const WRITE_TIMEOUT_MS: u32 = 5000;
const CONNECT_BUDGET_MS: u32 = 2000;

pub fn name() -> Option<HSTRING> {
    Some(HSTRING::from(format!(
        r"\\.\pipe\topbar-agent-{}",
        win32::user_sid()?
    )))
}

pub enum Read {
    Message(Vec<u8>),
    /// Longer than `MAX_MESSAGE`; drained and dropped.
    TooLong,
    /// The other end went away.
    Closed,
    /// One of the cancel events fired.
    Cancelled,
    Timeout,
}

/// One end of a connection.
pub struct Pipe {
    handle: Handle,
    read_io: Handle,
    write: Mutex<Handle>,
}

impl Pipe {
    fn new(handle: HANDLE) -> Result<Self> {
        Ok(Self {
            handle: Handle(handle),
            read_io: Handle::event()?,
            write: Mutex::new(Handle::event()?),
        })
    }

    /// Server side: a new instance (the first one claims the name), not yet connected.
    pub fn create(first: bool) -> Result<Self> {
        let name = name().ok_or_else(Error::from_thread)?;
        let sid = win32::user_sid().ok_or_else(Error::from_thread)?;
        let security = UserOnly::new(&sid)?;
        let attributes = security.attributes();
        let mut open = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED;
        if first {
            open |= FILE_FLAG_FIRST_PIPE_INSTANCE;
        }
        // SAFETY: `attributes` borrows `security`, which outlives the call.
        let handle = unsafe {
            CreateNamedPipeW(
                &name,
                open,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                CHUNK as u32,
                CHUNK as u32,
                0,
                Some(&attributes),
            )
        };
        if handle.is_invalid() {
            return Err(Error::from_thread());
        }
        Self::new(handle)
    }

    /// Server side: waits for a client. `Ok(false)` if `stop` fired first.
    pub fn accept(&self, stop: HANDLE) -> Result<bool> {
        let mut overlapped = self.overlapped(&self.read_io);
        // SAFETY: `overlapped` lives on this frame; the operation completes or is cancelled and awaited here.
        unsafe {
            match ConnectNamedPipe(self.handle.0, Some(&mut overlapped)) {
                Ok(()) => return Ok(true),
                Err(e) if e.code() == ERROR_PIPE_CONNECTED.to_hresult() => return Ok(true),
                Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => {}
                Err(e) => return Err(e),
            }
            let waited = WaitForMultipleObjects(&[self.read_io.0, stop], false, INFINITE);
            if waited == WAIT_OBJECT_0 {
                let mut n = 0;
                GetOverlappedResult(self.handle.0, &overlapped, &mut n, false)?;
                return Ok(true);
            }
            let _ = CancelIoEx(self.handle.0, Some(&overlapped));
            let mut n = 0;
            // A client may have connected just before the cancel.
            Ok(GetOverlappedResult(self.handle.0, &overlapped, &mut n, true).is_ok())
        }
    }

    /// Client side (the bridge): connects, waiting a little if every instance is busy.
    pub fn open() -> Result<Self> {
        let name = name().ok_or_else(Error::from_thread)?;
        let mut waited = 0;
        let handle = loop {
            // SAFETY: opens the pipe; the handle is owned by the returned `Pipe`.
            let opened = unsafe {
                CreateFileW(
                    &name,
                    (GENERIC_READ | GENERIC_WRITE).0,
                    FILE_SHARE_NONE,
                    None,
                    OPEN_EXISTING,
                    FILE_FLAG_OVERLAPPED,
                    None,
                )
            };
            match opened {
                Ok(handle) => break handle,
                Err(e)
                    if e.code() == ERROR_PIPE_BUSY.to_hresult() && waited < CONNECT_BUDGET_MS =>
                {
                    // SAFETY: waits on the pipe name; returns early when an instance frees up.
                    let _ = unsafe { WaitNamedPipeW(&name, 200) };
                    waited += 200;
                }
                Err(e) => return Err(e),
            }
        };
        let pipe = Self::new(handle)?;
        let mode = PIPE_READMODE_MESSAGE;
        // SAFETY: our pipe handle; `mode` outlives the call.
        unsafe { SetNamedPipeHandleState(pipe.handle.0, Some(&mode), None, None)? };
        Ok(pipe)
    }

    /// Reads one whole message, waiting up to `timeout_ms` (or `INFINITE`) or until a cancel event fires.
    pub fn read(&self, cancel: &[HANDLE], timeout_ms: u32) -> Result<Read> {
        let mut message = Vec::new();
        let mut too_long = false;
        let mut chunk = vec![0u8; CHUNK];
        let mut waits = vec![self.read_io.0];
        waits.extend_from_slice(cancel);
        loop {
            let mut overlapped = self.overlapped(&self.read_io);
            let mut n = 0u32;
            // SAFETY: `chunk` and `overlapped` outlive the read, which completes or is cancelled and awaited.
            let done = unsafe {
                match ReadFile(self.handle.0, Some(&mut chunk), None, Some(&mut overlapped)) {
                    // Done at once (the message was already waiting): the count comes from the overlapped
                    // result, since no byte-count pointer is passed.
                    Ok(()) => GetOverlappedResult(self.handle.0, &overlapped, &mut n, false),
                    Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => {
                        let waited = WaitForMultipleObjects(&waits, false, timeout_ms);
                        if waited != WAIT_OBJECT_0 {
                            let _ = CancelIoEx(self.handle.0, Some(&overlapped));
                            let _ = GetOverlappedResult(self.handle.0, &overlapped, &mut n, true);
                            return Ok(if waited == WAIT_TIMEOUT {
                                Read::Timeout
                            } else {
                                Read::Cancelled
                            });
                        }
                        GetOverlappedResult(self.handle.0, &overlapped, &mut n, false)
                    }
                    Err(e) => Err(e),
                }
            };
            let more = match done {
                Ok(()) => false,
                Err(e) if e.code() == ERROR_MORE_DATA.to_hresult() => {
                    // SAFETY: as above; a partial read reports its byte count through the overlapped result.
                    let _ =
                        unsafe { GetOverlappedResult(self.handle.0, &overlapped, &mut n, false) };
                    true
                }
                Err(e) if is_closed(&e) => return Ok(Read::Closed),
                Err(e) => return Err(e),
            };
            if !too_long {
                if message.len() + n as usize > MAX_MESSAGE {
                    too_long = true;
                    message = Vec::new();
                } else {
                    message.extend_from_slice(&chunk[..n as usize]);
                }
            }
            if !more {
                return Ok(if too_long {
                    Read::TooLong
                } else {
                    Read::Message(message)
                });
            }
        }
    }

    /// Writes one message. Safe from any thread; writes are serialized.
    pub fn write(&self, bytes: &[u8]) -> Result<()> {
        let event = self.write.lock().unwrap_or_else(|e| e.into_inner());
        let mut overlapped = self.overlapped(&event);
        let mut n = 0u32;
        // SAFETY: `bytes` and `overlapped` outlive the write, which completes or is cancelled and awaited.
        unsafe {
            match WriteFile(self.handle.0, Some(bytes), None, Some(&mut overlapped)) {
                Ok(()) => Ok(()),
                Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => {
                    let waited = WaitForMultipleObjects(&[event.0], false, WRITE_TIMEOUT_MS);
                    if waited != WAIT_OBJECT_0 {
                        let _ = CancelIoEx(self.handle.0, Some(&overlapped));
                        let _ = GetOverlappedResult(self.handle.0, &overlapped, &mut n, true);
                        return Err(Error::from_hresult(ERROR_TIMEOUT.to_hresult()));
                    }
                    GetOverlappedResult(self.handle.0, &overlapped, &mut n, false)
                }
                Err(e) => Err(e),
            }
        }
    }

    fn overlapped(&self, event: &Handle) -> OVERLAPPED {
        // SAFETY: resetting our own manual-reset event before reusing it.
        let _ = unsafe { ResetEvent(event.0) };
        OVERLAPPED {
            hEvent: event.0,
            ..Default::default()
        }
    }
}

fn is_closed(e: &Error) -> bool {
    [ERROR_BROKEN_PIPE, ERROR_PIPE_NOT_CONNECTED, ERROR_NO_DATA]
        .iter()
        .any(|c| e.code() == c.to_hresult())
}
