//! Album art: thumbnail stream → image data URL, with a small per-track cache.

use std::collections::VecDeque;

use base64::Engine;
use windows::Storage::Streams::{DataReader, IRandomAccessStreamReference};

use super::logic;

/// Larger thumbnails are skipped; they would be re-sent with every `media:state` emit.
const MAX_BYTES: u64 = 1_500_000;
const CACHE_LIMIT: usize = 16;

/// Only successful reads are cached: apps often publish a track's properties before its thumbnail.
#[derive(Default)]
pub struct ArtworkCache(VecDeque<(String, String)>);

impl ArtworkCache {
    pub fn get_or_read(
        &mut self,
        key: String,
        thumbnail: Option<&IRandomAccessStreamReference>,
    ) -> Option<String> {
        if let Some(i) = self.0.iter().position(|(k, _)| *k == key) {
            let hit = self.0.remove(i)?;
            let url = hit.1.clone();
            self.0.push_front(hit);
            return Some(url);
        }
        let url = match read(thumbnail?) {
            Ok(url) => url?,
            Err(e) => {
                log::debug!("reading media thumbnail failed: {e}");
                return None;
            }
        };
        self.0.push_front((key, url.clone()));
        self.0.truncate(CACHE_LIMIT);
        Some(url)
    }
}

fn read(reference: &IRandomAccessStreamReference) -> windows::core::Result<Option<String>> {
    let stream = reference.OpenReadAsync()?.join()?;
    let size = stream.Size()?;
    if size == 0 || size > MAX_BYTES {
        log::debug!("media thumbnail skipped: {size} bytes");
        return Ok(None);
    }
    let reader = DataReader::CreateDataReader(&stream)?;
    let loaded = reader.LoadAsync(size as u32)?.join()?;
    let mut bytes = vec![0u8; loaded as usize];
    reader.ReadBytes(&mut bytes)?;

    let reported = stream
        .ContentType()
        .map(|t| t.to_string())
        .unwrap_or_default();
    let mime = match logic::sniff_image_mime(&bytes) {
        Some(m) => m.to_string(),
        None if reported.starts_with("image/") => reported,
        None => return Ok(None),
    };
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(Some(format!("data:{mime};base64,{b64}")))
}
