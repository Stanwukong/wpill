//! Small Win32 helpers shared by feature modules.

use std::ffi::c_void;

use windows::core::{HSTRING, PWSTR};
use windows::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL, HWND};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{
    GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
    TOKEN_USER,
};
use windows::Win32::System::Threading::{CreateEventW, GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetWindowThreadProcessId};

/// A kernel handle closed on drop. Events are safe to signal from any thread.
pub struct Handle(pub HANDLE);

// SAFETY: kernel handles are process-wide values; SetEvent/CloseHandle are thread-safe.
unsafe impl Send for Handle {}
// SAFETY: as above; the only shared operation is SetEvent.
unsafe impl Sync for Handle {}

impl Handle {
    /// An unnamed manual-reset event.
    pub fn event() -> windows::core::Result<Self> {
        // SAFETY: creates an event, closed on drop.
        Ok(Self(unsafe { CreateEventW(None, true, false, None)? }))
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: we own this handle.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// The current user's SID as a string (`S-1-5-21-…`), for per-user pipe names and DACLs.
pub fn user_sid() -> Option<String> {
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

/// A security descriptor that grants everything to one SID and nothing to anyone else (protected DACL,
/// no inherited entries). Freed on drop; `attributes()` borrows it for a create call.
pub struct UserOnly(PSECURITY_DESCRIPTOR);

impl UserOnly {
    pub fn new(sid: &str) -> windows::core::Result<Self> {
        let sddl = HSTRING::from(format!("D:P(A;;GA;;;{sid})"));
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        // SAFETY: the conversion allocates `descriptor`, which drop frees with LocalFree.
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                &sddl,
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )?;
        }
        Ok(Self(descriptor))
    }

    pub fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0 .0,
            bInheritHandle: false.into(),
        }
    }
}

impl Drop for UserOnly {
    fn drop(&mut self) {
        // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW in `new`.
        let _ = unsafe { LocalFree(Some(HLOCAL(self.0 .0))) };
    }
}

/// HWNDs aren't `Send`; threads and IPC carry them as `isize`.
pub fn raw(hwnd: HWND) -> isize {
    hwnd.0 as isize
}

pub fn from_raw(value: isize) -> HWND {
    HWND(value as *mut c_void)
}

pub fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    // SAFETY: buf is a valid writable buffer; an invalid hwnd just returns 0.
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..len.max(0) as usize])
}

pub fn window_pid(hwnd: HWND) -> u32 {
    let mut pid = 0u32;
    // SAFETY: pid outlives the call; an invalid hwnd leaves it at 0.
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    pid
}
