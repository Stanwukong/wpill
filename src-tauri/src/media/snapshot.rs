//! Reads one GSMTC session into the parts of a `MediaSession` DTO.

use std::time::{SystemTime, UNIX_EPOCH};

use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};

use super::artwork::ArtworkCache;
use super::logic;
use crate::ipc::MediaSession;

/// Track metadata; refreshed only on `MediaPropertiesChanged` (it needs an async round trip).
#[derive(Debug, Clone, Default)]
pub struct Props {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub artwork: Option<String>,
}

pub fn read_props(
    id: u32,
    session: &Session,
    art: &mut ArtworkCache,
) -> windows::core::Result<Props> {
    let p = session.TryGetMediaPropertiesAsync()?.join()?;
    let title = p.Title()?.to_string();
    let artist = p.Artist()?.to_string();
    let album = p.AlbumTitle()?.to_string();
    let key = format!("{id}\u{1f}{title}\u{1f}{artist}\u{1f}{album}");
    let artwork = art.get_or_read(key, p.Thumbnail().ok().as_ref());
    Ok(Props {
        title,
        artist,
        album,
        artwork,
    })
}

/// Playback and timeline are cheap synchronous getters, so they're re-read on every change.
pub fn build(
    id: u32,
    app_name: &str,
    props: &Props,
    session: &Session,
) -> windows::core::Result<Option<MediaSession>> {
    let info = session.GetPlaybackInfo()?;
    let status = match info.PlaybackStatus()? {
        Status::Playing => "playing",
        Status::Paused => "paused",
        Status::Changing => "changing",
        Status::Closed => return Ok(None),
        _ => "stopped",
    };
    let controls = info.Controls()?;
    let rate = info
        .PlaybackRate()
        .and_then(|r| r.Value())
        .ok()
        .filter(|r| r.is_finite() && *r > 0.0)
        .unwrap_or(1.0);

    let timeline = session.GetTimelineProperties()?;
    let start = timeline.StartTime()?.Duration;
    let end = timeline.EndTime()?.Duration;
    let duration_ms = Some(logic::ticks_to_ms(end - start)).filter(|d| *d > 0);
    let position_ms = logic::ticks_to_ms(timeline.Position()?.Duration - start).max(0);
    let updated_at_ms = timeline
        .LastUpdatedTime()
        .ok()
        .and_then(|t| logic::datetime_to_unix_ms(t.UniversalTime))
        .unwrap_or_else(now_unix_ms);

    // Idle browser tabs keep a session with no metadata around; nothing worth a card.
    if props.title.is_empty() && status != "playing" {
        return Ok(None);
    }

    Ok(Some(MediaSession {
        id,
        app_name: app_name.to_string(),
        title: props.title.clone(),
        artist: props.artist.clone(),
        album: props.album.clone(),
        artwork: props.artwork.clone(),
        status,
        position_ms,
        duration_ms,
        updated_at_ms,
        playback_rate: rate,
        can_play: controls.IsPlayEnabled()?,
        can_pause: controls.IsPauseEnabled()?,
        can_next: controls.IsNextEnabled()?,
        can_prev: controls.IsPreviousEnabled()?,
        can_seek: controls.IsPlaybackPositionEnabled()? && duration_ms.is_some(),
    }))
}

/// Timeline start in ticks; seeks are relative to it.
pub fn start_ticks(session: &Session) -> i64 {
    session
        .GetTimelineProperties()
        .and_then(|t| t.StartTime())
        .map(|t| t.Duration)
        .unwrap_or(0)
}

pub fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
