//! IPC contract shared with `src/lib/ipc.ts` and `src/lib/types.ts`. Change both sides together.

use serde::Serialize;

pub mod events {
    pub const HOVER_ENTER: &str = "island:hover-enter";
    pub const HOVER_LEAVE: &str = "island:hover-leave";
    pub const VISIBILITY: &str = "island:visibility";
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
