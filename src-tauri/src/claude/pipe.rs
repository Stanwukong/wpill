//! The per-user named pipe the hook forwarder writes to, and the listener's server loop.
//!
//! One pipe instance, one message per connection. Connects and reads are overlapped and waited on together
//! with the stop event, so shutdown never leaves a `ConnectNamedPipe` pending, and a client that connects
//! but never writes can't wedge the listener.

use windows::core::{Error, Result, HSTRING, PWSTR};
use windows::Win32::Foundation::{
    LocalFree, ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_MORE_DATA, ERROR_PIPE_CONNECTED, HLOCAL,
    WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{
    GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
    TOKEN_USER,
};
use windows::Win32::Storage::FileSystem::{
    ReadFile, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_INBOUND,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_MESSAGE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE, PIPE_WAIT,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcessToken, WaitForMultipleObjects,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

use super::protocol::MAX_MESSAGE;
use crate::win32::Handle;

/// A connected client gets this long to deliver its message.
const READ_TIMEOUT_MS: u32 = 500;

/// `\\.\pipe\topbar-claude-<user SID>`: per user, so sessions of other users (fast user switching, RDP)
/// never see each other's pipe.
pub fn name() -> Option<HSTRING> {
    Some(HSTRING::from(format!(
        r"\\.\pipe\topbar-claude-{}",
        user_sid()?
    )))
}

fn user_sid() -> Option<String> {
    // SAFETY: the token handle is closed by the guard; `buf` is sized by the first call and holds a
    // TOKEN_USER whose SID points into it; the string from ConvertSidToStringSidW is freed with LocalFree.
    unsafe {
        let mut token = Default::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).ok()?;
        let token = Handle(token);
        let mut len = 0u32;
        let _ = GetTokenInformation(token.0, TokenUser, None, 0, &mut len);
        // u64s keep the TOKEN_USER aligned.
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        GetTokenInformation(
            token.0,
            TokenUser,
            Some(buf.as_mut_ptr().cast()),
            len,
            &mut len,
        )
        .ok()?;
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut text = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut text).ok()?;
        let sid = text.to_string().ok();
        let _ = LocalFree(Some(HLOCAL(text.0.cast())));
        sid
    }
}

pub struct Server {
    pipe: Handle,
    io: Handle,
}

/// What a wait on the pipe ended with.
pub enum Next {
    Message(Vec<u8>),
    /// A client connected but sent nothing usable (oversized, timed out, closed early).
    Nothing,
    /// The timeout passed with no client: time for housekeeping.
    Idle,
    Stop,
}

impl Server {
    /// Creates the only instance of the pipe, readable and writable by the current user alone. Fails if
    /// another process already owns the name (`FILE_FLAG_FIRST_PIPE_INSTANCE`).
    pub fn create() -> Result<Self> {
        let name = name().ok_or_else(Error::from_thread)?;
        let sid = user_sid().ok_or_else(Error::from_thread)?;
        // Protected DACL: generic all to this user's SID only; no inherited entries.
        let sddl = HSTRING::from(format!("D:P(A;;GA;;;{sid})"));
        // SAFETY: `descriptor` is allocated by the conversion and freed with LocalFree after the pipe is
        // created; the attributes struct only borrows it for that call.
        let pipe = unsafe {
            let mut descriptor = PSECURITY_DESCRIPTOR::default();
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                &sddl,
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )?;
            let attributes = SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor.0,
                bInheritHandle: false.into(),
            };
            let pipe = CreateNamedPipeW(
                &name,
                PIPE_ACCESS_INBOUND | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                0,
                MAX_MESSAGE as u32,
                0,
                Some(&attributes),
            );
            let _ = LocalFree(Some(HLOCAL(descriptor.0)));
            if pipe.is_invalid() {
                return Err(Error::from_thread());
            }
            pipe
        };
        Ok(Self {
            pipe: Handle(pipe),
            io: Handle::event()?,
        })
    }

    /// Waits up to `idle_ms` for a client and reads its message.
    pub fn next(&self, stop: &Handle, idle_ms: u32) -> Result<Next> {
        let mut overlapped = self.overlapped();
        // SAFETY: `overlapped` lives on this frame and every path waits for the operation to finish (or
        // cancels it and waits) before returning.
        unsafe {
            match ConnectNamedPipe(self.pipe.0, Some(&mut overlapped)) {
                Ok(()) => {}
                Err(e) if e.code() == ERROR_PIPE_CONNECTED.to_hresult() => {}
                Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => {
                    let waited = WaitForMultipleObjects(&[self.io.0, stop.0], false, idle_ms);
                    if waited != WAIT_OBJECT_0 {
                        let _ = CancelIoEx(self.pipe.0, Some(&overlapped));
                        let mut n = 0;
                        // A client may have connected just before the cancel; serve it.
                        if GetOverlappedResult(self.pipe.0, &overlapped, &mut n, true).is_err() {
                            return Ok(if waited == WAIT_TIMEOUT {
                                Next::Idle
                            } else {
                                Next::Stop
                            });
                        }
                    }
                }
                Err(e) => return Err(e),
            }
        }
        let next = self.read(stop);
        // SAFETY: drops the client so the single instance can accept the next one.
        let _ = unsafe { DisconnectNamedPipe(self.pipe.0) };
        next
    }

    fn read(&self, stop: &Handle) -> Result<Next> {
        // One byte more than allowed: a message that fills it is oversized.
        let mut buf = vec![0u8; MAX_MESSAGE + 1];
        let mut overlapped = self.overlapped();
        let mut n = 0u32;
        // SAFETY: `buf` and `overlapped` outlive the read, which completes or is cancelled and awaited here.
        unsafe {
            let started = ReadFile(self.pipe.0, Some(&mut buf), None, Some(&mut overlapped));
            match started {
                Ok(()) => {}
                Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => {
                    let waited =
                        WaitForMultipleObjects(&[self.io.0, stop.0], false, READ_TIMEOUT_MS);
                    if waited != WAIT_OBJECT_0 {
                        let _ = CancelIoEx(self.pipe.0, Some(&overlapped));
                        let _ = GetOverlappedResult(self.pipe.0, &overlapped, &mut n, true);
                        return Ok(if waited == WAIT_TIMEOUT {
                            Next::Nothing
                        } else {
                            Next::Stop
                        });
                    }
                }
                Err(e) if is_client_problem(&e) => return Ok(Next::Nothing),
                Err(e) => return Err(e),
            }
            match GetOverlappedResult(self.pipe.0, &overlapped, &mut n, false) {
                Ok(()) => {}
                Err(e) if is_client_problem(&e) => return Ok(Next::Nothing),
                Err(e) => return Err(e),
            }
        }
        let len = n as usize;
        if len == 0 || len > MAX_MESSAGE {
            return Ok(Next::Nothing);
        }
        buf.truncate(len);
        Ok(Next::Message(buf))
    }

    fn overlapped(&self) -> OVERLAPPED {
        // SAFETY: resetting our own manual-reset event before reusing it.
        let _ = unsafe { windows::Win32::System::Threading::ResetEvent(self.io.0) };
        OVERLAPPED {
            hEvent: self.io.0,
            ..Default::default()
        }
    }
}

/// Too long (the rest of the message is discarded with the client) or closed before writing.
fn is_client_problem(e: &Error) -> bool {
    e.code() == ERROR_MORE_DATA.to_hresult() || e.code() == ERROR_BROKEN_PIPE.to_hresult()
}
