//! "The network came back": `NotifyNetworkConnectivityHintChange`, so the watcher refreshes after Wi-Fi returns
//! instead of waiting out its backoff. The callback runs on a thread-pool thread and only sends a message.

use std::ffi::c_void;
use std::sync::mpsc::Sender;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::NetworkManagement::IpHelper::{
    CancelMibChangeNotify2, NotifyNetworkConnectivityHintChange,
};
use windows::Win32::Networking::WinSock::{
    NetworkConnectivityLevelHintConstrainedInternetAccess,
    NetworkConnectivityLevelHintInternetAccess, NL_NETWORK_CONNECTIVITY_HINT,
};

use super::Wake;

/// Registered while alive; dropping it cancels the notification (waiting for a running callback), then frees
/// the sender the callback borrowed.
pub struct Hint {
    handle: HANDLE,
    context: *mut Sender<Wake>,
}

impl Hint {
    pub fn register(wake: Sender<Wake>) -> Option<Self> {
        let context = Box::into_raw(Box::new(wake));
        let mut handle = HANDLE::default();
        // SAFETY: `context` stays valid until Drop has cancelled the notification; no initial callback.
        let status = unsafe {
            NotifyNetworkConnectivityHintChange(
                Some(callback),
                Some(context.cast_const().cast()),
                false,
                &mut handle,
            )
        };
        if status.is_err() {
            log::debug!("google: no network notifications ({status:?})");
            // SAFETY: registration failed, so nothing else holds `context`.
            drop(unsafe { Box::from_raw(context) });
            return None;
        }
        Some(Self { handle, context })
    }
}

impl Drop for Hint {
    fn drop(&mut self) {
        // SAFETY: cancels our own registration; it waits for a callback in progress before returning.
        let _ = unsafe { CancelMibChangeNotify2(self.handle) };
        // SAFETY: the callback can no longer run, so we own `context` again.
        drop(unsafe { Box::from_raw(self.context) });
    }
}

unsafe extern "system" fn callback(context: *const c_void, hint: NL_NETWORK_CONNECTIVITY_HINT) {
    let level = hint.ConnectivityLevel;
    if level == NetworkConnectivityLevelHintInternetAccess
        || level == NetworkConnectivityLevelHintConstrainedInternetAccess
    {
        // SAFETY: `context` is the `Sender` from `register`, alive until `Hint` is dropped (which waits for us).
        let wake = unsafe { &*context.cast::<Sender<Wake>>() };
        let _ = wake.send(Wake::Network);
    }
}
