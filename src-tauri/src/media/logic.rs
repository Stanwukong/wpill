//! Pure helpers for the media watcher: time conversion, change detection, app labels.

use crate::ipc::MediaSession;

/// WinRT `TimeSpan`/`DateTime` tick = 100 ns.
const TICKS_PER_MS: i64 = 10_000;
/// 1601-01-01 → 1970-01-01 in ticks.
const UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;
/// A timeline update is re-emitted only when it disagrees with React's interpolation by more than this.
pub const DRIFT_MS: i64 = 1_500;

pub fn ticks_to_ms(ticks: i64) -> i64 {
    ticks / TICKS_PER_MS
}

pub fn ms_to_ticks(ms: i64) -> i64 {
    ms.saturating_mul(TICKS_PER_MS)
}

/// `DateTime.UniversalTime` → Unix ms; `None` for the zero/garbage values some apps report.
pub fn datetime_to_unix_ms(universal_ticks: i64) -> Option<i64> {
    (universal_ticks > UNIX_EPOCH_TICKS)
        .then(|| (universal_ticks - UNIX_EPOCH_TICKS) / TICKS_PER_MS)
}

/// Where the session should be at `at_ms`, interpolating like the React progress bar does.
pub fn expected_position(s: &MediaSession, at_ms: i64) -> i64 {
    if s.status != "playing" {
        return s.position_ms;
    }
    let elapsed = (at_ms - s.updated_at_ms).max(0) as f64 * s.playback_rate;
    let pos = s.position_ms + elapsed as i64;
    match s.duration_ms {
        Some(d) => pos.clamp(0, d),
        None => pos.max(0),
    }
}

/// Whether `next` tells React anything it can't already work out from `prev`.
pub fn worth_emitting(prev: &MediaSession, next: &MediaSession) -> bool {
    let same_except_timeline = prev.id == next.id
        && prev.app_name == next.app_name
        && prev.title == next.title
        && prev.artist == next.artist
        && prev.album == next.album
        && prev.artwork == next.artwork
        && prev.status == next.status
        && prev.duration_ms == next.duration_ms
        && prev.playback_rate == next.playback_rate
        && prev.can_play == next.can_play
        && prev.can_pause == next.can_pause
        && prev.can_next == next.can_next
        && prev.can_prev == next.can_prev
        && prev.can_seek == next.can_seek;
    if !same_except_timeline {
        return true;
    }
    (expected_position(prev, next.updated_at_ms) - next.position_ms).abs() > DRIFT_MS
}

pub fn list_worth_emitting(prev: &[MediaSession], next: &[MediaSession]) -> bool {
    prev.len() != next.len() || prev.iter().zip(next).any(|(p, n)| worth_emitting(p, n))
}

/// Friendly name from a GSMTC `SourceAppUserModelId`, e.g. `Spotify.exe`, `MSEdge`,
/// `Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic`, or a full exe path.
pub fn app_label(aumid: &str) -> String {
    // Packaged apps: "Package_publisher!AppId"; the app id is the readable half.
    let id = aumid.rsplit('!').next().unwrap_or(aumid);
    let id = id.rsplit(['\\', '/']).next().unwrap_or(id);
    let id = id
        .strip_suffix(".exe")
        .or_else(|| id.strip_suffix(".EXE"))
        .unwrap_or(id);
    let id = id.rsplit('.').next().unwrap_or(id);
    match id.to_ascii_lowercase().as_str() {
        "msedge" => "Edge".into(),
        "chrome" => "Chrome".into(),
        "firefox" | "308046b0af4a39cb" => "Firefox".into(),
        "zunemusic" => "Media Player".into(),
        "spotify" | "spotifymusic" => "Spotify".into(),
        "" => "Media".into(),
        _ => {
            let mut chars = id.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
    }
}

/// Image MIME type from magic bytes; thumbnail streams often report an empty or generic content type.
pub fn sniff_image_mime(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some("image/png"),
        [0xFF, 0xD8, 0xFF, ..] => Some("image/jpeg"),
        [b'G', b'I', b'F', b'8', ..] => Some("image/gif"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("image/webp"),
        [b'B', b'M', ..] => Some("image/bmp"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> MediaSession {
        MediaSession {
            id: 1,
            app_name: "Spotify".into(),
            title: "Song".into(),
            artist: "Artist".into(),
            album: "Album".into(),
            artwork: None,
            status: "playing",
            position_ms: 10_000,
            duration_ms: Some(200_000),
            updated_at_ms: 1_000_000,
            playback_rate: 1.0,
            can_play: true,
            can_pause: true,
            can_next: true,
            can_prev: true,
            can_seek: true,
        }
    }

    #[test]
    fn converts_winrt_times() {
        assert_eq!(ticks_to_ms(ms_to_ticks(12_345)), 12_345);
        assert_eq!(datetime_to_unix_ms(0), None);
        assert_eq!(
            datetime_to_unix_ms(UNIX_EPOCH_TICKS + 5 * TICKS_PER_MS),
            Some(5)
        );
    }

    #[test]
    fn interpolates_only_while_playing_and_clamps() {
        let s = session();
        assert_eq!(expected_position(&s, 1_003_000), 13_000);
        assert_eq!(expected_position(&s, 9_000_000), 200_000);
        let paused = MediaSession {
            status: "paused",
            ..session()
        };
        assert_eq!(expected_position(&paused, 1_003_000), 10_000);
        let fast = MediaSession {
            playback_rate: 2.0,
            ..session()
        };
        assert_eq!(expected_position(&fast, 1_001_000), 12_000);
    }

    #[test]
    fn steady_timeline_updates_are_not_re_emitted() {
        let prev = session();
        let on_track = MediaSession {
            position_ms: 13_100,
            updated_at_ms: 1_003_000,
            ..session()
        };
        assert!(!worth_emitting(&prev, &on_track));
        let seeked = MediaSession {
            position_ms: 60_000,
            updated_at_ms: 1_003_000,
            ..session()
        };
        assert!(worth_emitting(&prev, &seeked));
        let paused = MediaSession {
            status: "paused",
            ..session()
        };
        assert!(worth_emitting(&prev, &paused));
        let list = std::slice::from_ref(&prev);
        assert!(list_worth_emitting(list, &[]));
        assert!(!list_worth_emitting(list, list));
    }

    #[test]
    fn labels_common_apps() {
        assert_eq!(app_label("Spotify.exe"), "Spotify");
        assert_eq!(app_label("MSEdge"), "Edge");
        assert_eq!(app_label("Chrome"), "Chrome");
        assert_eq!(app_label("308046B0AF4A39CB"), "Firefox");
        assert_eq!(
            app_label("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"),
            "Media Player"
        );
        assert_eq!(
            app_label("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"),
            "Spotify"
        );
        assert_eq!(app_label(r"C:\Program Files\VideoLAN\VLC\vlc.exe"), "Vlc");
        assert_eq!(app_label(""), "Media");
    }

    #[test]
    fn sniffs_image_types() {
        assert_eq!(
            sniff_image_mime(&[0x89, b'P', b'N', b'G', 0x0D]),
            Some("image/png")
        );
        assert_eq!(
            sniff_image_mime(&[0xFF, 0xD8, 0xFF, 0xE0]),
            Some("image/jpeg")
        );
        assert_eq!(sniff_image_mime(b"RIFF\0\0\0\0WEBPVP8"), Some("image/webp"));
        assert_eq!(sniff_image_mime(b"hello"), None);
    }
}
