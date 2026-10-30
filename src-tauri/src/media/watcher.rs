//! The `media-watcher` thread: owns the GSMTC manager and every session subscription.
//!
//! WinRT raises events on thread-pool threads. Handlers only forward a `Msg` here; all state
//! lives on this thread. Each event registration is held by a `Subscription`, whose `Drop`
//! unregisters it, so removing a session's `Entry` (or shutting down) tears everything down.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager as _};
use windows::core::{IUnknown, Interface};
use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as SessionManager,
};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

use super::artwork::ArtworkCache;
use super::snapshot::{self, Props};
use super::{logic, Handle, Media};
use crate::ipc::{events, MediaSession, MediaStatePayload};

/// Bursts (a track change fires all three session events) are batched into one re-read.
const COALESCE: Duration = Duration::from_millis(50);
/// Safety net while something plays, in case an app stops raising timeline events.
const PLAYING_RESYNC: Duration = Duration::from_secs(5);
/// Re-reads of a track's properties while its thumbnail is still missing.
const ARTWORK_RETRIES: u8 = 3;

pub enum Msg {
    SessionsChanged,
    CurrentChanged,
    /// Session id; `true` when its media properties (title, art) changed.
    Changed(u32, bool),
    /// Acknowledged once every subscription has been removed.
    Shutdown(Sender<()>),
}

/// An event registration, removed on drop.
struct Subscription<T> {
    source: T,
    token: i64,
    remove: fn(&T, i64) -> windows::core::Result<()>,
}

impl<T> Drop for Subscription<T> {
    fn drop(&mut self) {
        if let Err(e) = (self.remove)(&self.source, self.token) {
            log::debug!("removing a media event handler failed: {e}");
        }
    }
}

struct Entry {
    id: u32,
    /// COM identity (`IUnknown` pointer): stable across `GetSessions()` calls, unlike the
    /// app id, which two browser sessions can share.
    identity: usize,
    session: Session,
    app_name: String,
    props: Props,
    art_attempts: u8,
    last_active: u64,
    last_status: &'static str,
    _subscriptions: [Subscription<Session>; 3],
}

#[derive(Default)]
struct Batch {
    sessions: bool,
    current: bool,
    /// Session id → properties changed.
    changed: HashMap<u32, bool>,
    shutdown: Option<Sender<()>>,
}

struct Watcher {
    app: AppHandle,
    tx: Sender<Msg>,
    entries: Vec<Entry>,
    manager: SessionManager,
    _manager_subscriptions: [Subscription<SessionManager>; 2],
    art: ArtworkCache,
    emitted: Vec<MediaSession>,
    next_id: u32,
    seq: u64,
}

pub fn run(app: AppHandle, tx: Sender<Msg>, rx: Receiver<Msg>) {
    // SAFETY: called once at the top of this thread; balanced by CoUninitialize below.
    let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    match Watcher::new(app, tx) {
        Ok(watcher) => watcher.run(rx),
        Err(e) => log::warn!("media controls unavailable: {e}"),
    }
    if com.is_ok() {
        // SAFETY: matches the successful CoInitializeEx above; every WinRT object is dropped by now.
        unsafe { CoUninitialize() };
    }
}

impl Watcher {
    fn new(app: AppHandle, tx: Sender<Msg>) -> windows::core::Result<Self> {
        let manager = SessionManager::RequestAsync()?.join()?;
        let sessions = {
            let tx = tx.clone();
            let token = manager.SessionsChanged(&TypedEventHandler::new(move |_, _| {
                let _ = tx.send(Msg::SessionsChanged);
                Ok(())
            }))?;
            Subscription {
                source: manager.clone(),
                token,
                remove: SessionManager::RemoveSessionsChanged,
            }
        };
        let current = {
            let tx = tx.clone();
            let token = manager.CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
                let _ = tx.send(Msg::CurrentChanged);
                Ok(())
            }))?;
            Subscription {
                source: manager.clone(),
                token,
                remove: SessionManager::RemoveCurrentSessionChanged,
            }
        };
        Ok(Self {
            app,
            tx,
            entries: Vec::new(),
            manager,
            _manager_subscriptions: [sessions, current],
            art: ArtworkCache::default(),
            emitted: Vec::new(),
            next_id: 1,
            seq: 0,
        })
    }

    fn run(mut self, rx: Receiver<Msg>) {
        self.apply(Batch {
            sessions: true,
            current: true,
            ..Batch::default()
        });

        loop {
            let playing = self.emitted.iter().any(|s| s.status == "playing");
            let first = if playing {
                match rx.recv_timeout(PLAYING_RESYNC) {
                    Ok(msg) => Some(msg),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            } else {
                match rx.recv() {
                    Ok(msg) => Some(msg),
                    Err(_) => return,
                }
            };

            let mut batch = Batch::default();
            if let Some(msg) = first {
                absorb(&mut batch, msg);
                let deadline = Instant::now() + COALESCE;
                while batch.shutdown.is_none() {
                    let left = deadline.saturating_duration_since(Instant::now());
                    match rx.recv_timeout(left) {
                        Ok(msg) => absorb(&mut batch, msg),
                        Err(RecvTimeoutError::Timeout) => break,
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
            }

            if let Some(ack) = batch.shutdown.take() {
                let count = self.entries.len();
                drop(self);
                log::info!("media watcher stopped ({count} session subscriptions removed)");
                let _ = ack.send(());
                return;
            }
            self.apply(batch);
        }
    }

    fn apply(&mut self, batch: Batch) {
        if batch.sessions {
            if let Err(e) = self.sync_sessions() {
                log::warn!("listing media sessions failed: {e}");
            }
        }
        if batch.current {
            self.bump_current();
        }
        for entry in &mut self.entries {
            let props_changed = batch.changed.get(&entry.id).copied().unwrap_or(false);
            let art_missing = entry.props.artwork.is_none()
                && !entry.props.title.is_empty()
                && entry.art_attempts < ARTWORK_RETRIES
                && batch.changed.contains_key(&entry.id);
            if props_changed || art_missing {
                entry.art_attempts += 1;
                match snapshot::read_props(entry.id, &entry.session, &mut self.art) {
                    Ok(props) => {
                        if props.title != entry.props.title {
                            entry.art_attempts = 0;
                            self.seq += 1;
                            entry.last_active = self.seq;
                        }
                        entry.props = props;
                    }
                    Err(e) => log::debug!("reading media properties failed: {e}"),
                }
            }
        }
        self.publish();
    }

    /// Diffs `GetSessions()` against our entries: subscribes new sessions, drops vanished ones.
    fn sync_sessions(&mut self) -> windows::core::Result<()> {
        let mut kept = Vec::new();
        for session in &self.manager.GetSessions()? {
            let identity = identity(&session)?;
            if let Some(i) = self.entries.iter().position(|e| e.identity == identity) {
                kept.push(self.entries.swap_remove(i));
                continue;
            }
            match self.add(session, identity) {
                Ok(entry) => kept.push(entry),
                Err(e) => log::warn!("subscribing to a media session failed: {e}"),
            }
        }
        for gone in &self.entries {
            log::debug!("media session {} ({}) ended", gone.id, gone.app_name);
        }
        // Dropping the rest unregisters their handlers.
        self.entries = kept;
        Ok(())
    }

    fn add(&mut self, session: Session, identity: usize) -> windows::core::Result<Entry> {
        let id = self.next_id;
        self.next_id += 1;
        let aumid = session.SourceAppUserModelId()?.to_string();
        let subscriptions = subscribe(&session, id, &self.tx)?;
        let props = snapshot::read_props(id, &session, &mut self.art).unwrap_or_default();
        self.seq += 1;
        log::debug!("media session {id} started: {aumid} (identity {identity:#x})");
        Ok(Entry {
            id,
            identity,
            app_name: logic::app_label(&aumid),
            session,
            props,
            art_attempts: 0,
            last_active: self.seq,
            last_status: "stopped",
            _subscriptions: subscriptions,
        })
    }

    /// Windows' own "current session" (what the volume flyout shows) counts as recent activity.
    fn bump_current(&mut self) {
        let Ok(current) = self.manager.GetCurrentSession() else {
            return;
        };
        let Ok(identity) = identity(&current) else {
            return;
        };
        if let Some(entry) = self.entries.iter_mut().find(|e| e.identity == identity) {
            self.seq += 1;
            entry.last_active = self.seq;
        }
    }

    fn publish(&mut self) {
        let mut built = Vec::new();
        for entry in &mut self.entries {
            match snapshot::build(entry.id, &entry.app_name, &entry.props, &entry.session) {
                Ok(Some(s)) => {
                    if s.status == "playing" && entry.last_status != "playing" {
                        self.seq += 1;
                        entry.last_active = self.seq;
                    }
                    entry.last_status = s.status;
                    built.push((entry.last_active, s));
                }
                Ok(None) => entry.last_status = "stopped",
                Err(e) => log::debug!("reading media session {} failed: {e}", entry.id),
            }
        }
        built.sort_by_key(|(active, _)| std::cmp::Reverse(*active));
        let sessions: Vec<MediaSession> = built.into_iter().map(|(_, s)| s).collect();

        let handles = self
            .entries
            .iter()
            .map(|e| Handle {
                id: e.id,
                session: e.session.clone(),
            })
            .collect();
        let changed = logic::list_worth_emitting(&self.emitted, &sessions);
        let payload = MediaStatePayload { sessions };
        self.app.state::<Media>().publish(handles, payload.clone());

        if changed {
            log::debug!(
                "media:state {:?}",
                payload
                    .sessions
                    .iter()
                    .map(|s| (s.id, s.app_name.as_str(), s.status, s.title.as_str()))
                    .collect::<Vec<_>>()
            );
            self.emitted = payload.sessions.clone();
            if let Err(e) = self.app.emit(events::MEDIA_STATE, payload) {
                log::warn!("emit {} failed: {e}", events::MEDIA_STATE);
            }
        }
    }
}

fn absorb(batch: &mut Batch, msg: Msg) {
    match msg {
        Msg::SessionsChanged => batch.sessions = true,
        Msg::CurrentChanged => batch.current = true,
        Msg::Changed(id, props) => *batch.changed.entry(id).or_default() |= props,
        Msg::Shutdown(ack) => batch.shutdown = Some(ack),
    }
}

fn identity(session: &Session) -> windows::core::Result<usize> {
    Ok(session.cast::<IUnknown>()?.as_raw() as usize)
}

fn subscribe(
    session: &Session,
    id: u32,
    tx: &Sender<Msg>,
) -> windows::core::Result<[Subscription<Session>; 3]> {
    let props = {
        let tx = tx.clone();
        let token = session.MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
            let _ = tx.send(Msg::Changed(id, true));
            Ok(())
        }))?;
        Subscription {
            source: session.clone(),
            token,
            remove: Session::RemoveMediaPropertiesChanged,
        }
    };
    let playback = {
        let tx = tx.clone();
        let token = session.PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
            let _ = tx.send(Msg::Changed(id, false));
            Ok(())
        }))?;
        Subscription {
            source: session.clone(),
            token,
            remove: Session::RemovePlaybackInfoChanged,
        }
    };
    let timeline = {
        let tx = tx.clone();
        let token = session.TimelinePropertiesChanged(&TypedEventHandler::new(move |_, _| {
            let _ = tx.send(Msg::Changed(id, false));
            Ok(())
        }))?;
        Subscription {
            source: session.clone(),
            token,
            remove: Session::RemoveTimelinePropertiesChanged,
        }
    };
    Ok([props, playback, timeline])
}
