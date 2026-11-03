//! Google Calendar and Tasks (Phase 6a, D42): Cay signs in to Google itself and keeps the next 24 h of events and the
//! tasks due today in Rust. The `google-watcher` thread (started through `activity::Watcher`, so shutdown stops it)
//! fetches every few minutes, at each boundary (an event becoming soon, starting, ending; local midnight), after a
//! sleep (the clock jumps), when the network comes back, and backs off while Google can't be reached. Rust works out
//! what's pending, so React never runs a timer (rule 1).
//!
//! Data goes from Google to this PC only. The refresh token is in Credential Manager (`vault.rs`); a small cache of
//! titles and times (`google\cache.json`) lets the island show something before the first fetch, and goes on
//! Disconnect.

mod auth;
mod calendar;
mod gmail;
mod http;
mod logic;
mod mail;
mod network;
mod settings;
mod tasks;
pub mod tools;
mod vault;

pub use mail::Label as MailLabel;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use windows::core::HSTRING;
use windows::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, IDOK, MB_ICONINFORMATION, MB_ICONQUESTION, MB_ICONWARNING, MB_OK, MB_OKCANCEL,
    MB_SETFOREGROUND, MB_TOPMOST, MESSAGEBOX_STYLE,
};

use crate::activity::{self, Activities, Signal, Watcher};
use crate::error::{AppError, AppResult};
use crate::ipc::{
    events, Activity, CalendarSoonActivity, CalendarUpcoming, GoogleState, GoogleStatus,
};
use auth::{AuthError, ClientConfig, Tokens};
use http::{Client, FetchError};
use logic::{Upcoming, MINUTE_MS};
use settings::GoogleSettings;

/// The watcher never sleeps longer than this, so a clock jump (sleep, resume) is noticed within a minute.
const MAX_SLEEP: Duration = Duration::from_secs(60);
/// The wall clock running this far ahead of the monotonic one means the PC slept.
const JUMP_MS: i64 = 2 * MINUTE_MS;
/// A network notification refreshes only if the last fetch is older than this (they come in bursts).
const NETWORK_MIN_GAP_MS: i64 = 30_000;

/// What wakes the watcher.
pub enum Wake {
    /// Look again now (sign-in finished, a debug command).
    Refresh,
    /// The network says it has internet again.
    Network,
    /// Signed in: the first access token, so the watcher needn't refresh at once.
    SignedIn(Tokens),
    /// Disconnected: forget the access token.
    SignedOut,
    /// An agent tool call (6c) needs an access token; `stale` is one Google just refused.
    Token {
        stale: Option<String>,
        reply: Sender<Result<String, String>>,
    },
    Stop,
}

struct Inner {
    status: GoogleStatus,
    account: Option<String>,
    /// Space-separated, as Google granted them.
    scopes: String,
    last_sync_ms: Option<i64>,
    error: Option<String>,
    /// The last fetch, before the time-dependent parts.
    fetched: Upcoming,
    /// `fetched` at the last tick, with phases; what React has and what `calendar_open`/`calendar_join` look up.
    shown: Upcoming,
    settings: GoogleSettings,
    client: Option<ClientConfig>,
    wake: Option<Sender<Wake>>,
    /// Set to stop a sign-in that's waiting on the browser.
    cancel: Arc<AtomicBool>,
    signing_in: bool,
    cache: Option<PathBuf>,
    /// The activity channel, for "starting soon" and withdrawing it after Join.
    activities: Option<Sender<Signal>>,
}

/// Tauri-managed state shared by the watcher, the sign-in thread, the tray and commands.
pub struct Google(Mutex<Inner>);

impl Default for Google {
    fn default() -> Self {
        Self(Mutex::new(Inner {
            status: GoogleStatus::Disconnected,
            account: None,
            scopes: String::new(),
            last_sync_ms: None,
            error: None,
            fetched: Upcoming::default(),
            shown: Upcoming::default(),
            settings: GoogleSettings::default(),
            client: None,
            wake: None,
            cancel: Arc::new(AtomicBool::new(false)),
            signing_in: false,
            cache: None,
            activities: None,
        }))
    }
}

impl Google {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Withdraws an event's "starting soon" (you joined or opened it).
    fn retract(&self, event_id: &str) {
        if let Some(tx) = &self.lock().activities {
            let _ = tx.send(Signal::Retract(activity::calendar_key(event_id)));
        }
    }

    fn wake(&self, wake: Wake) {
        if let Some(tx) = &self.lock().wake {
            let _ = tx.send(wake);
        }
    }
}

impl Inner {
    fn state(&self) -> GoogleState {
        GoogleState {
            status: self.status,
            account: self.account.clone(),
            calendar: self.scopes.contains(auth::SCOPE_CALENDAR_EVENTS),
            tasks: self.scopes.contains(auth::SCOPE_TASKS),
            gmail: self.scopes.contains(auth::SCOPE_GMAIL_READ)
                && self.scopes.contains(auth::SCOPE_GMAIL_COMPOSE),
            last_sync_ms: self.last_sync_ms,
            error: self.error.clone(),
            configured: self.client.is_some(),
        }
    }

    fn upcoming(&self) -> CalendarUpcoming {
        CalendarUpcoming {
            events: self.shown.events.iter().map(|e| e.item.clone()).collect(),
            tasks: self.shown.tasks.iter().map(|t| t.item.clone()).collect(),
            pending: logic::pending(&self.shown),
        }
    }
}

/// What the cache keeps: titles, times and links of the last fetch, never a token.
#[derive(Serialize, Deserialize)]
struct Cache {
    account: Option<String>,
    scopes: String,
    last_sync_ms: Option<i64>,
    upcoming: Upcoming,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

fn offset(utc_ms: i64) -> i64 {
    crate::win32::local_offset_min(utc_ms)
}

/// Reads the settings and the saved sign-in, then starts the watcher.
pub fn spawn(app: &AppHandle, activities: &Activities) {
    let settings = settings::load(app);
    let client = auth::client(
        settings.client_id.as_deref(),
        settings.client_secret.as_deref(),
    );
    let cache = app
        .path()
        .app_config_dir()
        .ok()
        .map(|d| d.join("google").join("cache.json"));
    let (tx, rx) = mpsc::channel::<Wake>();
    {
        let google = app.state::<Google>();
        let mut inner = google.lock();
        if vault::load().is_some() {
            inner.status = GoogleStatus::Connected;
            if let Some(c) = cache.as_deref().and_then(read_cache) {
                inner.account = c.account;
                inner.scopes = c.scopes;
                inner.last_sync_ms = c.last_sync_ms;
                inner.fetched = c.upcoming;
            }
        }
        log::info!(
            "google: {:?}, client {}",
            inner.status,
            if client.is_some() {
                "configured"
            } else {
                "missing"
            }
        );
        inner.settings = settings;
        inner.client = client;
        inner.wake = Some(tx.clone());
        inner.cache = cache;
        inner.activities = Some(activities.sender());
    }
    let body_app = app.clone();
    let network = tx.clone();
    let stop_app = app.clone();
    let watcher = Watcher::spawn(
        "google-watcher",
        move || {
            let _hint = network::Hint::register(network);
            run(&body_app, rx);
        },
        move || {
            stop_app
                .state::<Google>()
                .lock()
                .cancel
                .store(true, Ordering::SeqCst);
            let _ = tx.send(Wake::Stop);
        },
    );
    match watcher {
        Ok(w) => activities.add_watcher(w),
        Err(e) => log::warn!("google watcher unavailable: {e}"),
    }
}

fn run(app: &AppHandle, rx: Receiver<Wake>) {
    let http = match Client::new() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("google: no HTTP session: {e}");
            return;
        }
    };
    let google = app.state::<Google>();
    let mut access: Option<Tokens> = None;
    let mut failures = 0u32;
    let mut next_fetch = now_ms();
    let mut last_fetch = 0i64;
    let mut announced = std::collections::HashSet::new();
    publish(app, now_ms());
    loop {
        let now = now_ms();
        let (soon, lead, refresh) = {
            let inner = google.lock();
            (
                i64::from(inner.settings.soon_minutes) * MINUTE_MS,
                i64::from(inner.settings.lead_minutes) * MINUTE_MS,
                i64::from(inner.settings.refresh_minutes) * MINUTE_MS,
            )
        };
        let boundary = logic::next_boundary(&google.lock().fetched, now, &[soon, lead], offset);
        let until = next_fetch.min(boundary) - now;
        let wait = Duration::from_millis(until.clamp(0, MAX_SLEEP.as_millis() as i64) as u64);
        let (before_wall, before) = (now, Instant::now());
        let mut force = false;
        match rx.recv_timeout(wait) {
            Ok(Wake::Stop) | Err(RecvTimeoutError::Disconnected) => break,
            Ok(Wake::Refresh) => force = true,
            Ok(Wake::Network) => {
                force = failures > 0 || now_ms() - last_fetch > NETWORK_MIN_GAP_MS;
                if force {
                    log::debug!("google: network is back, refreshing");
                }
            }
            Ok(Wake::SignedIn(tokens)) => {
                access = Some(tokens);
                failures = 0;
                force = true;
            }
            Ok(Wake::SignedOut) => {
                access = None;
                failures = 0;
            }
            Ok(Wake::Token { stale, reply }) => {
                // Lent for one tool call; the watcher keeps its own copy and refreshes it as usual.
                if stale.is_some() && access.as_ref().map(|t| &t.access) == stale.as_ref() {
                    access = None;
                }
                let connected = google.lock().status == GoogleStatus::Connected;
                let lent = if connected {
                    token(app, &http, &mut access)
                } else {
                    Err(Failure::Other("Google isn't connected".into()))
                };
                let _ = reply.send(match lent {
                    Ok(t) => Ok(t),
                    Err(Failure::Revoked) => {
                        access = None;
                        next_fetch = i64::MAX;
                        set_status(
                            app,
                            GoogleStatus::Reconnect,
                            Some("Google no longer accepts Cay's sign-in: connect again".into()),
                        );
                        Err("Google no longer accepts Cay's sign-in: Reconnect Google from Cay's tray"
                            .into())
                    }
                    Err(Failure::Other(why)) => Err(why),
                });
                continue;
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
        let now = now_ms();
        let expected = before_wall + before.elapsed().as_millis() as i64;
        if now - expected > JUMP_MS {
            log::info!(
                "google: the clock jumped {} s (sleep?), refreshing",
                (now - expected) / 1000
            );
            force = true;
        }
        let connected = google.lock().status == GoogleStatus::Connected;
        if connected && (force || now >= next_fetch) {
            last_fetch = now;
            match fetch(app, &http, &mut access, now) {
                Ok(()) => {
                    failures = 0;
                    next_fetch = now + refresh;
                }
                Err(Failure::Revoked) => {
                    access = None;
                    failures = 0;
                    next_fetch = i64::MAX;
                    set_status(
                        app,
                        GoogleStatus::Reconnect,
                        Some("Google no longer accepts Cay's sign-in: connect again".into()),
                    );
                }
                Err(Failure::Other(why)) => {
                    failures += 1;
                    let back = logic::backoff(failures);
                    next_fetch = now + back.as_millis() as i64;
                    log::info!(
                        "google: refresh failed ({why}); next try in {} min",
                        back.as_secs() / 60
                    );
                    google.lock().error = Some(why);
                }
            }
        } else if !connected {
            next_fetch = now;
        }
        publish(app, now);
        announce(app, now, lead, &mut announced);
    }
    log::info!("google watcher stopped");
}

enum Failure {
    Revoked,
    Other(String),
}

impl From<AuthError> for Failure {
    fn from(e: AuthError) -> Self {
        match e {
            AuthError::Revoked => Failure::Revoked,
            e => Failure::Other(e.to_string()),
        }
    }
}

/// One refresh: a fresh access token if needed, the calendars, their events, the task lists and due tasks. A 401
/// drops the access token and tries once more.
fn fetch(
    app: &AppHandle,
    http: &Client,
    access: &mut Option<Tokens>,
    now: i64,
) -> Result<(), Failure> {
    for attempt in 0..2 {
        let token = token(app, http, access)?;
        match fetch_with(app, http, &token, now) {
            Err(FetchError::Unauthorized) if attempt == 0 => {
                log::debug!("google: 401, refreshing the access token");
                *access = None;
            }
            Err(FetchError::Unauthorized) => return Err(Failure::Revoked),
            Err(e) => return Err(Failure::Other(e.to_string())),
            Ok(()) => return Ok(()),
        }
    }
    unreachable!("the second attempt always returns")
}

fn token(app: &AppHandle, http: &Client, access: &mut Option<Tokens>) -> Result<String, Failure> {
    if let Some(t) = access.as_ref().filter(|t| Instant::now() < t.expires_at) {
        return Ok(t.access.clone());
    }
    let client = app
        .state::<Google>()
        .lock()
        .client
        .clone()
        .ok_or_else(|| Failure::Other("this build has no Google client".into()))?;
    let saved = vault::load().ok_or(Failure::Revoked)?;
    let tokens = auth::refresh(http, &client, &saved.refresh_token)?;
    let token = tokens.access.clone();
    *access = Some(tokens);
    Ok(token)
}

fn fetch_with(app: &AppHandle, http: &Client, token: &str, now: i64) -> Result<(), FetchError> {
    let (ids, want_tasks, scopes) = {
        let google = app.state::<Google>();
        let inner = google.lock();
        (
            inner.settings.calendars.clone(),
            inner.settings.tasks,
            inner.scopes.clone(),
        )
    };
    let saved_scopes = vault::load().map(|s| s.scopes).unwrap_or(scopes);
    let mut events = Vec::new();
    let mut account = None;
    if saved_scopes.contains(auth::SCOPE_CALENDAR_LIST)
        && saved_scopes.contains(auth::SCOPE_CALENDAR_EVENTS)
    {
        let all = calendar::calendars(http, token)?;
        account = all.iter().find(|c| c.primary).map(|c| c.id.clone());
        for c in calendar::chosen(&all, ids.as_deref()) {
            events.extend(calendar::events(http, token, &c, now)?);
        }
    }
    let mut tasks = Vec::new();
    let mut note = None;
    if want_tasks && saved_scopes.contains(auth::SCOPE_TASKS) {
        let today = logic::local_day(now, offset);
        // Tasks failing (the Tasks API not enabled for the client, say) doesn't hide the events.
        let result = tasks::lists(http, token).and_then(|lists| {
            let mut out = Vec::new();
            for l in &lists {
                out.extend(tasks::due(http, token, l, today)?);
            }
            Ok(out)
        });
        match result {
            Ok(t) => tasks = t,
            Err(FetchError::Unauthorized) => return Err(FetchError::Unauthorized),
            Err(e) => {
                log::info!("google: tasks unavailable: {e}");
                note = Some(format!("Tasks: {e}"));
            }
        }
    }
    let upcoming = Upcoming {
        events: logic::merge(events),
        tasks: logic::sort_tasks(tasks),
    };
    log::debug!(
        "google: {} events, {} tasks",
        upcoming.events.len(),
        upcoming.tasks.len()
    );
    let google = app.state::<Google>();
    let mut inner = google.lock();
    // A Disconnect while this fetch ran wins.
    if inner.status != GoogleStatus::Connected {
        return Ok(());
    }
    inner.fetched = upcoming;
    inner.account = account.or(inner.account.take());
    inner.scopes = saved_scopes;
    inner.last_sync_ms = Some(now);
    inner.error = note;
    if let Some(path) = inner.cache.clone() {
        write_cache(
            &path,
            &Cache {
                account: inner.account.clone(),
                scopes: inner.scopes.clone(),
                last_sync_ms: inner.last_sync_ms,
                upcoming: inner.fetched.clone(),
            },
        );
    }
    Ok(())
}

/// Applies `now` to what was fetched and emits whatever changed (state and/or upcoming).
fn publish(app: &AppHandle, now: i64) {
    static LAST: Mutex<Option<(GoogleState, CalendarUpcoming)>> = Mutex::new(None);
    let (state, upcoming) = {
        let google = app.state::<Google>();
        let mut inner = google.lock();
        let soon = i64::from(inner.settings.soon_minutes) * MINUTE_MS;
        inner.shown = if inner.status == GoogleStatus::Connected {
            logic::at(&inner.fetched, now, soon, logic::local_day(now, offset))
        } else {
            Upcoming::default()
        };
        (inner.state(), inner.upcoming())
    };
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    let (old_state, old_upcoming) = match last.as_ref() {
        Some((s, u)) => (Some(s), Some(u)),
        None => (None, None),
    };
    if old_state != Some(&state) {
        if let Err(e) = app.emit(events::GOOGLE_STATE, &state) {
            log::warn!("emitting google:state failed: {e}");
        }
        crate::tray::show_google(tray_label_for(&state));
    }
    if old_upcoming != Some(&upcoming) {
        log::debug!(
            "calendar:upcoming: {} events, {} tasks, pending {}",
            upcoming.events.len(),
            upcoming.tasks.len(),
            upcoming.pending
        );
        if let Err(e) = app.emit(events::CALENDAR_UPCOMING, &upcoming) {
            log::warn!("emitting calendar:upcoming failed: {e}");
        }
    }
    *last = Some((state, upcoming));
}

/// Offers "starting soon" for each event inside its lead time, once per event and start time.
fn announce(
    app: &AppHandle,
    now: i64,
    lead: i64,
    announced: &mut std::collections::HashSet<(String, i64)>,
) {
    // Forget events that started over an hour ago, so the set stays small.
    announced.retain(|(_, start)| *start > now - 60 * MINUTE_MS);
    let google = app.state::<Google>();
    let inner = google.lock();
    let Some(tx) = inner.activities.as_ref() else {
        return;
    };
    for e in logic::to_announce(&inner.shown, now, lead, announced) {
        announced.insert((e.item.id.clone(), e.item.start_ms));
        log::debug!(
            "google: starting soon, in {} min",
            logic::minutes_until(e.item.start_ms, now)
        );
        let _ = tx.send(Signal::Offer(Activity::CalendarSoon(
            CalendarSoonActivity {
                event_id: e.item.id.clone(),
                title: e.item.title.clone(),
                starts_at_ms: e.item.start_ms,
                minutes: logic::minutes_until(e.item.start_ms, now),
                has_join: e.item.has_join,
                color: e.item.color.clone(),
            },
        )));
    }
}

fn set_status(app: &AppHandle, status: GoogleStatus, error: Option<String>) {
    {
        let google = app.state::<Google>();
        let mut inner = google.lock();
        inner.status = status;
        inner.error = error;
    }
    publish(app, now_ms());
}

fn read_cache(path: &std::path::Path) -> Option<Cache> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text)
        .map_err(|e| log::debug!("ignoring an unreadable Google cache: {e}"))
        .ok()
}

fn write_cache(path: &std::path::Path, cache: &Cache) {
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(path.parent().unwrap_or(path))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(
            &tmp,
            serde_json::to_vec(cache).map_err(std::io::Error::other)?,
        )?;
        std::fs::rename(&tmp, path)
    };
    if let Err(e) = write() {
        log::debug!("writing the Google cache failed: {e}");
    }
}

// ---- Connect / Disconnect ----

/// Starts a sign-in in the browser on its own thread. Errors only when it can't start at all.
fn connect(app: &AppHandle) -> AppResult<()> {
    let google = app.state::<Google>();
    let (client, cancel, scopes) = {
        let mut inner = google.lock();
        let Some(client) = inner.client.clone() else {
            return Err(AppError::Google(no_client_text().into()));
        };
        if inner.signing_in {
            return Ok(());
        }
        inner.signing_in = true;
        inner.cancel.store(false, Ordering::SeqCst);
        (
            client,
            inner.cancel.clone(),
            auth::scopes(inner.settings.gmail),
        )
    };
    let previous = google.lock().status;
    set_status(app, GoogleStatus::Connecting, None);
    let app = app.clone();
    std::thread::Builder::new()
        .name("google-auth".into())
        .spawn(move || {
            let result = Client::new()
                .map_err(AuthError::Net)
                .and_then(|http| auth::sign_in(&http, &client, &scopes, &cancel));
            let google = app.state::<Google>();
            google.lock().signing_in = false;
            match result {
                Ok(tokens) => match signed_in(&tokens) {
                    Ok(()) => {
                        log::info!("google: connected ({})", tokens.scopes);
                        google.lock().scopes = tokens.scopes.clone();
                        set_status(&app, GoogleStatus::Connected, None);
                        google.wake(Wake::SignedIn(tokens));
                    }
                    Err(e) => {
                        log::warn!("google: sign-in couldn't be saved: {e}");
                        set_status(&app, previous_or_disconnected(previous), Some(e));
                    }
                },
                Err(AuthError::Cancelled) => {
                    log::info!("google: sign-in cancelled");
                    set_status(&app, previous_or_disconnected(previous), None);
                }
                Err(e) => {
                    log::warn!("google: sign-in failed: {e}");
                    set_status(
                        &app,
                        previous_or_disconnected(previous),
                        Some(e.to_string()),
                    );
                }
            }
        })
        .map_err(tauri::Error::from)?;
    Ok(())
}

fn previous_or_disconnected(previous: GoogleStatus) -> GoogleStatus {
    match previous {
        GoogleStatus::Connecting => GoogleStatus::Disconnected,
        other => other,
    }
}

fn signed_in(tokens: &Tokens) -> Result<(), String> {
    let refresh = tokens
        .refresh
        .clone()
        .ok_or("Google didn't return a refresh token")?;
    if !tokens.scopes.contains(auth::SCOPE_CALENDAR_EVENTS)
        && !tokens.scopes.contains(auth::SCOPE_TASKS)
    {
        return Err(
            "nothing was granted: tick Calendar or Tasks on Google's consent screen".into(),
        );
    }
    vault::save(&vault::Saved {
        refresh_token: refresh,
        scopes: tokens.scopes.clone(),
    })
}

/// Revokes at Google (blocking, best effort), then forgets the sign-in, the cache and what was fetched.
fn disconnect(app: &AppHandle) {
    let google = app.state::<Google>();
    google.lock().cancel.store(true, Ordering::SeqCst);
    if let Some(saved) = vault::load() {
        match Client::new() {
            Ok(http) => auth::revoke(&http, &saved.refresh_token),
            Err(e) => log::warn!("google: couldn't revoke: {e}"),
        }
    }
    vault::delete();
    {
        let mut inner = google.lock();
        if let Some(path) = &inner.cache {
            let _ = std::fs::remove_file(path);
        }
        inner.fetched = Upcoming::default();
        inner.account = None;
        inner.scopes.clear();
        inner.last_sync_ms = None;
    }
    google.wake(Wake::SignedOut);
    set_status(app, GoogleStatus::Disconnected, None);
    log::info!("google: disconnected");
}

fn no_client_text() -> &'static str {
    "This build of Cay has no Google client. Build with CAY_GOOGLE_CLIENT_ID (and CAY_GOOGLE_CLIENT_SECRET) set, or \
     put your own Desktop app client in settings.json (google.clientId, google.clientSecret)."
}

// ---- Tray ----

pub fn tray_label(app: &AppHandle) -> &'static str {
    tray_label_for(&app.state::<Google>().lock().state())
}

fn tray_label_for(state: &GoogleState) -> &'static str {
    match state.status {
        GoogleStatus::Disconnected => "Connect Google…",
        GoogleStatus::Connecting => "Cancel Google sign-in",
        GoogleStatus::Connected => "Disconnect Google…",
        GoogleStatus::Reconnect => "Reconnect Google…",
    }
}

static DIALOG_OPEN: AtomicBool = AtomicBool::new(false);

/// The tray item: Connect (with what's asked for), Cancel a sign-in in progress, or Disconnect.
pub fn toggle_from_tray(app: &AppHandle) {
    if DIALOG_OPEN.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let status = app.state::<Google>().lock().status;
        match status {
            GoogleStatus::Connecting => {
                app.state::<Google>().lock().cancel.store(true, Ordering::SeqCst);
            }
            GoogleStatus::Connected => {
                if message(
                    "Disconnect Google?\n\nCay revokes its access at Google, then deletes the saved sign-in (Windows \
                     Credential Manager) and its cache of your upcoming events.",
                    MB_OKCANCEL | MB_ICONQUESTION,
                ) == IDOK
                {
                    disconnect(&app);
                }
            }
            GoogleStatus::Disconnected | GoogleStatus::Reconnect => {
                if app.state::<Google>().lock().client.is_none() {
                    message(no_client_text(), MB_OK | MB_ICONINFORMATION);
                } else if message(
                    &connect_text(app.state::<Google>().lock().settings.gmail),
                    MB_OKCANCEL | MB_ICONQUESTION,
                ) == IDOK
                {
                    if let Err(e) = connect(&app) {
                        message(&format!("Cay couldn't start Google's sign-in:\n\n{e}"), MB_OK | MB_ICONWARNING);
                    }
                }
            }
        }
        DIALOG_OPEN.store(false, Ordering::SeqCst);
    });
}

fn connect_text(gmail: bool) -> String {
    let mut text = String::from(
        "Cay will open Google's sign-in in your browser and ask to:\n\n\
         •  See the events on your calendars\n\
         •  See the list of calendars you're subscribed to\n\
         •  See your tasks\n",
    );
    if gmail {
        text.push_str(
            "•  Read your email, for agent runs, each time you allow it in the island\n\
             •  Write email drafts, which you send yourself (Google's wording also says \"send\": Cay never \
             sends)\n",
        );
    }
    text.push_str(if gmail {
        "\nCay can't change or delete anything, and never sends mail."
    } else {
        "\nRead-only: Cay can't change, send or delete anything."
    });
    text.push_str(
        " Your data goes from Google to this PC only; there's no Cay server. The sign-in is kept in Windows \
         Credential Manager, and Disconnect removes it.\n\n\
         Google may say it hasn't verified Cay yet: choose Advanced, then Go to Cay.",
    );
    text
}

fn message(
    text: &str,
    style: MESSAGEBOX_STYLE,
) -> windows::Win32::UI::WindowsAndMessaging::MESSAGEBOX_RESULT {
    // SAFETY: both strings are valid for the call; no owner window, so it never involves the island.
    unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(text),
            &HSTRING::from("Cay"),
            style | MB_SETFOREGROUND | MB_TOPMOST,
        )
    }
}

// ---- Agent tools (6c) ----

/// A tool call's result: what Claude gets, what the audit keeps (counts and ids, never content), how the messages
/// it saw are named in later approvals, and whether it carried mail into the run.
#[derive(Debug)]
pub struct ToolOutput {
    pub value: serde_json::Value,
    pub audit: serde_json::Value,
    pub labels: Vec<MailLabel>,
    pub read_mail: bool,
}

#[derive(Debug)]
pub struct ToolError {
    pub code: &'static str,
    pub message: String,
}

impl ToolError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// How long a tool call waits for the watcher to lend it an access token (it may be mid-refresh).
const TOKEN_WAIT: Duration = Duration::from_secs(30);

/// Runs one approved tool call for an agent run, on the caller's thread (blocking; never the main thread). A 401
/// asks the watcher for a fresh token and tries once more.
pub fn run_tool(app: &AppHandle, call: &tools::ToolCall) -> Result<ToolOutput, ToolError> {
    let google = app.state::<Google>();
    let (calendars, gmail_on) = {
        let inner = google.lock();
        if inner.status != GoogleStatus::Connected {
            return Err(ToolError::new(
                "notConnected",
                "Google isn't connected in Cay. The user can connect it from Cay's tray (Connect Google…).",
            ));
        }
        if let Some(missing) = call.scopes().iter().find(|s| !inner.scopes.contains(**s)) {
            return Err(ToolError::new(
                "notGranted",
                not_granted(missing, inner.settings.gmail),
            ));
        }
        (inner.settings.calendars.clone(), inner.settings.gmail)
    };
    if call.is_mail() && !gmail_on {
        return Err(ToolError::new(
            "notGranted",
            not_granted(auth::SCOPE_GMAIL_READ, false),
        ));
    }
    let http = Client::new().map_err(|e| ToolError::new("network", e.to_string()))?;
    let mut stale = None;
    for attempt in 0..2 {
        let token = lend_token(&google, stale.take())?;
        match call_with(&http, &token, call, calendars.as_deref()) {
            Err(FetchError::Unauthorized) if attempt == 0 => stale = Some(token),
            Err(FetchError::Unauthorized) => {
                return Err(ToolError::new(
                    "notConnected",
                    "Google refused Cay's sign-in. The user can Reconnect Google from Cay's tray.",
                ))
            }
            Err(FetchError::Net(e)) => {
                return Err(ToolError::new(
                    "network",
                    format!("couldn't reach Google: {e}"),
                ))
            }
            Err(FetchError::Http(status, message)) => {
                return Err(ToolError::new(
                    "google",
                    format!("Google answered {status}: {message}"),
                ))
            }
            Ok(out) => return Ok(out),
        }
    }
    unreachable!("the second attempt always returns")
}

fn not_granted(scope: &str, gmail_on: bool) -> String {
    if scope.contains("gmail") && !gmail_on {
        "Gmail isn't turned on in Cay. The user can set \"gmail\": true in the google block of Cay's settings.json, \
         restart Cay and Reconnect Google from the tray."
            .into()
    } else if scope.contains("gmail") {
        "Cay's Google sign-in doesn't include Gmail. The user can Reconnect Google from Cay's tray and allow it.".into()
    } else {
        "Cay's Google sign-in doesn't include this. The user can Reconnect Google from Cay's tray and allow it.".into()
    }
}

fn lend_token(google: &Google, stale: Option<String>) -> Result<String, ToolError> {
    let (tx, rx) = mpsc::channel();
    google.wake(Wake::Token { stale, reply: tx });
    match rx.recv_timeout(TOKEN_WAIT) {
        Ok(Ok(token)) => Ok(token),
        Ok(Err(why)) => Err(ToolError::new("notConnected", why)),
        Err(_) => Err(ToolError::new(
            "network",
            "Cay's Google watcher didn't answer in time",
        )),
    }
}

fn call_with(
    http: &Client,
    token: &str,
    call: &tools::ToolCall,
    calendar_ids: Option<&[String]>,
) -> Result<ToolOutput, FetchError> {
    use serde_json::json;
    use tools::ToolCall;
    let plain = |value: serde_json::Value, audit: serde_json::Value| ToolOutput {
        value,
        audit,
        labels: vec![],
        read_mail: false,
    };
    match call {
        ToolCall::Events {
            from,
            to,
            query,
            max,
        } => {
            let now = now_ms();
            let (start, end) = tools::range(from.as_deref(), to.as_deref(), now, offset)
                .map_err(|e| FetchError::Http(400, e))?;
            let all = calendar::calendars(http, token)?;
            let mut events = Vec::new();
            for c in calendar::chosen(&all, calendar_ids) {
                events.extend(calendar::range(
                    http,
                    token,
                    &c,
                    (start, end),
                    query.as_deref(),
                    *max,
                )?);
            }
            let (events, more) = tools::merge_events(events, *max);
            let audit = json!({ "events": events.len(), "more": more });
            Ok(plain(
                json!({ "from": logic::format_rfc3339(start), "to": logic::format_rfc3339(end),
                        "events": events, "more": more }),
                audit,
            ))
        }
        ToolCall::Tasks { due } => {
            let today = logic::local_day(now_ms(), offset);
            let mut out = Vec::new();
            for l in tasks::lists(http, token)? {
                let raw = tasks::open(http, token, &l, tools::due_max(*due, today))?;
                out.extend(tools::agent_tasks(raw, &l, *due, today));
            }
            let audit = json!({ "tasks": out.len() });
            Ok(plain(
                json!({ "today": logic::format_date(today), "tasks": out }),
                audit,
            ))
        }
        ToolCall::MailSearch { query, max } => {
            let (messages, estimate) = gmail::search(http, token, query, *max)?;
            Ok(ToolOutput {
                audit: json!({ "messages": messages.len(),
                               "ids": messages.iter().map(|m| m.id.as_str()).collect::<Vec<_>>() }),
                labels: messages.iter().map(mail::Message::label).collect(),
                value: json!({ "messages": messages.iter().map(mail::Message::summary).collect::<Vec<_>>(),
                               "estimate": estimate }),
                read_mail: true,
            })
        }
        ToolCall::MailRead { id } => {
            let m = gmail::read(http, token, id)?;
            let full = m.full();
            Ok(ToolOutput {
                audit: json!({ "id": m.id, "bodyCut": full["bodyCut"],
                               "attachments": full["attachments"].as_array().map_or(0, Vec::len) }),
                labels: vec![m.label()],
                value: json!({ "message": full }),
                read_mail: true,
            })
        }
        ToolCall::MailDraft(d) => {
            let (draft, subject, to) = gmail::draft(http, token, d)?;
            Ok(plain(
                json!({ "draftId": draft.id, "messageId": draft.message.id, "to": to, "subject": subject,
                        "note": "Saved as a draft in Gmail (Drafts). It is not sent: the user reviews and sends it." }),
                json!({ "draftId": draft.id, "to": to.len(), "reply": d.reply_to.is_some() }),
            ))
        }
    }
}

// ---- Commands ----

#[tauri::command]
pub fn google_state(google: State<'_, Google>) -> GoogleState {
    google.lock().state()
}

#[tauri::command]
pub fn calendar_upcoming(google: State<'_, Google>) -> CalendarUpcoming {
    google.lock().upcoming()
}

/// Opens Google's sign-in in the browser; `google:state` follows it.
#[tauri::command]
pub fn google_connect(app: AppHandle) -> AppResult<()> {
    connect(&app)
}

/// Revokes and forgets the sign-in (Phase 4b's settings window confirms first; the tray asks itself).
#[tauri::command]
pub async fn google_disconnect(app: AppHandle) -> AppResult<()> {
    tauri::async_runtime::spawn_blocking(move || disconnect(&app))
        .await
        .map_err(|e| AppError::Google(e.to_string()))
}

/// Opens an event (or a task) in the browser.
#[tauri::command]
pub fn calendar_open(google: State<'_, Google>, id: String) -> AppResult<()> {
    let link = {
        let inner = google.lock();
        inner
            .shown
            .events
            .iter()
            .find(|e| e.item.id == id)
            .map(|e| {
                e.open
                    .clone()
                    .unwrap_or_else(|| "https://calendar.google.com/".into())
            })
            .or_else(|| {
                inner.shown.tasks.iter().find(|t| t.item.id == id).map(|t| {
                    t.open
                        .clone()
                        .unwrap_or_else(|| "https://tasks.google.com/".into())
                })
            })
    };
    open(link.ok_or(AppError::CalendarItemGone)?)?;
    google.retract(&id);
    Ok(())
}

/// Opens an event's Meet (or other meeting) link.
#[tauri::command]
pub fn calendar_join(google: State<'_, Google>, id: String) -> AppResult<()> {
    let link = google
        .lock()
        .shown
        .events
        .iter()
        .find(|e| e.item.id == id)
        .and_then(|e| e.join.clone());
    open(link.ok_or(AppError::CalendarItemGone)?)?;
    google.retract(&id);
    Ok(())
}

fn open(link: String) -> AppResult<()> {
    auth::open_browser(&link).map_err(|e| AppError::Google(e.to_string()))
}

/// Debug builds: `cay.exe google-test state | refresh | connect | disconnect`, results in the log.
#[cfg(debug_assertions)]
pub fn debug_command(app: &AppHandle, args: &[String]) {
    let google = app.state::<Google>();
    match args.first().map(String::as_str) {
        Some("refresh") => google.wake(Wake::Refresh),
        Some("connect") => {
            if let Err(e) = connect(app) {
                log::warn!("google-test connect: {e}");
            }
        }
        Some("disconnect") => {
            let app = app.clone();
            std::thread::spawn(move || disconnect(&app));
        }
        // `tool <name> '<json args>'` (6c): one agent tool call without Claude or approvals; the log gets what the
        // audit would (counts, ids, errors), never the mail, events or tasks.
        Some("tool") => {
            let name = args.get(1).cloned().unwrap_or_default();
            let parsed = serde_json::from_str(args.get(2).map_or("{}", String::as_str))
                .map_err(|e| e.to_string())
                .and_then(|v| tools::parse(&name, &v));
            match parsed {
                Err(e) => log::warn!("google-test tool {name}: {e}"),
                Ok(call) => {
                    let app = app.clone();
                    std::thread::spawn(move || match run_tool(&app, &call) {
                        Ok(out) => log::info!(
                            "google-test tool {name}: ok, read mail {}, {}",
                            out.read_mail,
                            out.audit
                        ),
                        Err(e) => log::warn!("google-test tool {name}: {}: {}", e.code, e.message),
                    });
                }
            }
        }
        _ => {}
    }
    let inner = google.lock();
    log::info!("google-test: {:?}", inner.state());
    for e in &inner.shown.events {
        log::info!(
            "google-test: event {} {:?} {} join {}",
            logic::format_rfc3339(e.item.start_ms),
            e.item.phase,
            e.item.title.chars().count(),
            e.join.is_some()
        );
    }
    log::info!("google-test: {} tasks due", inner.shown.tasks.len());
}
