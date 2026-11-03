//! A small blocking HTTPS client on WinHTTP: Windows' own TLS, certificate store and proxy settings, so Cay ships
//! no TLS stack of its own (D42). Used only from Cay's own threads (`google-watcher`, `google-auth`).

use std::ffi::c_void;

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Networking::WinHttp::{
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders,
    WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetOption,
    WinHttpSetTimeouts, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_DECOMPRESSION_FLAG_DEFLATE,
    WINHTTP_DECOMPRESSION_FLAG_GZIP, WINHTTP_FLAG_SECURE, WINHTTP_OPEN_REQUEST_FLAGS,
    WINHTTP_OPTION_DECOMPRESSION, WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
};

/// Larger responses are refused (a 24 h window of events is a few KB).
const MAX_BODY: usize = 4 * 1024 * 1024;
const HTTPS_PORT: u16 = 443;

#[derive(Debug)]
pub struct Response {
    pub status: u32,
    pub body: Vec<u8>,
}

impl Response {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// The request never got an answer: no network, DNS, TLS, a timeout. The caller backs off and tries again.
#[derive(Debug)]
pub struct NetError(pub String);

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why an API call failed.
#[derive(Debug)]
pub enum FetchError {
    Net(NetError),
    /// 401: the access token expired early or was revoked; refresh and try once more.
    Unauthorized,
    /// 403 for a scope you unticked on Google's consent screen, quota, or anything else Google refused.
    Http(u32, String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Net(e) => write!(f, "couldn't reach Google: {e}"),
            Self::Unauthorized => f.write_str("Google refused Cay's sign-in"),
            Self::Http(status, message) => write!(f, "Google answered {status}: {message}"),
        }
    }
}

impl From<NetError> for FetchError {
    fn from(e: NetError) -> Self {
        Self::Net(e)
    }
}

/// A WinHTTP handle, closed on drop.
struct Internet(*mut c_void);

impl Drop for Internet {
    fn drop(&mut self) {
        // SAFETY: a handle WinHTTP returned to us and nobody else closes.
        let _ = unsafe { WinHttpCloseHandle(self.0) };
    }
}

fn handle(raw: *mut c_void) -> Result<Internet, NetError> {
    if raw.is_null() {
        Err(NetError(windows::core::Error::from_thread().message()))
    } else {
        Ok(Internet(raw))
    }
}

fn net(e: windows::core::Error) -> NetError {
    NetError(e.message())
}

/// One WinHTTP session; make one per thread.
pub struct Client {
    session: Internet,
}

impl Client {
    pub fn new() -> Result<Self, NetError> {
        let agent = HSTRING::from(format!("Cay/{}", env!("CARGO_PKG_VERSION")));
        // SAFETY: a synchronous session with the system's proxy settings; the strings outlive the call.
        let session = handle(unsafe {
            WinHttpOpen(
                &agent,
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                PCWSTR::null(),
                PCWSTR::null(),
                0,
            )
        })?;
        let flags: u32 = WINHTTP_DECOMPRESSION_FLAG_GZIP | WINHTTP_DECOMPRESSION_FLAG_DEFLATE;
        // SAFETY: the option takes a u32 by pointer; `flags` outlives the call. Failing only costs compression.
        let _ = unsafe {
            WinHttpSetOption(
                Some(session.0),
                WINHTTP_OPTION_DECOMPRESSION,
                Some(std::slice::from_raw_parts(
                    (&flags as *const u32).cast::<u8>(),
                    4,
                )),
            )
        };
        // SAFETY: plain timeouts in ms on our session: resolve, connect, send, receive.
        unsafe { WinHttpSetTimeouts(session.0, 10_000, 10_000, 15_000, 20_000) }.map_err(net)?;
        Ok(Self { session })
    }

    pub fn get(&self, host: &str, path: &str, bearer: &str) -> Result<Response, NetError> {
        let auth = format!("Authorization: Bearer {bearer}\r\nAccept: application/json\r\n");
        self.request("GET", host, path, &auth, None)
    }

    /// A Google API GET, parsed as `T`.
    pub fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        host: &str,
        path: &str,
        bearer: &str,
    ) -> Result<T, FetchError> {
        let r = self.get(host, path, bearer)?;
        match r.status {
            200..=299 => serde_json::from_slice(&r.body)
                .map_err(|e| FetchError::Http(r.status, format!("unreadable reply: {e}"))),
            401 => Err(FetchError::Unauthorized),
            status => Err(FetchError::Http(status, google_message(&r.body))),
        }
    }

    /// A Google API POST of a JSON body, parsed as `T` (Gmail drafts).
    pub fn post_json<T: serde::de::DeserializeOwned>(
        &self,
        host: &str,
        path: &str,
        bearer: &str,
        body: &serde_json::Value,
    ) -> Result<T, FetchError> {
        let headers = format!(
            "Authorization: Bearer {bearer}\r\nContent-Type: application/json\r\nAccept: application/json\r\n"
        );
        let body = body.to_string();
        let r = self.request("POST", host, path, &headers, Some(body.as_bytes()))?;
        match r.status {
            200..=299 => serde_json::from_slice(&r.body)
                .map_err(|e| FetchError::Http(r.status, format!("unreadable reply: {e}"))),
            401 => Err(FetchError::Unauthorized),
            status => Err(FetchError::Http(status, google_message(&r.body))),
        }
    }

    /// An `application/x-www-form-urlencoded` POST (the token endpoints).
    pub fn post_form(&self, host: &str, path: &str, form: &str) -> Result<Response, NetError> {
        let headers =
            "Content-Type: application/x-www-form-urlencoded\r\nAccept: application/json\r\n";
        self.request("POST", host, path, headers, Some(form.as_bytes()))
    }

    fn request(
        &self,
        method: &str,
        host: &str,
        path: &str,
        headers: &str,
        body: Option<&[u8]>,
    ) -> Result<Response, NetError> {
        let host = HSTRING::from(host);
        // SAFETY: a connection handle under our session; the host string outlives the call.
        let connection = handle(unsafe { WinHttpConnect(self.session.0, &host, HTTPS_PORT, 0) })?;
        let (method, path) = (HSTRING::from(method), HSTRING::from(path));
        // SAFETY: a request handle under the connection; strings outlive the call; no accept-types list.
        let request = handle(unsafe {
            WinHttpOpenRequest(
                connection.0,
                &method,
                &path,
                PCWSTR::null(),
                PCWSTR::null(),
                std::ptr::null(),
                WINHTTP_OPEN_REQUEST_FLAGS(WINHTTP_FLAG_SECURE.0),
            )
        })?;
        let headers: Vec<u16> = headers.encode_utf16().collect();
        let length = body.map_or(0, |b| b.len() as u32);
        // SAFETY: headers and body stay alive until the synchronous send returns.
        unsafe {
            WinHttpSendRequest(
                request.0,
                Some(&headers),
                body.map(|b| b.as_ptr().cast::<c_void>()),
                length,
                length,
                0,
            )
        }
        .map_err(net)?;
        // SAFETY: waits for the response on our request handle.
        unsafe { WinHttpReceiveResponse(request.0, std::ptr::null_mut()) }.map_err(net)?;

        let mut status = 0u32;
        let mut size = 4u32;
        // SAFETY: the status code as a number into a u32 we own; `size` says how big it is.
        unsafe {
            WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                PCWSTR::null(),
                Some((&mut status as *mut u32).cast()),
                &mut size,
                std::ptr::null_mut(),
            )
        }
        .map_err(net)?;

        let mut out = Vec::new();
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let mut read = 0u32;
            // SAFETY: reads at most `buf.len()` bytes into `buf`.
            unsafe {
                WinHttpReadData(
                    request.0,
                    buf.as_mut_ptr().cast(),
                    buf.len() as u32,
                    &mut read,
                )
            }
            .map_err(net)?;
            if read == 0 {
                break;
            }
            out.extend_from_slice(&buf[..read as usize]);
            if out.len() > MAX_BODY {
                return Err(NetError("the response was too large".into()));
            }
        }
        Ok(Response { status, body: out })
    }
}

/// `{"error":{"message":…}}` → the message (short), else a generic note.
fn google_message(body: &[u8]) -> String {
    let message = serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(String::from))
        .unwrap_or_else(|| "no details".into());
    message.chars().take(200).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live network, no account: `cargo test -- --ignored winhttp`. A public discovery document, then the Calendar
    /// API with a made-up token (Google answers 401, nothing is signed in).
    #[test]
    #[ignore]
    fn winhttp_reaches_google() {
        let http = Client::new().unwrap();
        let r = http
            .get(
                "www.googleapis.com",
                "/discovery/v1/apis/calendar/v3/rest?fields=title",
                "",
            )
            .unwrap();
        assert!(r.ok(), "{} {}", r.status, r.text());
        assert!(r.text().contains("Calendar"));
        let e = http
            .get_json::<serde_json::Value>(
                "www.googleapis.com",
                "/calendar/v3/users/me/calendarList",
                "not-a-token",
            )
            .unwrap_err();
        assert!(matches!(e, FetchError::Unauthorized), "{e}");
    }
}
