//! Gmail messages for agent runs (Phase 6c): Google's message JSON → what Claude reads, and a draft as RFC 2822.
//! Pure; unit-tested with synthetic fixtures.
//!
//! Mail is untrusted input: anyone can send you a message that says "ignore your instructions". The text goes to
//! Claude marked as such (`agent::google_tools`), and the run counts as having read mail from then on.

use base64::engine::general_purpose::{STANDARD, URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine;
use serde::Deserialize;
use serde_json::{json, Value};

/// The most of one message's text Claude gets.
pub const MAX_BODY_CHARS: usize = 30_000;
const MAX_SNIPPET_CHARS: usize = 300;
const MAX_HEADER_CHARS: usize = 300;
/// At most this many recipients in one draft.
const MAX_RECIPIENTS: usize = 20;
pub const MAX_SUBJECT_CHARS: usize = 250;
pub const MAX_DRAFT_CHARS: usize = 50_000;

// ---- Google's JSON (only the fields Cay reads) ----

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MessageList {
    pub messages: Vec<MessageRef>,
    pub result_size_estimate: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MessageRef {
    pub id: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Message {
    pub id: String,
    pub thread_id: String,
    pub label_ids: Vec<String>,
    pub snippet: String,
    pub payload: Part,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Part {
    pub mime_type: String,
    pub filename: String,
    pub headers: Vec<Header>,
    pub body: Body,
    pub parts: Vec<Part>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Header {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Body {
    pub size: u64,
    /// base64url; absent for attachments (they have an `attachmentId`).
    pub data: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Draft {
    pub id: String,
    pub message: MessageRef,
}

// ---- What Claude gets ----

/// How a message is named in an approval ("Invoice 42 · from Bob"), learned from a search or a read.
#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    pub id: String,
    pub text: String,
}

impl Message {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.payload
            .headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case(name))
            .map(|h| h.value.as_str())
    }

    fn short_header(&self, name: &str) -> String {
        one_line(self.header(name).unwrap_or(""), MAX_HEADER_CHARS)
    }

    pub fn label(&self) -> Label {
        let subject = self.short_header("Subject");
        let from = display_name(self.header("From").unwrap_or(""));
        let subject = if subject.is_empty() {
            "(no subject)".to_string()
        } else {
            one_line(&subject, 60)
        };
        let text = if from.is_empty() {
            format!("\"{subject}\"")
        } else {
            format!("\"{subject}\" from {}", one_line(&from, 40))
        };
        Label {
            id: self.id.clone(),
            text,
        }
    }

    /// One search result: headers and Gmail's snippet, no body.
    pub fn summary(&self) -> Value {
        json!({
            "id": self.id,
            "threadId": self.thread_id,
            "from": self.short_header("From"),
            "to": self.short_header("To"),
            "subject": self.short_header("Subject"),
            "date": self.short_header("Date"),
            "snippet": one_line(&entities(&self.snippet), MAX_SNIPPET_CHARS),
            "unread": self.label_ids.iter().any(|l| l == "UNREAD"),
        })
    }

    /// A whole message: headers, the text (plain if there is one, else the HTML as text), attachment names.
    pub fn full(&self) -> Value {
        let (text, cut) = cut(&self.text(), MAX_BODY_CHARS);
        let mut attachments = Vec::new();
        attachments_of(&self.payload, &mut attachments);
        json!({
            "id": self.id,
            "threadId": self.thread_id,
            "from": self.short_header("From"),
            "to": self.short_header("To"),
            "cc": self.short_header("Cc"),
            "subject": self.short_header("Subject"),
            "date": self.short_header("Date"),
            "attachments": attachments,
            "body": text,
            "bodyCut": cut,
        })
    }

    fn text(&self) -> String {
        if let Some(plain) = find_text(&self.payload, "text/plain") {
            return tidy(&plain);
        }
        find_text(&self.payload, "text/html")
            .map(|html| tidy(&html_to_text(&html)))
            .unwrap_or_default()
    }
}

/// The first non-attachment part of `mime`, decoded (depth-first, as mail clients read `multipart/alternative`).
fn find_text(part: &Part, mime: &str) -> Option<String> {
    if part.mime_type.eq_ignore_ascii_case(mime) && part.filename.is_empty() {
        if let Some(text) = part.body.data.as_deref().and_then(decode_body) {
            return Some(text);
        }
    }
    part.parts.iter().find_map(|p| find_text(p, mime))
}

fn attachments_of(part: &Part, out: &mut Vec<Value>) {
    if !part.filename.is_empty() && out.len() < 50 {
        out.push(
            json!({ "name": one_line(&part.filename, 120), "type": part.mime_type,
                         "bytes": part.body.size }),
        );
    }
    for p in &part.parts {
        attachments_of(p, out);
    }
}

/// Gmail's base64url body (padded or not) → text. Gmail has already undone the transfer encoding.
fn decode_body(data: &str) -> Option<String> {
    let trimmed = data.trim();
    let bytes = URL_SAFE
        .decode(trimmed)
        .or_else(|_| URL_SAFE_NO_PAD.decode(trimmed.trim_end_matches('=')))
        .ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// "Bob Smith <bob@x.com>" → "Bob Smith"; a bare address stays.
fn display_name(from: &str) -> String {
    let from = from.trim();
    match from.split_once('<') {
        Some((name, _)) if !name.trim().is_empty() => name.trim().trim_matches('"').to_string(),
        Some((_, addr)) => addr.trim_end_matches('>').trim().to_string(),
        None => from.to_string(),
    }
}

/// HTML mail → readable text: no scripts, styles or tags; block ends as line breaks; links keep their address.
pub fn html_to_text(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len() / 2);
    let mut i = 0;
    let bytes = html.as_bytes();
    while i < bytes.len() {
        if bytes[i] != b'<' {
            let next = html[i..].find('<').map_or(html.len(), |n| i + n);
            out.push_str(&html[i..next]);
            i = next;
            continue;
        }
        // Comments, scripts and styles go with their content.
        if lower[i..].starts_with("<!--") {
            i = lower[i..].find("-->").map_or(html.len(), |n| i + n + 3);
            continue;
        }
        let end = lower[i..].find('>').map_or(html.len(), |n| i + n + 1);
        let tag = &lower[i..end];
        let name: String = tag
            .trim_start_matches('<')
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        if !tag.starts_with("</") && (name == "script" || name == "style" || name == "head") {
            let close = format!("</{name}");
            i = lower[end..].find(&close).map_or(html.len(), |n| {
                let after = end + n;
                lower[after..]
                    .find('>')
                    .map_or(html.len(), |m| after + m + 1)
            });
            continue;
        }
        match name.as_str() {
            "br" | "p" | "div" | "tr" | "li" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
            | "table" | "blockquote" | "hr" => out.push('\n'),
            "td" | "th" => out.push(' '),
            "a" if !tag.starts_with("</") => {
                if let Some(href) = attribute(&html[i..end], "href")
                    .filter(|h| h.starts_with("http://") || h.starts_with("https://"))
                {
                    out.push_str(&format!("[{href}] "));
                }
            }
            _ => {}
        }
        i = end;
    }
    entities(&out)
}

/// An attribute's value in one tag (`href="…"` or `href='…'`).
fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let at = lower.find(&format!("{name}="))? + name.len() + 1;
    let rest = &tag[at..];
    let quote = rest.chars().next()?;
    let value = if quote == '"' || quote == '\'' {
        rest[1..].split(quote).next()?
    } else {
        rest.split([' ', '>']).next()?
    };
    Some(entities(value.trim()))
}

/// The common HTML entities and numeric ones.
fn entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(semi) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..semi];
        let ch = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some(' '),
            _ => name
                .strip_prefix("#x")
                .or_else(|| name.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| name.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Line ends as `\n`, trailing spaces gone, at most one blank line in a row.
fn tidy(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank = 0;
    for line in text.replace("\r\n", "\n").replace('\r', "\n").lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim().to_string()
}

fn one_line(text: &str, max: usize) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    cut(&joined, max).0
}

/// `text` cut to `max` characters, and whether it was.
fn cut(text: &str, max: usize) -> (String, bool) {
    if text.chars().count() <= max {
        return (text.to_string(), false);
    }
    let mut out: String = text.chars().take(max - 1).collect();
    out.push('…');
    (out, true)
}

// ---- Drafts ----

/// A draft to create: the recipients, subject and text, and the message it answers, if any.
#[derive(Debug, Clone, PartialEq)]
pub struct DraftRequest {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: String,
    pub body: String,
    pub reply_to: Option<String>,
}

/// What a reply takes from the message it answers.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Thread {
    pub thread_id: String,
    pub message_id: Option<String>,
    pub references: Option<String>,
}

impl Thread {
    pub fn of(m: &Message) -> Self {
        Thread {
            thread_id: m.thread_id.clone(),
            message_id: m.header("Message-ID").map(str::trim).map(String::from),
            references: m.header("References").map(str::trim).map(String::from),
        }
    }
}

/// An address is `local@domain` or `Name <local@domain>`, on one line, without the characters that would start
/// another header or address.
pub fn check_address(address: &str) -> Result<String, String> {
    let a = address.trim();
    if a.is_empty() || a.chars().any(|c| c.is_control() || c == ',' || c == ';') {
        return Err(format!("not one email address: {a:?}"));
    }
    let bare = match a.rsplit_once('<') {
        Some((_, rest)) => rest
            .strip_suffix('>')
            .ok_or_else(|| format!("not an email address: {a:?}"))?,
        None => a,
    };
    let (local, domain) = bare
        .split_once('@')
        .ok_or_else(|| format!("not an email address: {a:?}"))?;
    let ok = !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !bare.contains(char::is_whitespace)
        && !domain.contains('@');
    if ok {
        Ok(a.to_string())
    } else {
        Err(format!("not an email address: {a:?}"))
    }
}

/// Checks a draft before anyone is asked about it: addresses, sizes, one-line subject.
pub fn check_draft(d: &DraftRequest) -> Result<(), String> {
    if d.to.is_empty() && d.reply_to.is_none() {
        return Err("a draft needs at least one address in `to` (or replyToId)".into());
    }
    if d.to.len() + d.cc.len() > MAX_RECIPIENTS {
        return Err(format!("at most {MAX_RECIPIENTS} recipients"));
    }
    for a in d.to.iter().chain(&d.cc) {
        check_address(a)?;
    }
    if d.subject.chars().any(char::is_control) {
        return Err("the subject must be one line".into());
    }
    if d.subject.chars().count() > MAX_SUBJECT_CHARS {
        return Err(format!(
            "the subject is over {MAX_SUBJECT_CHARS} characters"
        ));
    }
    if d.body.chars().count() > MAX_DRAFT_CHARS {
        return Err(format!("the text is over {MAX_DRAFT_CHARS} characters"));
    }
    if let Some(id) = &d.reply_to {
        check_id(id)?;
    }
    Ok(())
}

/// Gmail ids are short hex-ish strings; anything else would end up in a URL path.
pub fn check_id(id: &str) -> Result<(), String> {
    let ok = !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric());
    if ok {
        Ok(())
    } else {
        Err(format!("not a Gmail message id: {id:?}"))
    }
}

/// The draft as Gmail's `raw` (base64url RFC 2822): plain UTF-8 text, base64 transfer encoding, and the reply
/// headers when it answers a message. `subject` is the final subject (a reply's "Re: …" already added).
pub fn raw_draft(d: &DraftRequest, subject: &str, thread: Option<&Thread>) -> String {
    let mut m = String::new();
    if !d.to.is_empty() {
        m.push_str(&format!("To: {}\r\n", d.to.join(", ")));
    }
    if !d.cc.is_empty() {
        m.push_str(&format!("Cc: {}\r\n", d.cc.join(", ")));
    }
    m.push_str(&format!("Subject: {}\r\n", encode_header(subject)));
    if let Some(id) = thread
        .and_then(|t| t.message_id.as_deref())
        .filter(|id| safe_header(id))
    {
        m.push_str(&format!("In-Reply-To: {id}\r\n"));
        let refs = thread
            .and_then(|t| t.references.as_deref())
            .filter(|r| safe_header(r))
            .map_or(id.to_string(), |r| format!("{r} {id}"));
        m.push_str(&format!("References: {refs}\r\n"));
    }
    m.push_str("MIME-Version: 1.0\r\n");
    m.push_str("Content-Type: text/plain; charset=UTF-8\r\n");
    m.push_str("Content-Transfer-Encoding: base64\r\n\r\n");
    let body = d.body.replace("\r\n", "\n").replace('\n', "\r\n");
    let encoded = STANDARD.encode(body.as_bytes());
    for chunk in encoded.as_bytes().chunks(76) {
        m.push_str(std::str::from_utf8(chunk).unwrap_or_default());
        m.push_str("\r\n");
    }
    URL_SAFE.encode(m.as_bytes())
}

/// "Re: " + the original subject, unless it already starts with it.
pub fn reply_subject(original: &str) -> String {
    let s = original.trim();
    if s.to_ascii_lowercase().starts_with("re:") {
        s.to_string()
    } else {
        format!("Re: {s}")
    }
}

/// A header value from another message is reused only if it's one printable line.
fn safe_header(v: &str) -> bool {
    !v.is_empty() && v.len() <= 2000 && !v.chars().any(char::is_control)
}

/// ASCII stays as it is; anything else as one RFC 2047 encoded word.
fn encode_header(text: &str) -> String {
    if text.is_ascii() {
        text.to_string()
    } else {
        format!("=?UTF-8?B?{}?=", STANDARD.encode(text.as_bytes()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(file: &str) -> Message {
        let text = match file {
            "plain" => include_str!("fixtures/message_plain.json"),
            _ => include_str!("fixtures/message_html.json"),
        };
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn plain_message() {
        let m = message("plain");
        let full = m.full();
        assert_eq!(full["subject"], "Invoice 42");
        assert_eq!(full["from"], "Bob Smith <bob@example.com>");
        assert_eq!(full["body"], "Hi,\n\nThe invoice is attached.\n\nBob");
        assert_eq!(full["bodyCut"], false);
        assert_eq!(full["attachments"][0]["name"], "invoice-42.pdf");
        assert_eq!(m.label().text, "\"Invoice 42\" from Bob Smith");
        let s = m.summary();
        assert_eq!(s["unread"], true);
        assert_eq!(s["snippet"], "Hi, The invoice is attached. Bob & co");
        assert!(s.get("body").is_none(), "a search never carries the body");
    }

    #[test]
    fn html_only_message_becomes_text() {
        let m = message("html");
        let body = m.full()["body"].as_str().unwrap().to_string();
        assert!(body.starts_with("Your order\n\nShipped today."), "{body}");
        assert!(body.contains("[https://shop.example.com/track?id=1&x=2] Track it"));
        assert!(!body.contains("alert"), "scripts are dropped");
        assert!(!body.contains("color"), "styles are dropped");
        assert!(!body.contains('<'));
        assert_eq!(
            m.label().text,
            "\"Your order\" from orders@shop.example.com"
        );
    }

    #[test]
    fn html_details() {
        assert_eq!(
            html_to_text("a<br>b&nbsp;&amp;&#233;&#x41;&bogus;"),
            "a\nb &éA&bogus;"
        );
        assert_eq!(html_to_text("<!-- hidden -->x<style>p{}</style>y"), "xy");
        assert_eq!(html_to_text("<a href='javascript:x'>no</a>"), "no");
        assert_eq!(tidy("a  \r\n\r\n\r\n\nb\n"), "a\n\nb");
        assert_eq!(display_name("\"Ann\" <a@x.io>"), "Ann");
        assert_eq!(display_name("<a@x.io>"), "a@x.io");
    }

    #[test]
    fn long_bodies_are_cut() {
        let (s, was) = cut(&"x".repeat(10), 5);
        assert_eq!((s.as_str(), was), ("xxxx…", true));
    }

    #[test]
    fn addresses() {
        assert!(check_address("bob@example.com").is_ok());
        assert!(check_address("Bob Smith <bob@example.com>").is_ok());
        for bad in [
            "",
            "bob",
            "bob@localhost",
            "a@b.com, c@d.com",
            "a@b.com\r\nBcc: x@y.com",
            "Bob <bob@example.com",
            "a b@c.com",
            "a@b.c@d.com",
        ] {
            assert!(check_address(bad).is_err(), "{bad:?}");
        }
    }

    fn request() -> DraftRequest {
        DraftRequest {
            to: vec!["Bob <bob@example.com>".into()],
            cc: vec![],
            subject: "Café".into(),
            body: "Line 1\nLine 2".into(),
            reply_to: None,
        }
    }

    #[test]
    fn drafts_are_checked() {
        assert!(check_draft(&request()).is_ok());
        let bad = |f: fn(&mut DraftRequest)| {
            let mut d = request();
            f(&mut d);
            check_draft(&d).is_err()
        };
        assert!(bad(|d| d.to.clear()));
        assert!(bad(|d| d.subject = "a\r\nBcc: x@y.com".into()));
        assert!(bad(|d| d.to = vec!["x@y.com".into(); 21]));
        assert!(bad(|d| d.reply_to = Some("../../drafts".into())));
        assert!(bad(|d| d.body = "x".repeat(MAX_DRAFT_CHARS + 1)));
        let mut reply = request();
        reply.to.clear();
        reply.reply_to = Some("18c2f0a1b2c3d4e5".into());
        assert!(check_draft(&reply).is_ok());
    }

    #[test]
    fn raw_draft_is_rfc_2822() {
        let thread = Thread {
            thread_id: "t1".into(),
            message_id: Some("<abc@mail.example.com>".into()),
            references: Some("<root@mail.example.com>".into()),
        };
        let raw = raw_draft(&request(), &reply_subject("Café"), Some(&thread));
        let text = String::from_utf8(URL_SAFE.decode(raw).unwrap()).unwrap();
        let (head, body) = text.split_once("\r\n\r\n").unwrap();
        assert!(head.starts_with("To: Bob <bob@example.com>\r\n"));
        assert!(head.contains("Subject: =?UTF-8?B?UmU6IENhZsOp?=\r\n"));
        assert!(head.contains("In-Reply-To: <abc@mail.example.com>\r\n"));
        assert!(head.contains("References: <root@mail.example.com> <abc@mail.example.com>\r\n"));
        assert!(!head.contains("Bcc"));
        let decoded = STANDARD
            .decode(body.replace("\r\n", ""))
            .map(String::from_utf8)
            .unwrap()
            .unwrap();
        assert_eq!(decoded, "Line 1\r\nLine 2");
        // A header from the other message that isn't one line is left out.
        let odd = Thread {
            message_id: Some("<a@b>\r\nBcc: x@y.com".into()),
            ..thread
        };
        let text = String::from_utf8(
            URL_SAFE
                .decode(raw_draft(&request(), "Hi", Some(&odd)))
                .unwrap(),
        )
        .unwrap();
        assert!(!text.contains("In-Reply-To") && !text.contains("Bcc"));
        assert_eq!(reply_subject("RE: x"), "RE: x");
    }

    #[test]
    fn ids() {
        assert!(check_id("18c2f0a1b2c3d4e5").is_ok());
        assert!(check_id("a/b").is_err());
        assert!(check_id("").is_err());
    }
}
