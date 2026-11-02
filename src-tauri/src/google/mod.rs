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
mod http;
mod logic;
mod network;
mod settings;
mod tasks;
mod vault;

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

use crate::activity::{Activities, Watcher};
use crate::error::{AppError, AppResult};
use crate::ipc::{events, CalendarUpcoming, GoogleState, GoogleStatus};
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
        }))
    }
}

impl Google {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
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
    publish(app, now_ms());
    loop {
        let now = now_ms();
        let (soon, refresh) = {
            let inner = google.lock();
            (
                i64::from(inner.settings.soon_minutes) * MINUTE_MS,
                i64::from(inner.settings.refresh_minutes) * MINUTE_MS,
            )
        };
        let boundary = logic::next_boundary(&google.lock().fetched, now, soon, offset);
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
    let (client, cancel) = {
        let mut inner = google.lock();
        let Some(client) = inner.client.clone() else {
            return Err(AppError::Google(no_client_text().into()));
        };
        if inner.signing_in {
            return Ok(());
        }
        inner.signing_in = true;
        inner.cancel.store(false, Ordering::SeqCst);
        (client, inner.cancel.clone())
    };
    let previous = google.lock().status;
    set_status(app, GoogleStatus::Connecting, None);
    let app = app.clone();
    std::thread::Builder::new()
        .name("google-auth".into())
        .spawn(move || {
            let result = Client::new()
                .map_err(AuthError::Net)
                .and_then(|http| auth::sign_in(&http, &client, &cancel));
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
                } else if message(&connect_text(), MB_OKCANCEL | MB_ICONQUESTION) == IDOK {
                    if let Err(e) = connect(&app) {
                        message(&format!("Cay couldn't start Google's sign-in:\n\n{e}"), MB_OK | MB_ICONWARNING);
                    }
                }
            }
        }
        DIALOG_OPEN.store(false, Ordering::SeqCst);
    });
}

fn connect_text() -> String {
    "Cay will open Google's sign-in in your browser and ask to:\n\n\
     •  See the events on your calendars\n\
     •  See the list of calendars you're subscribed to\n\
     •  See your tasks\n\n\
     Read-only: Cay can't change, send or delete anything. Your events go from Google to this PC only; there's no Cay \
     server. The sign-in is kept in Windows Credential Manager, and Disconnect removes it.\n\n\
     Google may say it hasn't verified Cay yet: choose Advanced, then Go to Cay."
        .into()
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
    open(link.ok_or(AppError::CalendarItemGone)?)
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
    open(link.ok_or(AppError::CalendarItemGone)?)
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
