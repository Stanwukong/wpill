//! Google sign-in for an installed app (D42): PKCE (S256), a one-shot loopback listener on `127.0.0.1`, the system
//! browser (never a webview), then the token exchange. Refresh and revoke for the watcher and Disconnect.
//!
//! A desktop client's "secret" isn't secret (Google: "desktop apps are public clients and cannot keep secrets
//! confidential"); PKCE is what protects the code. Cay takes the client from the build (`CAY_GOOGLE_CLIENT_ID`,
//! `CAY_GOOGLE_CLIENT_SECRET`) or `settings.json` (`google.clientId`), so nothing is committed.

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::Deserialize;
use windows::core::{w, HSTRING};
use windows::Win32::Security::Cryptography::{
    BCryptGenRandom, BCryptHash, BCRYPT_SHA256_ALG_HANDLE, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use super::http::{Client, NetError};
use super::logic::{decode, encode};

pub const SCOPE_CALENDAR_EVENTS: &str = "https://www.googleapis.com/auth/calendar.events.readonly";
pub const SCOPE_CALENDAR_LIST: &str =
    "https://www.googleapis.com/auth/calendar.calendarlist.readonly";
pub const SCOPE_TASKS: &str = "https://www.googleapis.com/auth/tasks.readonly";
/// Read-only Calendar and Tasks: what 6a–b need. Gmail comes only with 6c, and only when you turn it on.
pub const SCOPES: [&str; 3] = [SCOPE_CALENDAR_EVENTS, SCOPE_CALENDAR_LIST, SCOPE_TASKS];

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_HOST: &str = "oauth2.googleapis.com";
/// How long Cay waits for you to finish in the browser.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const MAX_REQUEST: usize = 8 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct ClientConfig {
    pub id: String,
    pub secret: Option<String>,
}

/// The settings' client if set, else the one built in. `None`: this build can't sign in.
pub fn client(id: Option<&str>, secret: Option<&str>) -> Option<ClientConfig> {
    let pick = |s: Option<&str>, built: Option<&'static str>| {
        s.or(built)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    let from_settings = id.is_some_and(|s| !s.trim().is_empty());
    let id = pick(id, option_env!("CAY_GOOGLE_CLIENT_ID"))?;
    let secret = if from_settings {
        pick(secret, None)
    } else {
        pick(None, option_env!("CAY_GOOGLE_CLIENT_SECRET"))
    };
    Some(ClientConfig { id, secret })
}

#[derive(Debug)]
pub enum AuthError {
    Net(NetError),
    /// `invalid_grant`: the refresh token was revoked or expired. Connect again.
    Revoked,
    Google(String),
    Cancelled,
    TimedOut,
    Local(String),
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Net(e) => write!(f, "couldn't reach Google: {e}"),
            Self::Revoked => f.write_str("Google no longer accepts Cay's sign-in"),
            Self::Google(e) => write!(f, "Google said: {e}"),
            Self::Cancelled => f.write_str("sign-in cancelled"),
            Self::TimedOut => f.write_str("sign-in timed out"),
            Self::Local(e) => f.write_str(e),
        }
    }
}

impl From<NetError> for AuthError {
    fn from(e: NetError) -> Self {
        Self::Net(e)
    }
}

#[derive(Debug, Clone)]
pub struct Tokens {
    pub access: String,
    pub expires_at: Instant,
    /// Only the first exchange returns one (refreshes keep the old).
    pub refresh: Option<String>,
    pub scopes: String,
}

#[derive(Deserialize)]
struct TokenReply {
    access_token: String,
    #[serde(default)]
    expires_in: u64,
    refresh_token: Option<String>,
    #[serde(default)]
    scope: String,
}

#[derive(Deserialize)]
struct ErrorReply {
    error: String,
    error_description: Option<String>,
}

/// Opens Google's sign-in in your browser and waits (≤5 min, or until `cancel`) for it to come back.
pub fn sign_in(
    http: &Client,
    client: &ClientConfig,
    cancel: &AtomicBool,
) -> Result<Tokens, AuthError> {
    let verifier = URL_SAFE_NO_PAD.encode(random(32)?);
    let challenge = challenge(&verifier)?;
    let state = URL_SAFE_NO_PAD.encode(random(16)?);
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| AuthError::Local(e.to_string()))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| AuthError::Local(e.to_string()))?;
    let port = listener
        .local_addr()
        .map_err(|e| AuthError::Local(e.to_string()))?
        .port();
    let redirect = format!("http://127.0.0.1:{port}");
    open_browser(&auth_url(&client.id, &redirect, &challenge, &state))?;
    log::info!("google: sign-in opened in the browser, waiting on port {port}");

    let deadline = Instant::now() + SIGN_IN_TIMEOUT;
    let code = loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(AuthError::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(AuthError::TimedOut);
        }
        match listener.accept() {
            Ok((stream, _)) => match answer(stream, &state) {
                Callback::Code(code) => break code,
                Callback::Denied(why) => return Err(AuthError::Google(why)),
                Callback::Ignore => {}
            },
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(150))
            }
            Err(e) => return Err(AuthError::Local(e.to_string())),
        }
    };
    let mut form = format!(
        "grant_type=authorization_code&code={}&client_id={}&redirect_uri={}&code_verifier={}",
        encode(&code),
        encode(&client.id),
        encode(&redirect),
        encode(&verifier)
    );
    push_secret(&mut form, client);
    token_request(http, &form)
}

pub fn refresh(
    http: &Client,
    client: &ClientConfig,
    refresh_token: &str,
) -> Result<Tokens, AuthError> {
    let mut form = format!(
        "grant_type=refresh_token&refresh_token={}&client_id={}",
        encode(refresh_token),
        encode(&client.id)
    );
    push_secret(&mut form, client);
    token_request(http, &form)
}

/// Revokes the token (and with a refresh token, every grant Cay holds). Best effort: Disconnect deletes it anyway.
pub fn revoke(http: &Client, token: &str) {
    match http.post_form(TOKEN_HOST, "/revoke", &format!("token={}", encode(token))) {
        Ok(r) if r.ok() => log::info!("google: token revoked"),
        Ok(r) => log::warn!("google: revoke answered {}: {}", r.status, r.text()),
        Err(e) => log::warn!("google: revoke failed: {e}"),
    }
}

fn push_secret(form: &mut String, client: &ClientConfig) {
    if let Some(secret) = &client.secret {
        form.push_str("&client_secret=");
        form.push_str(&encode(secret));
    }
}

fn token_request(http: &Client, form: &str) -> Result<Tokens, AuthError> {
    let reply = http.post_form(TOKEN_HOST, "/token", form)?;
    parse_tokens(reply.status, &reply.body, Instant::now())
}

fn parse_tokens(status: u32, body: &[u8], now: Instant) -> Result<Tokens, AuthError> {
    if (200..300).contains(&status) {
        let t: TokenReply = serde_json::from_slice(body)
            .map_err(|e| AuthError::Google(format!("unreadable token reply: {e}")))?;
        // A minute early, so a request never goes out with a token that expires on the way.
        let life = Duration::from_secs(t.expires_in.max(120) - 60);
        return Ok(Tokens {
            access: t.access_token,
            expires_at: now + life,
            refresh: t.refresh_token,
            scopes: t.scope,
        });
    }
    match serde_json::from_slice::<ErrorReply>(body) {
        Ok(e) if e.error == "invalid_grant" => Err(AuthError::Revoked),
        Ok(e) => Err(AuthError::Google(match e.error_description {
            Some(d) => format!("{} ({d})", e.error),
            None => e.error,
        })),
        Err(_) => Err(AuthError::Google(format!("HTTP {status}"))),
    }
}

pub fn auth_url(client_id: &str, redirect: &str, challenge: &str, state: &str) -> String {
    format!(
        "{AUTH_URL}?response_type=code&client_id={}&redirect_uri={}&scope={}&code_challenge={}\
         &code_challenge_method=S256&state={}&access_type=offline",
        encode(client_id),
        encode(redirect),
        encode(&SCOPES.join(" ")),
        encode(challenge),
        encode(state)
    )
}

#[derive(Debug, PartialEq)]
pub enum Callback {
    Code(String),
    Denied(String),
    /// Not the redirect (a favicon request, a wrong state): keep waiting.
    Ignore,
}

/// The browser's request line (`GET /?state=…&code=… HTTP/1.1`) → what it says.
pub fn parse_callback(request_line: &str, state: &str) -> Callback {
    let mut parts = request_line.split(' ');
    let (Some("GET"), Some(target)) = (parts.next(), parts.next()) else {
        return Callback::Ignore;
    };
    let Some(query) = target.strip_prefix("/?") else {
        return Callback::Ignore;
    };
    let (mut code, mut error, mut got_state) = (None, None, None);
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        match k {
            "code" => code = Some(decode(v)),
            "error" => error = Some(decode(v)),
            "state" => got_state = Some(decode(v)),
            _ => {}
        }
    }
    // A redirect without our state didn't come from the sign-in we started.
    if got_state.as_deref() != Some(state) {
        return Callback::Ignore;
    }
    match (code, error) {
        (_, Some(e)) => Callback::Denied(e),
        (Some(c), None) if !c.is_empty() => Callback::Code(c),
        _ => Callback::Ignore,
    }
}

fn answer(mut stream: TcpStream, state: &str) -> Callback {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    while buf.len() < MAX_REQUEST && !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    let text = String::from_utf8_lossy(&buf);
    let callback = parse_callback(text.lines().next().unwrap_or(""), state);
    let (status, page) = match &callback {
        Callback::Code(_) => (
            "200 OK",
            "Cay is connected to Google. You can close this tab.",
        ),
        Callback::Denied(_) => (
            "200 OK",
            "Cay wasn't connected to Google. You can close this tab.",
        ),
        Callback::Ignore => ("404 Not Found", "Not found."),
    };
    let html = format!(
        "<!doctype html><meta charset=utf-8><title>Cay</title>\
         <body style=\"font:16px 'Segoe UI',sans-serif;margin:3em\">{page}</body>"
    );
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n{html}",
        html.len()
    );
    callback
}

pub fn open_browser(url: &str) -> Result<(), AuthError> {
    // SAFETY: strings outlive the call; no owner window, so the island is never involved.
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(url),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    // ShellExecute returns a value greater than 32 on success.
    if result.0 as isize > 32 {
        Ok(())
    } else {
        Err(AuthError::Local("couldn't open the browser".into()))
    }
}

fn random(bytes: usize) -> Result<Vec<u8>, AuthError> {
    let mut buf = vec![0u8; bytes];
    // SAFETY: fills `buf`; with the system-preferred flag no algorithm handle is needed.
    unsafe { BCryptGenRandom(None, &mut buf, BCRYPT_USE_SYSTEM_PREFERRED_RNG) }
        .ok()
        .map_err(|e| AuthError::Local(format!("no random numbers: {}", e.message())))?;
    Ok(buf)
}

/// PKCE S256: base64url(SHA-256(verifier)).
fn challenge(verifier: &str) -> Result<String, AuthError> {
    let mut digest = [0u8; 32];
    // SAFETY: the SHA-256 pseudo-handle needs no setup; input and output buffers are ours.
    unsafe {
        BCryptHash(
            BCRYPT_SHA256_ALG_HANDLE,
            None,
            verifier.as_bytes(),
            &mut digest,
        )
    }
    .ok()
    .map_err(|e| AuthError::Local(format!("SHA-256 failed: {}", e.message())))?;
    Ok(URL_SAFE_NO_PAD.encode(digest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_matches_rfc_7636() {
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk").unwrap(),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        let v = URL_SAFE_NO_PAD.encode(random(32).unwrap());
        assert_eq!(v.len(), 43);
    }

    #[test]
    fn the_auth_url_asks_for_read_only_scopes_with_pkce() {
        let url = auth_url(
            "id.apps.googleusercontent.com",
            "http://127.0.0.1:5123",
            "ch",
            "st",
        );
        assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth?response_type=code&"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A5123"));
        assert!(url.contains("code_challenge=ch&code_challenge_method=S256&state=st"));
        assert!(url.contains("calendar.events.readonly%20https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fcalendar.calendarlist.readonly"));
        assert!(!url.contains("gmail"));
    }

    #[test]
    fn callbacks_need_our_state() {
        let ok = "GET /?state=st&code=4%2F0Ab-x&scope=a+b HTTP/1.1";
        assert_eq!(parse_callback(ok, "st"), Callback::Code("4/0Ab-x".into()));
        assert_eq!(parse_callback(ok, "other"), Callback::Ignore);
        assert_eq!(
            parse_callback("GET /?error=access_denied&state=st HTTP/1.1", "st"),
            Callback::Denied("access_denied".into())
        );
        assert_eq!(
            parse_callback("GET /favicon.ico HTTP/1.1", "st"),
            Callback::Ignore
        );
        assert_eq!(
            parse_callback("POST /?state=st&code=x HTTP/1.1", "st"),
            Callback::Ignore
        );
        assert_eq!(
            parse_callback("GET /?state=st&code= HTTP/1.1", "st"),
            Callback::Ignore
        );
    }

    #[test]
    fn token_replies_and_errors() {
        let now = Instant::now();
        let t = parse_tokens(
            200,
            br#"{"access_token":"ya29.x","expires_in":3599,"refresh_token":"1//r","scope":"a b","token_type":"Bearer"}"#,
            now,
        )
        .unwrap();
        assert_eq!(
            (t.access.as_str(), t.refresh.as_deref(), t.scopes.as_str()),
            ("ya29.x", Some("1//r"), "a b")
        );
        assert_eq!(t.expires_at - now, Duration::from_secs(3539));
        assert!(matches!(
            parse_tokens(400, br#"{"error":"invalid_grant","error_description":"Token has been expired or revoked."}"#, now),
            Err(AuthError::Revoked)
        ));
        assert!(
            matches!(parse_tokens(401, br#"{"error":"invalid_client"}"#, now), Err(AuthError::Google(e)) if e == "invalid_client")
        );
        assert!(
            matches!(parse_tokens(503, b"<html>", now), Err(AuthError::Google(e)) if e == "HTTP 503")
        );
    }

    #[test]
    fn the_settings_client_wins_with_its_own_secret() {
        let c = client(Some(" mine.apps.googleusercontent.com "), Some("s")).unwrap();
        assert_eq!(
            (c.id.as_str(), c.secret.as_deref()),
            ("mine.apps.googleusercontent.com", Some("s"))
        );
        let c = client(Some("mine"), None).unwrap();
        assert_eq!(c.secret, None);
    }
}
