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
