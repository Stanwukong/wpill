//! IPC contract shared with `src/lib/ipc.ts` and `src/lib/types.ts`. Change both sides together.

use serde::Serialize;

pub mod events {
    pub const HOVER_ENTER: &str = "island:hover-enter";
    pub const HOVER_LEAVE: &str = "island:hover-leave";
    pub const VISIBILITY: &str = "island:visibility";
    pub const MEDIA_STATE: &str = "media:state";
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibilityPayload {
    pub hidden: bool,
    pub reason: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IslandStatePayload {
    pub expanded: bool,
    pub hidden: bool,
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
