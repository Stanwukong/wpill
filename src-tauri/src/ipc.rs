//! IPC contract shared with `src/lib/ipc.ts` and `src/lib/types.ts`. Change both sides together.

use serde::Serialize;

pub mod events {
    pub const HOVER_ENTER: &str = "island:hover-enter";
    pub const HOVER_LEAVE: &str = "island:hover-leave";
    pub const VISIBILITY: &str = "island:visibility";
    pub const MEDIA_STATE: &str = "media:state";
    pub const ACTIVITY_SHOW: &str = "activity:show";
    pub const ACTIVITY_HIDE: &str = "activity:hide";
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibilityPayload {
    pub hidden: bool,
    pub reason: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IslandStatePayload {
    pub expanded: bool,
    pub hidden: bool,
    pub activity: Option<ActivityPayload>,
}

/// `activity:show`. Re-sent with the same `id` when a burst (e.g. volume key presses) updates it in place.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityPayload {
    pub id: u32,
    #[serde(flatten)]
    pub activity: Activity,
    /// Time left at emit; Rust times it (and holds it while an interactive activity is hovered).
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityHidePayload {
    pub id: u32,
}

/// Serialized as `{ kind, payload }`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", content = "payload", rename_all = "camelCase")]
pub enum Activity {
    Volume(VolumeActivity),
    Brightness(BrightnessActivity),
    Bluetooth(BluetoothActivity),
    Power(PowerActivity),
    CapsLock(CapsLockActivity),
    Screenshot(ScreenshotActivity),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VolumeActivity {
    /// 0–100.
    pub level: u8,
    pub muted: bool,
    pub device: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrightnessActivity {
    /// 0–100.
    pub level: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BluetoothActivity {
    pub name: String,
    pub connected: bool,
    /// 0–100, when the device reports it.
    pub battery: Option<u8>,
    /// Device container id; one headset's A2DP and hands-free endpoints share it.
    #[serde(skip)]
    pub container: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PowerEvent {
    PluggedIn,
    Unplugged,
    Low,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PowerActivity {
    pub event: PowerEvent,
    /// Battery percentage, 0–100.
    pub level: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapsLockActivity {
    pub on: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotActivity {
    pub file_name: String,
    /// PNG data URL, about 256 px on its long side.
    pub thumbnail: String,
    pub width: u32,
    pub height: u32,
    /// Stays in Rust: drag/open commands look it up by activity id, so JS never names a path.
    #[serde(skip)]
    pub path: std::path::PathBuf,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowInfo {
    pub hwnd: i64,
    pub title: String,
    pub exe_name: Option<String>,
    /// PNG data URL.
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaStatePayload {
    /// Most recently active first.
    pub sessions: Vec<MediaSession>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaSession {
    /// Ours, stable for the session's lifetime; not a Windows id.
    pub id: u32,
    pub app_name: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// Image data URL.
    pub artwork: Option<String>,
    /// `"playing" | "paused" | "stopped" | "changing"`
    pub status: &'static str,
    /// Position at `updated_at_ms`; React interpolates from there while playing.
    pub position_ms: i64,
    pub duration_ms: Option<i64>,
    /// Unix epoch ms.
    pub updated_at_ms: i64,
    pub playback_rate: f64,
    pub can_play: bool,
    pub can_pause: bool,
    pub can_next: bool,
    pub can_prev: bool,
    pub can_seek: bool,
}
