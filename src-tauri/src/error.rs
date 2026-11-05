use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};

/// Error returned by every command. Serialized as `{ kind, message }` (see `AppError` in `src/lib/types.ts`).
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("That window has closed.")]
    WindowGone,
    #[error("Windows didn't allow switching to that window.")]
    ForegroundDenied,
    #[error("The island window isn't available.")]
    NoIslandWindow,
    #[error("No monitor was found.")]
    NoMonitor,
    #[error("That media session has ended.")]
    MediaSessionGone,
    #[error("The app didn't accept that media command.")]
    MediaRejected,
    #[error("That activity has ended.")]
    ActivityGone,
    #[error("That Claude Code session has ended.")]
    SessionGone,
    #[error("Couldn't find the window that session runs in.")]
    SessionWindowNotFound,
    #[error("{0}")]
    ClaudeSettings(String),
    #[error("That agent run is gone.")]
    AgentRunGone,
    #[error("That approval was already answered or has expired.")]
    AgentApprovalGone,
    #[error(
        "Claude Code wasn't found. Install it, or set agent.claudePath in Cay's settings.json."
    )]
    AgentNoClaude,
    #[error("{0}")]
    AgentInvalid(String),
    #[error("{0}")]
    AgentStart(String),
    #[error("The command window isn't open.")]
    NoCommandWindow,
    #[error("{0}")]
    Google(String),
    #[error("That event or task is no longer listed.")]
    CalendarItemGone,
    #[error("That app is no longer listed.")]
    ProcessGroupGone,
    #[error("Cay is already ending an app.")]
    ProcessEndBusy,
    #[error(transparent)]
    Win32(#[from] windows::core::Error),
    #[error(transparent)]
    Tauri(#[from] tauri::Error),
}

impl AppError {
    fn kind(&self) -> &'static str {
        match self {
            Self::WindowGone => "windowGone",
            Self::ForegroundDenied => "foregroundDenied",
            Self::NoIslandWindow => "noIslandWindow",
            Self::NoMonitor => "noMonitor",
            Self::MediaSessionGone => "mediaSessionGone",
            Self::MediaRejected => "mediaRejected",
            Self::ActivityGone => "activityGone",
            Self::SessionGone => "sessionGone",
            Self::SessionWindowNotFound => "sessionWindowNotFound",
            Self::ClaudeSettings(_) => "claudeSettings",
            Self::AgentRunGone => "agentRunGone",
            Self::AgentApprovalGone => "agentApprovalGone",
            Self::AgentNoClaude => "agentNoClaude",
            Self::AgentInvalid(_) => "agentInvalid",
            Self::AgentStart(_) => "agentStart",
            Self::NoCommandWindow => "noCommandWindow",
            Self::Google(_) => "google",
            Self::CalendarItemGone => "calendarItemGone",
            Self::ProcessGroupGone => "processGroupGone",
            Self::ProcessEndBusy => "processEndBusy",
            Self::Win32(_) => "win32",
            Self::Tauri(_) => "tauri",
        }
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut s = serializer.serialize_struct("AppError", 2)?;
        s.serialize_field("kind", self.kind())?;
        s.serialize_field("message", &self.to_string())?;
        s.end()
    }
}

pub type AppResult<T> = Result<T, AppError>;
