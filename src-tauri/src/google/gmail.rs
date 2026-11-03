//! Gmail API v1 for agent runs (Phase 6c): search, read one message, create a draft. There is no send call: drafts
//! wait in Gmail for you.

use serde_json::json;

use super::http::{Client, FetchError};
use super::logic::encode;
use super::mail::{self, Draft, DraftRequest, Message, MessageList, Thread};

const HOST: &str = "gmail.googleapis.com";
const HEADERS: [&str; 4] = ["From", "To", "Subject", "Date"];

/// Messages matching `query` (Gmail's search syntax), newest first, with headers and snippets only.
pub fn search(
    http: &Client,
    token: &str,
    query: &str,
    max: usize,
) -> Result<(Vec<Message>, Option<u64>), FetchError> {
    let path = format!(
        "/gmail/v1/users/me/messages?q={}&maxResults={max}&fields={}",
        encode(query),
        encode("messages(id),resultSizeEstimate")
    );
    let list: MessageList = http.get_json(HOST, &path, token)?;
    let mut out = Vec::new();
    for m in list.messages.iter().take(max) {
        if mail::check_id(&m.id).is_ok() {
            out.push(metadata(http, token, &m.id)?);
        }
    }
    Ok((out, list.result_size_estimate))
}

fn metadata(http: &Client, token: &str, id: &str) -> Result<Message, FetchError> {
    let mut path = format!(
        "/gmail/v1/users/me/messages/{id}?format=metadata&fields={}",
        encode("id,threadId,labelIds,snippet,payload/headers")
    );
    for h in HEADERS
        .iter()
        .chain(&["Message-ID", "References", "Reply-To"])
    {
        path.push_str(&format!("&metadataHeaders={h}"));
    }
    http.get_json(HOST, &path, token)
}

/// One whole message (`id` already checked by `mail::check_id`).
pub fn read(http: &Client, token: &str, id: &str) -> Result<Message, FetchError> {
    let path = format!(
        "/gmail/v1/users/me/messages/{id}?format=full&fields={}",
        encode("id,threadId,labelIds,snippet,payload")
    );
    http.get_json(HOST, &path, token)
}

/// Creates a draft; a reply goes into the answered message's thread with its subject and reply headers.
/// Returns the draft, its final subject and, for a reply, who it answers.
pub fn draft(
    http: &Client,
    token: &str,
    d: &DraftRequest,
) -> Result<(Draft, String, Vec<String>), FetchError> {
    let original = match &d.reply_to {
        Some(id) => Some(metadata(http, token, id)?),
        None => None,
    };
    let thread = original.as_ref().map(Thread::of);
    let subject = match (&original, d.subject.is_empty()) {
        (Some(m), true) => mail::reply_subject(m.header("Subject").unwrap_or("")),
        _ => d.subject.clone(),
    };
    // A reply with no `to` goes to the sender of the message it answers, if that's a usable address.
    let mut request = d.clone();
    if request.to.is_empty() {
        let from = original
            .as_ref()
            .and_then(|m| m.header("Reply-To").or(m.header("From")))
            .and_then(|f| mail::check_address(f).ok());
        request.to.extend(from);
    }
    if request.to.is_empty() {
        return Err(FetchError::Http(
            400,
            "the message it answers has no usable sender; give `to`".into(),
        ));
    }
    let mut message = json!({ "raw": mail::raw_draft(&request, &subject, thread.as_ref()) });
    if let Some(t) = &thread {
        message["threadId"] = json!(t.thread_id);
    }
    let created: Draft = http.post_json(
        HOST,
        "/gmail/v1/users/me/drafts",
        token,
        &json!({ "message": message }),
    )?;
    Ok((created, subject, request.to))
}
