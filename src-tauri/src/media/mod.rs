//! Now playing: GSMTC sessions → `media:state`, plus transport-control commands.

mod artwork;
mod logic;
mod snapshot;
mod watcher;

use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use tauri::{AppHandle, State};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};

use crate::error::{AppError, AppResult};
use crate::ipc::MediaStatePayload;
use watcher::Msg;

const SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(500);

/// A session commands can act on. GSMTC objects are agile, so they're usable from any thread.
#[derive(Clone)]
struct Handle {
    id: u32,
    session: Session,
}

#[derive(Default)]
struct Inner {
    handles: Vec<Handle>,
    state: MediaStatePayload,
    tx: Option<Sender<Msg>>,
}

/// Tauri-managed state: the watcher's latest snapshot, read by commands.
#[derive(Default)]
pub struct Media(Mutex<Inner>);

impl Media {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn publish(&self, handles: Vec<Handle>, state: MediaStatePayload) {
        let mut inner = self.lock();
        inner.handles = handles;
        inner.state = state;
    }

    fn session(&self, id: u32) -> AppResult<Session> {
        self.lock()
            .handles
            .iter()
            .find(|h| h.id == id)
            .map(|h| h.session.clone())
            .ok_or(AppError::MediaSessionGone)
    }

    /// Unregisters every WinRT handler before the process exits (bounded wait).
    pub fn shutdown(&self) {
        let Some(tx) = self.lock().tx.take() else {
            return;
        };
        let (ack_tx, ack_rx) = mpsc::channel();
        if tx.send(Msg::Shutdown(ack_tx)).is_ok() {
            let _ = ack_rx.recv_timeout(SHUTDOWN_TIMEOUT);
        }
    }
}

pub fn spawn(app: AppHandle, media: &Media) -> AppResult<()> {
    let (tx, rx) = mpsc::channel();
    media.lock().tx = Some(tx.clone());
    thread::Builder::new()
        .name("media-watcher".into())
        .spawn(move || watcher::run(app, tx, rx))
        .map_err(tauri::Error::from)?;
    Ok(())
}

fn accepted(ok: bool) -> AppResult<()> {
    if ok {
        Ok(())
    } else {
        Err(AppError::MediaRejected)
    }
}

#[tauri::command]
pub fn get_media_state(media: State<'_, Media>) -> MediaStatePayload {
    media.lock().state.clone()
}

#[tauri::command]
pub async fn media_play_pause(media: State<'_, Media>, session_id: u32) -> AppResult<()> {
    let session = media.session(session_id)?;
    // Explicit play/pause: not every app enables the toggle command.
    let playing = session.GetPlaybackInfo()?.PlaybackStatus()? == Status::Playing;
    let op = if playing {
        session.TryPauseAsync()?
    } else {
        session.TryPlayAsync()?
    };
    accepted(op.await?)
}

#[tauri::command]
pub async fn media_next(media: State<'_, Media>, session_id: u32) -> AppResult<()> {
    let session = media.session(session_id)?;
    accepted(session.TrySkipNextAsync()?.await?)
}

#[tauri::command]
pub async fn media_prev(media: State<'_, Media>, session_id: u32) -> AppResult<()> {
    let session = media.session(session_id)?;
    accepted(session.TrySkipPreviousAsync()?.await?)
}

#[tauri::command]
pub async fn media_seek(
    media: State<'_, Media>,
    session_id: u32,
    position_ms: i64,
) -> AppResult<()> {
    let session = media.session(session_id)?;
    let ticks = snapshot::start_ticks(&session) + logic::ms_to_ticks(position_ms.max(0));
    accepted(session.TryChangePlaybackPositionAsync(ticks)?.await?)
}
