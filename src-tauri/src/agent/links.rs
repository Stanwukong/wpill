//! Links in the command window's transcript (2.10a): a Markdown link in Claude's text opens in your browser, https
//! only. Pure and unit-tested; `agent_open_link` in `mod.rs` opens what this accepts.

use crate::desktop::plan::{launch_url, ActError};

/// The link to open, or why not. Built on `launch`'s checks (no spaces, a host, ≤2048 chars), narrowed to https:
/// a transcript is someone else's text, so no http, `ms-settings:`, `file:` or anything that runs.
pub fn checked(url: &str) -> Result<String, String> {
    let url = launch_url(url).map_err(why)?;
    if url.len() > 8 && url[..8].eq_ignore_ascii_case("https://") {
        Ok(url)
    } else {
        Err("Cay only opens https links from a transcript.".into())
    }
}

/// An error from the shell's checks or open, in words.
pub fn why(e: ActError) -> String {
    match e {
        ActError::Refused(why) | ActError::Failed(why) => why,
        other => format!("Couldn't open the link ({}).", other.code()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_only() {
        assert_eq!(
            checked(" https://docs.rs/tauri ").unwrap(),
            "https://docs.rs/tauri"
        );
        assert_eq!(checked("HTTPS://x.dev").unwrap(), "HTTPS://x.dev");
        for bad in [
            "http://example.com",
            "ms-settings:display",
            "file:///C:/Windows/notepad.exe",
            "javascript:alert(1)",
            "https://",
            "https://@evil",
            "https://a b.com",
            r"C:\Windows",
            "",
        ] {
            assert!(checked(bad).is_err(), "{bad}");
        }
    }
}
