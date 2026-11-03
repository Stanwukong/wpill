//! Google for agent runs (Phase 6c): the five tools' arguments, and Google's replies as Claude reads them. Pure;
//! unit-tested with the fixtures. The MCP side (definitions, approvals, replies) is `agent::google_tools`; the
//! fetching is `run_tool` in `mod.rs`.

use serde_json::{json, Value};

use super::auth;
use super::logic::{
    self, is_https_on, join_link, parse_date, parse_rfc3339, title, CalendarEntry, RawEvent,
    RawTask, TaskListEntry, DAY_MS,
};
use super::mail::{self, DraftRequest};

pub const CALENDAR_LIST_EVENTS: &str = "calendar_list_events";
pub const TASKS_LIST: &str = "tasks_list";
pub const GMAIL_SEARCH: &str = "gmail_search";
pub const GMAIL_READ: &str = "gmail_read";
pub const GMAIL_DRAFT: &str = "gmail_draft";
pub const NAMES: &[&str] = &[
    CALENDAR_LIST_EVENTS,
    TASKS_LIST,
    GMAIL_SEARCH,
    GMAIL_READ,
    GMAIL_DRAFT,
];

/// A week ahead unless the call says otherwise; never more than about two months at once.
const DEFAULT_SPAN_MS: i64 = 7 * DAY_MS;
const MAX_SPAN_MS: i64 = 62 * DAY_MS;
pub const MAX_EVENTS: usize = 100;
const DEFAULT_EVENTS: usize = 50;
pub const MAX_SEARCH: usize = 20;
const DEFAULT_SEARCH: usize = 10;
const MAX_QUERY: usize = 500;
const MAX_TASKS: usize = 200;
const MAX_NOTES: usize = 1000;
const MAX_LOCATION: usize = 200;

/// Which open tasks `tasks_list` returns.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TaskDue {
    /// Due today or overdue (like the island).
    Today,
    /// Due on or before this local day.
    By(i64),
    /// Every open task, with or without a date.
    All,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToolCall {
    /// `from`/`to` as given (a date or RFC 3339); resolved against the clock when it runs.
    Events {
        from: Option<String>,
        to: Option<String>,
        query: Option<String>,
        max: usize,
    },
    Tasks {
        due: TaskDue,
    },
    MailSearch {
        query: String,
        max: usize,
    },
    MailRead {
        id: String,
    },
    MailDraft(DraftRequest),
}

impl ToolCall {
    /// The scopes it needs, all of them.
    pub fn scopes(&self) -> &'static [&'static str] {
        match self {
            Self::Events { .. } => &[auth::SCOPE_CALENDAR_LIST, auth::SCOPE_CALENDAR_EVENTS],
            Self::Tasks { .. } => &[auth::SCOPE_TASKS],
            Self::MailSearch { .. } | Self::MailRead { .. } => &[auth::SCOPE_GMAIL_READ],
            // A reply reads the message it answers.
            Self::MailDraft(_) => &[auth::SCOPE_GMAIL_COMPOSE, auth::SCOPE_GMAIL_READ],
        }
    }

    pub fn is_mail(&self) -> bool {
        matches!(
            self,
            Self::MailSearch { .. } | Self::MailRead { .. } | Self::MailDraft(_)
        )
    }
}

/// A `tools/call`'s arguments → the call, or why they're wrong (checked before anyone is asked).
pub fn parse(name: &str, args: &Value) -> Result<ToolCall, String> {
    let text = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    let count = |key: &str, default: usize, max: usize| -> Result<usize, String> {
        match args.get(key).filter(|v| !v.is_null()) {
            None => Ok(default),
            Some(v) => v
                .as_u64()
                .or_else(|| v.as_str()?.trim().parse().ok())
                .map(|n| (n as usize).clamp(1, max))
                .ok_or_else(|| format!("{key} must be a number")),
        }
    };
    match name {
        CALENDAR_LIST_EVENTS => {
            let (from, to) = (text("from"), text("to"));
            for t in [&from, &to].into_iter().flatten() {
                if parse_date(t).is_none() && parse_rfc3339(t).is_none() {
                    return Err(format!(
                        "{t:?} isn't a date (YYYY-MM-DD) or an RFC 3339 time"
                    ));
                }
            }
            let query = text("query");
            if query
                .as_ref()
                .is_some_and(|q| q.chars().count() > MAX_QUERY)
            {
                return Err(format!("the query is over {MAX_QUERY} characters"));
            }
            Ok(ToolCall::Events {
                from,
                to,
                query,
                max: count("maxResults", DEFAULT_EVENTS, MAX_EVENTS)?,
            })
        }
        TASKS_LIST => {
            let due = match text("due").as_deref() {
                None | Some("today") => TaskDue::Today,
                Some("all") => TaskDue::All,
                Some(d) => TaskDue::By(
                    parse_date(d).ok_or("due is \"today\", \"all\" or a date (YYYY-MM-DD)")?,
                ),
            };
            Ok(ToolCall::Tasks { due })
        }
        GMAIL_SEARCH => {
            let query =
                text("query").ok_or("gmail_search needs a query (Gmail's search syntax)")?;
            if query.chars().count() > MAX_QUERY {
                return Err(format!("the query is over {MAX_QUERY} characters"));
            }
            Ok(ToolCall::MailSearch {
                query,
                max: count("maxResults", DEFAULT_SEARCH, MAX_SEARCH)?,
            })
        }
        GMAIL_READ => {
            let id = text("id").ok_or("gmail_read needs a message id from gmail_search")?;
            mail::check_id(&id)?;
            Ok(ToolCall::MailRead { id })
        }
        GMAIL_DRAFT => {
            let list = |key: &str| -> Vec<String> {
                match args.get(key) {
                    Some(Value::String(s)) => s
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(String::from)
                        .collect(),
                    Some(Value::Array(a)) => a
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(String::from)
                        .collect(),
                    _ => vec![],
                }
            };
            let body = args
                .get("body")
                .and_then(Value::as_str)
                .ok_or("gmail_draft needs a body")?
                .to_string();
            let draft = DraftRequest {
                to: list("to"),
                cc: list("cc"),
                subject: text("subject").unwrap_or_default(),
                body,
                reply_to: text("replyToId"),
            };
            mail::check_draft(&draft)?;
            Ok(ToolCall::MailDraft(draft))
        }
        _ => Err("unknown tool".into()),
    }
}

/// `from`/`to` against the clock: a date is local midnight, an RFC 3339 time is itself. Defaults: now, and a week
/// after `from`. The span is at most 62 days.
pub fn range(
    from: Option<&str>,
    to: Option<&str>,
    now: i64,
    offset_min: impl Fn(i64) -> i64,
) -> Result<(i64, i64), String> {
    let at = |t: &str| {
        parse_rfc3339(t).or_else(|| {
            let day = parse_date(t)?;
            let utc = day * DAY_MS;
            Some(utc - offset_min(utc) * logic::MINUTE_MS)
        })
    };
    let start = from
        .map_or(Some(now), at)
        .ok_or("from isn't a date or time")?;
    let end = to
        .map_or(Some(start + DEFAULT_SPAN_MS), at)
        .ok_or("to isn't a date or time")?;
    if end <= start {
        return Err("to must be after from".into());
    }
    if end - start > MAX_SPAN_MS {
        return Err("at most 62 days at once".into());
    }
    Ok((start, end))
}

/// One calendar's events as Claude reads them: all-day ones too, never cancelled ones. No descriptions: anyone can
/// send you an invitation, so its text is someone else's.
pub fn agent_events(raw: Vec<RawEvent>, calendar: &CalendarEntry) -> Vec<(i64, Value)> {
    raw.into_iter()
        .filter(|e| e.status.as_deref() != Some("cancelled") && !e.id.is_empty())
        .filter_map(|e| {
            let all_day = e.start.date_time.is_none();
            let (start, end) = if all_day {
                (
                    e.start.date.clone()?,
                    e.end.date.clone().unwrap_or_default(),
                )
            } else {
                (
                    e.start.date_time.clone()?,
                    e.end.date_time.clone().unwrap_or_default(),
                )
            };
            let sort = parse_rfc3339(&start)
                .or_else(|| parse_date(&start).map(|d| d * DAY_MS))
                .unwrap_or(0);
            let me = e.attendees.iter().find(|a| a.is_self);
            let location: String = e
                .location
                .as_deref()
                .unwrap_or("")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(MAX_LOCATION)
                .collect();
            let value = json!({
                "id": e.id,
                "calendar": calendar.name(),
                "title": title(e.summary.as_deref(), "(No title)"),
                "start": start,
                "end": end,
                "allDay": all_day,
                "status": e.status,
                "location": (!location.is_empty()).then_some(location),
                "meetingLink": join_link(&e),
                "myResponse": me.and_then(|a| a.response_status.clone()),
                "attendees": e.attendees.len(),
                "link": e.html_link.filter(|l| is_https_on(l, &["google.com"])),
            });
            Some((sort, value))
        })
        .collect()
}

/// Every calendar's events in one list, soonest first, the same event once, at most `max`.
pub fn merge_events(mut events: Vec<(i64, Value)>, max: usize) -> (Vec<Value>, bool) {
    events.sort_by_key(|e| e.0);
    let mut seen = std::collections::HashSet::new();
    events.retain(|(_, v)| seen.insert((v["id"].to_string(), v["start"].to_string())));
    let more = events.len() > max;
    (events.into_iter().take(max).map(|(_, v)| v).collect(), more)
}

/// One list's open tasks as Claude reads them, filtered by `due` (`today` is the local day number).
pub fn agent_tasks(
    raw: Vec<RawTask>,
    list: &TaskListEntry,
    due: TaskDue,
    today: i64,
) -> Vec<Value> {
    raw.into_iter()
        .filter(|t| {
            !t.deleted && !t.hidden && t.status.as_deref() != Some("completed") && !t.id.is_empty()
        })
        .filter_map(|t| {
            let day = t.due.as_deref().and_then(|d| parse_date(d.get(..10)?));
            let keep = match due {
                TaskDue::All => true,
                TaskDue::Today => day.is_some_and(|d| d <= today),
                TaskDue::By(by) => day.is_some_and(|d| d <= by),
            };
            keep.then(|| {
                json!({
                    "id": t.id,
                    "list": list.title,
                    "title": title(t.title.as_deref(), "(No title)"),
                    "due": day.map(logic::format_date),
                    "overdue": day.is_some_and(|d| d < today),
                    "notes": t.notes.map(|n| n.chars().take(MAX_NOTES).collect::<String>()),
                })
            })
        })
        .take(MAX_TASKS)
        .collect()
}

/// The last day `tasks_list` needs from Google (`dueMax`), or `None` for every task.
pub fn due_max(due: TaskDue, today: i64) -> Option<i64> {
    match due {
        TaskDue::Today => Some(today),
        TaskDue::By(d) => Some(d),
        TaskDue::All => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use logic::{EventList, TaskItems};

    /// 2026-11-02T09:00:00Z, as in `logic`'s tests.
    const NOW: i64 = 1_793_610_000_000;

    fn calendar() -> CalendarEntry {
        CalendarEntry {
            id: "primary-id".into(),
            summary: "Work".into(),
            primary: true,
            selected: true,
            ..CalendarEntry::default()
        }
    }

    #[test]
    fn arguments() {
        assert_eq!(
            parse(CALENDAR_LIST_EVENTS, &json!({})),
            Ok(ToolCall::Events {
                from: None,
                to: None,
                query: None,
                max: DEFAULT_EVENTS
            })
        );
        assert!(parse(CALENDAR_LIST_EVENTS, &json!({ "from": "tomorrow" })).is_err());
        assert!(matches!(
            parse(
                CALENDAR_LIST_EVENTS,
                &json!({ "from": "2026-11-03", "maxResults": 999 })
            ),
            Ok(ToolCall::Events {
                max: MAX_EVENTS,
                ..
            })
        ));
        assert_eq!(
            parse(TASKS_LIST, &json!({ "due": "all" })),
            Ok(ToolCall::Tasks { due: TaskDue::All })
        );
        assert!(parse(TASKS_LIST, &json!({ "due": "soon" })).is_err());
        assert!(parse(GMAIL_SEARCH, &json!({ "query": " " })).is_err());
        assert!(parse(GMAIL_READ, &json!({ "id": "../x" })).is_err());
        let Ok(ToolCall::MailDraft(d)) = parse(
            GMAIL_DRAFT,
            &json!({ "to": "a@b.com, Bob <bob@c.org>", "subject": "Hi", "body": "Text" }),
        ) else {
            panic!()
        };
        assert_eq!(d.to, ["a@b.com", "Bob <bob@c.org>"]);
        assert!(
            parse(GMAIL_DRAFT, &json!({ "to": ["a@b.com"] })).is_err(),
            "no body"
        );
        assert!(parse(GMAIL_DRAFT, &json!({ "to": "x", "body": "y" })).is_err());
        assert!(ToolCall::MailRead { id: "a".into() }.is_mail());
        assert!(ToolCall::MailDraft(d)
            .scopes()
            .contains(&auth::SCOPE_GMAIL_COMPOSE));
    }

    #[test]
    fn ranges() {
        let utc = |_| 0;
        assert_eq!(
            range(None, None, NOW, utc),
            Ok((NOW, NOW + DEFAULT_SPAN_MS))
        );
        // A date is local midnight: at UTC+1 that's 23:00 the evening before.
        let (start, end) = range(Some("2026-11-03"), Some("2026-11-04"), NOW, |_| 60).unwrap();
        assert_eq!(logic::format_rfc3339(start), "2026-11-02T23:00:00Z");
        assert_eq!(end - start, DAY_MS);
        assert!(range(Some("2026-11-04"), Some("2026-11-03"), NOW, utc).is_err());
        assert!(range(Some("2026-11-01"), Some("2027-02-01"), NOW, utc).is_err());
    }

    #[test]
    fn events_for_claude() {
        let list: EventList = serde_json::from_str(include_str!("fixtures/events.json")).unwrap();
        let (events, more) = merge_events(agent_events(list.items, &calendar()), 50);
        assert!(!more);
        let ids: Vec<_> = events.iter().map(|e| e["id"].as_str().unwrap()).collect();
        assert!(ids.contains(&"allday1"), "all-day events are listed");
        assert!(ids.contains(&"declined1"), "with myResponse saying so");
        assert!(!ids.contains(&"cancelled1"));
        let standup = events
            .iter()
            .find(|e| e["id"] == "standup1_20261102T084500Z")
            .unwrap();
        assert_eq!(
            standup["meetingLink"],
            "https://meet.google.com/abc-defg-hij"
        );
        assert_eq!(standup["myResponse"], "accepted");
        assert_eq!(standup["attendees"], 2);
        let lunch = events.iter().find(|e| e["id"] == "lunch1").unwrap();
        assert_eq!(lunch["meetingLink"], Value::Null, "no userinfo links");
        let allday = events.iter().find(|e| e["id"] == "allday1").unwrap();
        assert_eq!(allday["allDay"], true);
        assert!(events.iter().all(|e| e.get("description").is_none()));
        let (two, more) = merge_events(
            agent_events(
                serde_json::from_str::<EventList>(include_str!("fixtures/events.json"))
                    .unwrap()
                    .items,
                &calendar(),
            ),
            2,
        );
        assert!(more && two.len() == 2);
    }

    #[test]
    fn tasks_for_claude() {
        let raw = || {
            serde_json::from_str::<TaskItems>(include_str!("fixtures/tasks.json"))
                .unwrap()
                .items
        };
        let list = TaskListEntry {
            id: "l1".into(),
            title: "My Tasks".into(),
        };
        let today = parse_date("2026-11-02").unwrap();
        let titles = |due| {
            agent_tasks(raw(), &list, due, today)
                .iter()
                .map(|t| t["title"].as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        };
        let due_today = titles(TaskDue::Today);
        assert!(due_today.contains(&"Call the bank".to_string()));
        assert!(due_today.contains(&"Send invoice".to_string()));
        assert!(!due_today.contains(&"Book flights".to_string()));
        assert!(!due_today.contains(&"Done already".to_string()));
        assert!(
            titles(TaskDue::By(parse_date("2026-11-05").unwrap())).contains(&"Book flights".into())
        );
        assert!(titles(TaskDue::All).contains(&"Someday".into()));
        assert_eq!(due_max(TaskDue::All, today), None);
    }
}
