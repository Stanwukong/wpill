//! The `brightness-watcher` thread: internal-panel brightness changes from WMI's `WmiMonitorBrightnessEvent`.
//!
//! Semi-synchronous: the thread pulls events with a timeout instead of WMI calling a sink, so no COM object
//! has to be implemented and nothing calls into this process from WMI's threads. The timeout is also how it
//! notices the stop flag. External (DDC/CI) monitors don't raise this event.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use windows::core::{w, Result, BSTR, HRESULT, PCWSTR};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoSetProxyBlanket, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_MULTITHREADED, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE,
};
use windows::Win32::System::Rpc::{RPC_C_AUTHN_WINNT, RPC_C_AUTHZ_NONE};
use windows::Win32::System::Variant::{VariantClear, VariantToInt32, VARIANT};
use windows::Win32::System::Wmi::{
    IWbemLocator, WbemLocator, WBEM_FLAG_FORWARD_ONLY, WBEM_FLAG_RETURN_IMMEDIATELY,
    WBEM_S_TIMEDOUT,
};

use super::{Signal, Watcher};
use crate::error::AppResult;
use crate::ipc::{Activity, BrightnessActivity};

/// How long one pull waits; bounds how quickly the thread sees the stop flag.
const PULL_TIMEOUT_MS: i32 = 500;

pub fn start(out: Sender<Signal>) -> AppResult<Watcher> {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    Watcher::spawn(
        "brightness-watcher",
        move || run(&out, &flag),
        move || stop.store(true, Ordering::Relaxed),
    )
}

fn run(out: &Sender<Signal>, stop: &AtomicBool) {
    // SAFETY: called once at the top of this thread; balanced by CoUninitialize below.
    let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    match watch(out, stop) {
        Ok(()) => log::info!("brightness watcher stopped"),
        Err(e) => log::warn!("brightness activities unavailable: {e}"),
    }
    if com.is_ok() {
        // SAFETY: matches the successful CoInitializeEx; every COM object is dropped by now.
        unsafe { CoUninitialize() };
    }
}

fn watch(out: &Sender<Signal>, stop: &AtomicBool) -> Result<()> {
    // SAFETY: plain WMI calls; every object is a COM smart pointer released on drop, and the VARIANT is
    // cleared after reading.
    unsafe {
        let locator: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER)?;
        let services = locator.ConnectServer(
            &BSTR::from("ROOT\\WMI"),
            &BSTR::new(),
            &BSTR::new(),
            &BSTR::new(),
            0,
            &BSTR::new(),
            None,
        )?;
        CoSetProxyBlanket(
            &services,
            RPC_C_AUTHN_WINNT,
            RPC_C_AUTHZ_NONE,
            PCWSTR::null(),
            RPC_C_AUTHN_LEVEL_CALL,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOAC_NONE,
        )?;
        let events = services.ExecNotificationQuery(
            &BSTR::from("WQL"),
            &BSTR::from("SELECT * FROM WmiMonitorBrightnessEvent"),
            WBEM_FLAG_RETURN_IMMEDIATELY | WBEM_FLAG_FORWARD_ONLY,
            None,
        )?;
        log::debug!("watching brightness");

        let mut last = None;
        while !stop.load(Ordering::Relaxed) {
            let mut objects = [None];
            let mut returned = 0;
            let hr = events.Next(PULL_TIMEOUT_MS, &mut objects, &mut returned);
            if hr == HRESULT(WBEM_S_TIMEDOUT.0) {
                continue;
            }
            hr.ok()?;
            let Some(event) = objects[0].take() else {
                continue;
            };
            let mut value = VARIANT::default();
            event.Get(w!("Brightness"), 0, &mut value, None, None)?;
            let level = VariantToInt32(&value);
            let _ = VariantClear(&mut value);
            let level = level?.clamp(0, 100) as u8;
            // Each key press can raise the same value twice; only changes are activities.
            if last.replace(level) != Some(level) {
                let _ = out.send(Activity::Brightness(BrightnessActivity { level }).into());
            }
        }
    }
    Ok(())
}
