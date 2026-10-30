//! The `audio-watcher` thread: master volume/mute of the default output, and Bluetooth audio connects.
//!
//! Core Audio calls both callbacks on its own threads; they only forward a `Msg` here (they must not call
//! back into the device API). Each registration is a guard that unregisters on drop. A Bluetooth headset
//! has several endpoints (stereo and hands-free) in one device container, so connects are deduplicated by
//! container id.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use windows::core::{implement, Interface, Result, GUID, HSTRING, PCWSTR};
use windows::Devices::Enumeration::{DeviceInformation, DeviceInformationKind};
use windows::Foundation::IReference;
use windows::Win32::Devices::FunctionDiscovery::{
    PKEY_Device_ContainerId, PKEY_Device_FriendlyName,
};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::Endpoints::{
    IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl,
};
use windows::Win32::Media::Audio::{
    eConsole, eRender, EDataFlow, ERole, IMMDevice, IMMDeviceEnumerator, IMMEndpoint,
    IMMNotificationClient, IMMNotificationClient_Impl, MMDeviceEnumerator,
    AUDIO_VOLUME_NOTIFICATION_DATA, DEVICE_STATE, DEVICE_STATE_ACTIVE, DEVICE_STATE_NOTPRESENT,
    DEVICE_STATE_UNPLUGGED,
};
use windows::Win32::System::Com::StructuredStorage::{
    PropVariantClear, PropVariantToGUID, PropVariantToStringAlloc, PROPVARIANT,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_MULTITHREADED, STGM_READ,
};
use windows_collections::IIterable;

use super::Watcher;
use crate::error::AppResult;
use crate::ipc::{Activity, BluetoothActivity, VolumeActivity};

/// Undocumented, but what Settings shows: the battery level a hands-free device reports, on one of its nodes.
const BATTERY_KEY: &str = "{104EA319-6EE2-4701-BD47-8DDBF425BBE5} 2";
/// Headsets often report their battery a moment after connecting; look again while the activity shows.
const BATTERY_RECHECK: Duration = Duration::from_millis(1500);
const BLUETOOTH_PREFIXES: [&str; 4] = ["BTHENUM\\", "BTHLE\\", "BTHLEDEVICE\\", "BTHHFENUM\\"];

enum Msg {
    Volume { level: u8, muted: bool },
    DefaultChanged,
    StateChanged(String, DEVICE_STATE),
    Shutdown,
}

pub fn start(out: Sender<Activity>) -> AppResult<Watcher> {
    let (tx, rx) = mpsc::channel();
    let stop = tx.clone();
    Watcher::spawn(
        "audio-watcher",
        move || run(out, tx, rx),
        move || {
            let _ = stop.send(Msg::Shutdown);
        },
    )
}

fn run(out: Sender<Activity>, tx: Sender<Msg>, rx: Receiver<Msg>) {
    // SAFETY: called once at the top of this thread; balanced by CoUninitialize below.
    let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    match AudioWatcher::new(out, tx) {
        Ok(watcher) => watcher.run(rx),
        Err(e) => log::warn!("audio activities unavailable: {e}"),
    }
    if com.is_ok() {
        // SAFETY: matches the successful CoInitializeEx; every COM object is dropped by now.
        unsafe { CoUninitialize() };
    }
}

#[implement(IAudioEndpointVolumeCallback)]
struct VolumeEvents {
    tx: Sender<Msg>,
}

impl IAudioEndpointVolumeCallback_Impl for VolumeEvents_Impl {
    fn OnNotify(&self, data: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> Result<()> {
        // SAFETY: Core Audio passes a pointer that is valid for the duration of the call (or null).
        if let Some(d) = unsafe { data.as_ref() } {
            let _ = self.tx.send(Msg::Volume {
                level: percent(d.fMasterVolume),
                muted: d.bMuted.as_bool(),
            });
        }
        Ok(())
    }
}

#[implement(IMMNotificationClient)]
struct DeviceEvents {
    tx: Sender<Msg>,
}

impl IMMNotificationClient_Impl for DeviceEvents_Impl {
    fn OnDeviceStateChanged(&self, id: &PCWSTR, state: DEVICE_STATE) -> Result<()> {
        // SAFETY: id is a NUL-terminated string that is valid for the duration of the call.
        if let Ok(id) = unsafe { id.to_string() } {
            let _ = self.tx.send(Msg::StateChanged(id, state));
        }
        Ok(())
    }

    fn OnDeviceAdded(&self, _id: &PCWSTR) -> Result<()> {
        Ok(())
    }

    fn OnDeviceRemoved(&self, _id: &PCWSTR) -> Result<()> {
        Ok(())
    }

    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, _id: &PCWSTR) -> Result<()> {
        if flow == eRender && role == eConsole {
            let _ = self.tx.send(Msg::DefaultChanged);
        }
        Ok(())
    }

    fn OnPropertyValueChanged(&self, _id: &PCWSTR, _key: &PROPERTYKEY) -> Result<()> {
        Ok(())
    }
}

/// Device notifications, unregistered on drop.
struct DeviceSubscription {
    enumerator: IMMDeviceEnumerator,
    client: IMMNotificationClient,
}

impl Drop for DeviceSubscription {
    fn drop(&mut self) {
        // SAFETY: unregisters the client registered when this guard was built.
        if let Err(e) = unsafe {
            self.enumerator
                .UnregisterEndpointNotificationCallback(&self.client)
        } {
            log::debug!("removing the audio device callback failed: {e}");
        }
    }
}

/// Volume notifications for one endpoint, unregistered on drop.
struct VolumeSubscription {
    endpoint: IAudioEndpointVolume,
    callback: IAudioEndpointVolumeCallback,
    device: String,
    last: (u8, bool),
}

impl Drop for VolumeSubscription {
    fn drop(&mut self) {
        // SAFETY: unregisters the callback registered when this guard was built.
        if let Err(e) = unsafe { self.endpoint.UnregisterControlChangeNotify(&self.callback) } {
            log::debug!("removing the volume callback failed: {e}");
        }
    }
}

struct AudioWatcher {
    out: Sender<Activity>,
    tx: Sender<Msg>,
    enumerator: IMMDeviceEnumerator,
    volume: Option<VolumeSubscription>,
    /// Container id → connected, so a headset's endpoints report once.
    bluetooth: HashMap<String, bool>,
    /// Containers already found not to be Bluetooth.
    wired: HashSet<String>,
    battery_checks: Vec<(Instant, BluetoothActivity)>,
    _devices: DeviceSubscription,
}

impl AudioWatcher {
    fn new(out: Sender<Activity>, tx: Sender<Msg>) -> Result<Self> {
        // SAFETY: plain COM creation and registration; the guard below owns the registration.
        let (enumerator, devices) = unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let client: IMMNotificationClient = DeviceEvents { tx: tx.clone() }.into();
            enumerator.RegisterEndpointNotificationCallback(&client)?;
            let devices = DeviceSubscription {
                enumerator: enumerator.clone(),
                client,
            };
            (enumerator, devices)
        };
        let mut watcher = Self {
            out,
            tx,
            enumerator,
            volume: None,
            bluetooth: HashMap::new(),
            wired: HashSet::new(),
            battery_checks: Vec::new(),
            _devices: devices,
        };
        watcher.subscribe_default();
        Ok(watcher)
    }

    fn run(mut self, rx: Receiver<Msg>) {
        loop {
            let next_check = self.battery_checks.iter().map(|(at, _)| *at).min();
            let msg = match next_check {
                Some(at) => match rx.recv_timeout(at.saturating_duration_since(Instant::now())) {
                    Ok(msg) => Some(msg),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                },
                None => match rx.recv() {
                    Ok(msg) => Some(msg),
                    Err(_) => return,
                },
            };
            match msg {
                Some(Msg::Shutdown) => {
                    log::info!("audio watcher stopped");
                    return;
                }
                Some(Msg::Volume { level, muted }) => self.on_volume(level, muted),
                Some(Msg::DefaultChanged) => self.subscribe_default(),
                Some(Msg::StateChanged(id, state)) => self.on_state(&id, state),
                None => {}
            }
            self.run_battery_checks();
        }
    }

    fn send(&self, activity: Activity) {
        let _ = self.out.send(activity);
    }

    fn on_volume(&mut self, level: u8, muted: bool) {
        let Some(volume) = &mut self.volume else {
            return;
        };
        // Channel-balance changes notify too; only level and mute matter.
        if volume.last == (level, muted) {
            return;
        }
        volume.last = (level, muted);
        let activity = Activity::Volume(VolumeActivity {
            level,
            muted,
            device: volume.device.clone(),
        });
        self.send(activity);
    }

    /// (Re)subscribes to the default output's volume. Its current level is the baseline, not an activity.
    fn subscribe_default(&mut self) {
        self.volume = None;
        match self.open_volume() {
            Ok(v) => {
                log::debug!("watching volume of {:?} (at {:?})", v.device, v.last);
                self.volume = Some(v);
            }
            Err(e) => log::debug!("no default audio output to watch: {e}"),
        }
    }

    fn open_volume(&self) -> Result<VolumeSubscription> {
        // SAFETY: plain COM calls; the guard owns the registration as soon as it succeeds.
        unsafe {
            let device = self.enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
            let endpoint: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None)?;
            let callback: IAudioEndpointVolumeCallback = VolumeEvents {
                tx: self.tx.clone(),
            }
            .into();
            endpoint.RegisterControlChangeNotify(&callback)?;
            let mut subscription = VolumeSubscription {
                endpoint,
                callback,
                device: string_property(&device, &PKEY_Device_FriendlyName).unwrap_or_default(),
                last: (0, false),
            };
            subscription.last = (
                percent(subscription.endpoint.GetMasterVolumeLevelScalar()?),
                subscription.endpoint.GetMute()?.as_bool(),
            );
            Ok(subscription)
        }
    }

    fn on_state(&mut self, id: &str, state: DEVICE_STATE) {
        let connected = state == DEVICE_STATE_ACTIVE;
        if !connected && state != DEVICE_STATE_UNPLUGGED && state != DEVICE_STATE_NOTPRESENT {
            return; // Disabled in Sound settings: not a connection change.
        }
        let Ok(Some((container, endpoint_name))) = self.render_endpoint(id) else {
            return;
        };
        let key = format!("{container:?}");
        if self.bluetooth.get(&key) == Some(&connected) || self.wired.contains(&key) {
            return;
        }
        let info = match bluetooth_info(&container) {
            Ok(Some(info)) => info,
            Ok(None) => {
                self.wired.insert(key);
                return;
            }
            Err(e) => {
                log::debug!("looking up audio device {key} failed: {e}");
                return;
            }
        };
        self.bluetooth.insert(key.clone(), connected);
        let activity = BluetoothActivity {
            name: info.name.unwrap_or(endpoint_name),
            connected,
            battery: info.battery.filter(|_| connected),
            container: key,
        };
        log::debug!(
            "bluetooth audio {:?}: connected {connected}, battery {:?}",
            activity.name,
            activity.battery
        );
        if connected {
            self.battery_checks
                .push((Instant::now() + BATTERY_RECHECK, activity.clone()));
        }
        self.send(Activity::Bluetooth(activity));
    }

    /// Re-reads the battery of headsets that just connected and updates their activity if it changed.
    fn run_battery_checks(&mut self) {
        let now = Instant::now();
        let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.battery_checks)
            .into_iter()
            .partition(|(at, _)| *at <= now);
        self.battery_checks = later;
        for (_, mut activity) in due {
            if self.bluetooth.get(&activity.container) != Some(&true) {
                continue;
            }
            let Ok(container) = GUID::try_from(activity.container.as_str()) else {
                continue;
            };
            let battery = bluetooth_info(&container)
                .ok()
                .flatten()
                .and_then(|i| i.battery);
            if battery.is_some() && battery != activity.battery {
                activity.battery = battery;
                self.send(Activity::Bluetooth(activity));
            }
        }
    }

    /// The endpoint's container id and name, if `id` is an output (render) endpoint.
    fn render_endpoint(&self, id: &str) -> Result<Option<(GUID, String)>> {
        // SAFETY: plain COM queries on an endpoint id Core Audio just gave us.
        unsafe {
            let device = self.enumerator.GetDevice(&HSTRING::from(id))?;
            if device.cast::<IMMEndpoint>()?.GetDataFlow()? != eRender {
                return Ok(None);
            }
            let store = device.OpenPropertyStore(STGM_READ)?;
            let mut value = store.GetValue(&PKEY_Device_ContainerId)?;
            let container = PropVariantToGUID(&value);
            let _ = PropVariantClear(&mut value);
            let name = string_property(&device, &PKEY_Device_FriendlyName).unwrap_or_default();
            Ok(Some((container?, name)))
        }
    }
}

struct BluetoothInfo {
    name: Option<String>,
    battery: Option<u8>,
}

/// Looks at every device node in the container: `None` if none is a Bluetooth node.
fn bluetooth_info(container: &GUID) -> Result<Option<BluetoothInfo>> {
    let aqs = HSTRING::from(format!("System.Devices.ContainerId:=\"{{{container:?}}}\""));
    let key = HSTRING::from(BATTERY_KEY);
    let properties: IIterable<HSTRING> = vec![key.clone()].into();
    let devices = DeviceInformation::FindAllAsyncWithKindAqsFilterAndAdditionalProperties(
        &aqs,
        &properties,
        DeviceInformationKind::Device,
    )?
    .join()?;

    let mut bluetooth = false;
    let mut name = None;
    let mut battery = None;
    for device in &devices {
        let id = device.Id()?.to_string().to_ascii_uppercase();
        if BLUETOOTH_PREFIXES.iter().any(|p| id.starts_with(p)) {
            bluetooth = true;
        }
        // The device's own node (not a profile) carries the name shown in Settings.
        if name.is_none() && (id.starts_with("BTHENUM\\DEV_") || id.starts_with("BTHLE\\DEV_")) {
            name = Some(device.Name()?.to_string());
        }
        if battery.is_none() {
            battery = device
                .Properties()
                .and_then(|p| p.Lookup(&key))
                .and_then(|v| v.cast::<IReference<u8>>())
                .and_then(|r| r.Value())
                .ok()
                .filter(|b| *b <= 100);
        }
    }
    Ok(bluetooth.then_some(BluetoothInfo { name, battery }))
}

fn string_property(device: &IMMDevice, key: &PROPERTYKEY) -> Result<String> {
    // SAFETY: the PROPVARIANT is cleared and the allocated string freed before returning.
    unsafe {
        let store = device.OpenPropertyStore(STGM_READ)?;
        let mut value: PROPVARIANT = store.GetValue(key)?;
        let text = PropVariantToStringAlloc(&value);
        let _ = PropVariantClear(&mut value);
        let text = text?;
        let out = String::from_utf16_lossy(text.as_wide());
        CoTaskMemFree(Some(text.0 as *const _));
        Ok(out)
    }
}

fn percent(scalar: f32) -> u8 {
    (scalar.clamp(0.0, 1.0) * 100.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_rounds_to_whole_percent() {
        assert_eq!(percent(0.0), 0);
        assert_eq!(percent(0.625), 63);
        assert_eq!(percent(1.2), 100);
    }

    #[test]
    fn container_ids_round_trip_through_their_key() {
        let guid = GUID::from_u128(0xad0b441d_e004_5de8_b07f_cb51106386cb);
        let key = format!("{guid:?}");
        assert_eq!(key, "AD0B441D-E004-5DE8-B07F-CB51106386CB");
        assert_eq!(GUID::try_from(key.as_str()).unwrap(), guid);
    }
}
