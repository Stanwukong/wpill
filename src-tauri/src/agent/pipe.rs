//! The agent pipe, `\\.\pipe\cay-agent-<user SID>`: one `duplex` instance per bridge.

use windows::core::{Error, Result, HSTRING};

pub use crate::duplex::{Pipe, Read, MAX_MESSAGE};
use crate::win32;

pub fn name() -> Option<HSTRING> {
    Some(HSTRING::from(format!(
        r"\\.\pipe\cay-agent-{}",
        win32::user_sid()?
    )))
}

/// Server side: a new instance (the first one claims the name), not yet connected.
pub fn create(first: bool) -> Result<Pipe> {
    Pipe::create(&name().ok_or_else(Error::from_thread)?, first)
}

/// Client side (the bridge).
pub fn open() -> Result<Pipe> {
    Pipe::open(&name().ok_or_else(Error::from_thread)?)
}
