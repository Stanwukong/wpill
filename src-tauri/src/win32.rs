//! Small Win32 helpers shared by feature modules.

use std::ffi::c_void;

use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND};
use windows::Win32::System::Threading::CreateEventW;
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
