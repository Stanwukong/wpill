//! Pure parts of the Google watcher (Phase 6a): what Google's JSON means for the island, and when to look again.
//! Unit-tested with recorded fixtures (`fixtures/`, synthetic data), never a live account.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::ipc::{CalendarEvent, EventPhase, GoogleTask};

pub const MINUTE_MS: i64 = 60_000;
const DAY_MS: i64 = 24 * 60 * MINUTE_MS;
/// Events starting within this window are listed (and fetched).
pub const AHEAD_MS: i64 = DAY_MS;
/// Events that started this long ago and haven't ended are still fetched (a long meeting in progress).
pub const BEHIND_MS: i64 = 12 * 60 * MINUTE_MS;
const MAX_EVENTS: usize = 30;
const MAX_TASKS: usize = 30;
const MAX_TITLE: usize = 200;
/// Only these event types are shown: focus time, out of office and working location aren't things you join.
pub const EVENT_TYPES: [&str; 2] = ["default", "fromGmail"];
/// Hosts whose https link in an event's location counts as a Join link (Google's own conference data first).
const MEETING_HOSTS: [&str; 6] = [
    "meet.google.com",
    "zoom.us",
    "teams.microsoft.com",
    "teams.live.com",
    "webex.com",
    "whereby.com",
];

// ---- Google's JSON (only the fields Cay reads) ----

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CalendarList {
    pub items: Vec<CalendarEntry>,
    pub next_page_token: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CalendarEntry {
    pub id: String,
    pub summary: String,
    pub summary_override: Option<String>,
    pub background_color: Option<String>,
    pub primary: bool,
    pub selected: bool,
    pub hidden: bool,
}

impl CalendarEntry {
    pub fn name(&self) -> &str {
        self.summary_override.as_deref().unwrap_or(&self.summary)
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EventList {
    pub items: Vec<RawEvent>,
    pub next_page_token: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RawEvent {
    pub id: String,
    pub status: Option<String>,
    pub summary: Option<String>,
    pub start: RawTime,
    pub end: RawTime,
    pub hangout_link: Option<String>,
    pub conference_data: Option<ConferenceData>,
    pub attendees: Vec<Attendee>,
    pub html_link: Option<String>,
    pub location: Option<String>,
    pub event_type: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RawTime {
    /// All-day events have a date and no time.
    pub date: Option<String>,
    pub date_time: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ConferenceData {
    pub entry_points: Vec<EntryPoint>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EntryPoint {
    pub entry_point_type: String,
    pub uri: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Attendee {
    #[serde(rename = "self")]
    pub is_self: bool,
    pub response_status: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TaskLists {
    pub items: Vec<TaskListEntry>,
    pub next_page_token: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TaskListEntry {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TaskItems {
    pub items: Vec<RawTask>,
    pub next_page_token: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RawTask {
    pub id: String,
    pub title: Option<String>,
    /// RFC 3339, but only the date means anything: "the time portion of the timestamp is discarded".
    pub due: Option<String>,
    pub status: Option<String>,
    pub deleted: bool,
    pub hidden: bool,
    pub web_view_link: Option<String>,
}

// ---- What Cay keeps ----

/// An event with the links that stay in Rust (React gets `has_join`, and asks by id).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub item: CalendarEvent,
    pub open: Option<String>,
    pub join: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub item: GoogleTask,
    pub open: Option<String>,
}

/// The events and tasks last fetched, before the time-dependent parts (`phase`, `overdue`) are applied.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Upcoming {
    pub events: Vec<Event>,
    pub tasks: Vec<Task>,
}

/// One calendar's events → what the island lists. `now` filters out what ended or is more than a day away.
pub fn events(raw: Vec<RawEvent>, calendar: &CalendarEntry, now: i64) -> Vec<Event> {
    raw.into_iter()
        .filter_map(|e| event(e, calendar, now))
        .collect()
}

fn event(e: RawEvent, calendar: &CalendarEntry, now: i64) -> Option<Event> {
    if e.status.as_deref() == Some("cancelled") || e.id.is_empty() {
        return None;
    }
    if e.event_type
        .as_deref()
        .is_some_and(|t| !EVENT_TYPES.contains(&t))
    {
        return None;
    }
    // All-day events (a date, no time) aren't "starting soon".
    let start = parse_rfc3339(e.start.date_time.as_deref()?)?;
    let end = parse_rfc3339(e.end.date_time.as_deref()?)?.max(start);
    if end <= now || start >= now + AHEAD_MS {
        return None;
    }
    if e.attendees
        .iter()
        .any(|a| a.is_self && a.response_status.as_deref() == Some("declined"))
    {
        return None;
    }
    let join = join_link(&e);
    let open = e.html_link.filter(|l| is_https_on(l, &["google.com"]));
    Some(Event {
        item: CalendarEvent {
            id: e.id,
            title: title(e.summary.as_deref(), "(No title)"),
            start_ms: start,
            end_ms: end,
            calendar: calendar.name().to_string(),
            color: calendar
                .background_color
                .clone()
                .filter(|c| is_hex_color(c)),
            has_join: join.is_some(),
            phase: EventPhase::Later,
        },
        open,
        join,
    })
}

fn join_link(e: &RawEvent) -> Option<String> {
    if let Some(link) = e.hangout_link.as_deref().filter(|l| is_https(l)) {
        return Some(link.to_string());
    }
    if let Some(point) = e.conference_data.as_ref().and_then(|c| {
        c.entry_points
            .iter()
            .find(|p| p.entry_point_type == "video" && is_https(&p.uri))
    }) {
        return Some(point.uri.clone());
    }
    let location = e.location.as_deref()?.trim();
    is_https_on(location, &MEETING_HOSTS).then(|| location.to_string())
}

/// Every calendar's events in one list: duplicates (the same meeting on two calendars) once, soonest first.
pub fn merge(mut events: Vec<Event>) -> Vec<Event> {
    events.sort_by(|a, b| {
        (a.item.start_ms, &a.item.title, &a.item.id).cmp(&(
            b.item.start_ms,
            &b.item.title,
            &b.item.id,
        ))
    });
    let mut seen = std::collections::HashSet::new();
    events.retain(|e| seen.insert((e.item.id.clone(), e.item.start_ms)));
    events.truncate(MAX_EVENTS);
    events
}

/// One list's open tasks due today or earlier (`today` is the local date as a day number).
pub fn tasks(raw: Vec<RawTask>, list: &TaskListEntry, today: i64) -> Vec<Task> {
    raw.into_iter()
        .filter_map(|t| {
            if t.deleted || t.hidden || t.status.as_deref() == Some("completed") || t.id.is_empty()
            {
                return None;
            }
            let due = t.due.as_deref()?.get(..10)?;
            let day = parse_date(due)?;
            (day <= today).then(|| Task {
                item: GoogleTask {
                    id: t.id,
                    title: title(t.title.as_deref(), "(No title)"),
                    due: due.to_string(),
                    overdue: day < today,
                    list: list.title.clone(),
                },
                open: t.web_view_link.filter(|l| is_https_on(l, &["google.com"])),
            })
        })
        .collect()
}

/// Overdue first (oldest first), then today's, by title.
pub fn sort_tasks(mut tasks: Vec<Task>) -> Vec<Task> {
    tasks.sort_by(|a, b| (&a.item.due, &a.item.title).cmp(&(&b.item.due, &b.item.title)));
    tasks.truncate(MAX_TASKS);
    tasks
}

/// Applies the time-dependent parts at `now`: events that ended are dropped, each gets its phase, tasks are
/// overdue or due today against `today`; tasks from before a midnight that passed stay until the next fetch.
pub fn at(upcoming: &Upcoming, now: i64, soon_ms: i64, today: i64) -> Upcoming {
    let events = upcoming
        .events
        .iter()
        .filter(|e| e.item.end_ms > now)
        .map(|e| {
            let mut e = e.clone();
            e.item.phase = phase(&e.item, now, soon_ms);
            e
        })
        .collect();
    let tasks = upcoming
        .tasks
        .iter()
        .filter_map(|t| {
            let day = parse_date(&t.item.due)?;
            let mut t = t.clone();
            t.item.overdue = day < today;
            (day <= today).then_some(t)
        })
        .collect();
    Upcoming { events, tasks }
}

pub fn phase(e: &CalendarEvent, now: i64, soon_ms: i64) -> EventPhase {
    if e.start_ms <= now {
        EventPhase::Now
    } else if e.start_ms - soon_ms <= now {
        EventPhase::Soon
    } else {
        EventPhase::Later
    }
}

/// Something wants the pill's calendar icon: an event starting soon or in progress, or a task due.
pub fn pending(upcoming: &Upcoming) -> bool {
    !upcoming.tasks.is_empty()
        || upcoming
            .events
            .iter()
            .any(|e| e.item.phase != EventPhase::Later)
}

/// The next moment something changes: an event reaching one of the `ahead` offsets before its start (soon, the
/// "starting soon" lead), starting or ending, or local midnight.
pub fn next_boundary(
    upcoming: &Upcoming,
    now: i64,
    ahead: &[i64],
    offset_min: impl Fn(i64) -> i64,
) -> i64 {
    let midnight = next_local_midnight(now, offset_min);
    upcoming
        .events
        .iter()
        .flat_map(|e| {
            ahead
                .iter()
                .map(|a| e.item.start_ms - a)
                .chain([e.item.start_ms, e.item.end_ms])
        })
        .filter(|&t| t > now)
        .fold(midnight, i64::min)
}

/// Events to announce now: inside the lead time before their start, not started, not announced yet (keyed by id and
/// start, so a moved event is announced again).
pub fn to_announce<'a>(
    upcoming: &'a Upcoming,
    now: i64,
    lead_ms: i64,
    announced: &std::collections::HashSet<(String, i64)>,
) -> Vec<&'a Event> {
    upcoming
        .events
        .iter()
        .filter(|e| e.item.start_ms - lead_ms <= now && now < e.item.start_ms)
        .filter(|e| !announced.contains(&(e.item.id.clone(), e.item.start_ms)))
        .collect()
}

/// Whole minutes from `now` to `start`, rounded up ("in 5 min" at 4:30 left), 0 once it's started.
pub fn minutes_until(start: i64, now: i64) -> u32 {
    ((start - now).max(0) + MINUTE_MS - 1).div_euclid(MINUTE_MS) as u32
}

/// After `failures` refreshes in a row went wrong (offline, Google down): 1, 2, 5, 10, 20, then 30 minutes.
pub fn backoff(failures: u32) -> Duration {
    const MINUTES: [u64; 6] = [1, 2, 5, 10, 20, 30];
    let i = (failures.max(1) - 1) as usize;
    Duration::from_secs(MINUTES[i.min(MINUTES.len() - 1)] * 60)
}

fn title(raw: Option<&str>, fallback: &str) -> String {
    let t: String = raw
        .unwrap_or("")
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let t = t.trim();
    if t.is_empty() {
        return fallback.to_string();
    }
    t.chars().take(MAX_TITLE).collect()
}

fn is_hex_color(c: &str) -> bool {
    c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|ch| ch.is_ascii_hexdigit())
}

fn is_https(link: &str) -> bool {
    host(link).is_some()
}

/// An https link whose host is one of `hosts` or a subdomain of one.
fn is_https_on(link: &str, hosts: &[&str]) -> bool {
    host(link).is_some_and(|h| {
        hosts
            .iter()
            .any(|allowed| h == *allowed || h.ends_with(&format!(".{allowed}")))
    })
}

fn host(link: &str) -> Option<String> {
    let rest = link.strip_prefix("https://")?;
    if link.len() > 2048 || link.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let host = rest.split(['/', '?', '#']).next()?;
    // No userinfo (`https://meet.google.com@evil.example`), no port games.
    if host.is_empty() || host.contains('@') {
        return None;
    }
    let host = host.split(':').next()?.to_ascii_lowercase();
    Some(host)
}

// ---- Time ----

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's `days_from_civil`).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `(year, month, day)` for a day number (inverse of `days_from_civil`).
pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (
        if m <= 2 {
            yoe + era * 400 + 1
        } else {
            yoe + era * 400
        },
        m,
        d,
    )
}

/// `YYYY-MM-DD` → day number.
pub fn parse_date(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let (y, m, d) = (num(&s[0..4])?, num(&s[5..7])?, num(&s[8..10])?);
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then(|| days_from_civil(y, m, d))
}

/// RFC 3339 (`2026-11-02T09:30:00+01:00`, `…Z`, fractions allowed) → Unix ms.
pub fn parse_rfc3339(s: &str) -> Option<i64> {
    if s.len() < 20 || !s.is_char_boundary(19) {
        return None;
    }
    let day = parse_date(&s[0..10])?;
    let b = s.as_bytes();
    if !matches!(b[10], b'T' | b't') || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let (h, mi, sec) = (num(&s[11..13])?, num(&s[14..16])?, num(&s[17..19])?);
    if h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    let mut rest = &s[19..];
    let mut ms = 0;
    if let Some(frac) = rest.strip_prefix('.') {
        let digits = frac.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        let padded: String = frac[..digits]
            .chars()
            .chain("000".chars())
            .take(3)
            .collect();
        ms = num(&padded)?;
        rest = &frac[digits..];
    }
    let offset_min = match rest {
        "Z" | "z" => 0,
        _ => {
            let sign = match rest.as_bytes().first()? {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            if rest.len() != 6 || rest.as_bytes()[3] != b':' {
                return None;
            }
            sign * (num(&rest[1..3])? * 60 + num(&rest[4..6])?)
        }
    };
    Some(((day * 24 + h) * 60 + mi - offset_min) * MINUTE_MS + sec * 1000 + ms)
}

fn num(s: &str) -> Option<i64> {
    (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        .then(|| s.parse().ok())
        .flatten()
}

/// Unix ms → `2026-11-02T08:30:00Z`, for `timeMin` / `timeMax`.
pub fn format_rfc3339(ms: i64) -> String {
    let day = ms.div_euclid(DAY_MS);
    let (y, m, d) = civil_from_days(day);
    let secs = ms.rem_euclid(DAY_MS) / 1000;
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        secs / 60 % 60,
        secs % 60
    )
}

/// The local date at `now` as a day number.
pub fn local_day(now: i64, offset_min: impl Fn(i64) -> i64) -> i64 {
    (now + offset_min(now) * MINUTE_MS).div_euclid(DAY_MS)
}

/// A day number as `YYYY-MM-DD`.
pub fn format_date(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{y:04}-{m:02}-{d:02}")
}

/// The next local midnight after `now`, in UTC ms. Uses the offset at the guess, so a DST change that day lands
/// on the real midnight (zones don't change at midnight often; when one does, an hour either side is fine).
fn next_local_midnight(now: i64, offset_min: impl Fn(i64) -> i64) -> i64 {
    let tomorrow = local_day(now, &offset_min) + 1;
    let guess = tomorrow * DAY_MS - offset_min(now) * MINUTE_MS;
    let exact = tomorrow * DAY_MS - offset_min(guess) * MINUTE_MS;
    if exact > now {
        exact
    } else {
        guess.max(now + MINUTE_MS)
    }
}

/// Percent-encodes everything but RFC 3986's unreserved characters (query values, path segments, form bodies).
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Decodes `%XX` and `+` (a query string from the browser). Invalid escapes are kept as they are.
pub fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => match (hex(b[i + 1]), hex(b[i + 2])) {
                (Some(hi), Some(lo)) => {
                    out.push(hi << 4 | lo);
                    i += 3;
                    continue;
                }
                _ => out.push(b'%'),
            },
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(b: u8) -> Option<u8> {
    (b as char).to_digit(16).map(|d| d as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-11-02T09:00:00Z
    const NOW: i64 = 1_793_610_000_000;

    fn calendar() -> CalendarEntry {
        CalendarEntry {
            id: "primary-id".into(),
            summary: "Work".into(),
            background_color: Some("#16a765".into()),
            primary: true,
            selected: true,
            ..Default::default()
        }
    }

    fn fixture_events() -> Vec<Event> {
        let list: EventList = serde_json::from_str(include_str!("fixtures/events.json")).unwrap();
        events(list.items, &calendar(), NOW)
    }

    #[test]
    fn dates_round_trip() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(days_from_civil(2026, 11, 2)), (2026, 11, 2));
        assert_eq!(civil_from_days(days_from_civil(2024, 2, 29)), (2024, 2, 29));
        assert_eq!(format_rfc3339(NOW), "2026-11-02T09:00:00Z");
        assert_eq!(format_date(local_day(NOW, |_| 0)), "2026-11-02");
    }

    #[test]
    fn rfc3339_with_offsets_and_fractions() {
        assert_eq!(parse_rfc3339("2026-11-02T09:00:00Z"), Some(NOW));
        assert_eq!(parse_rfc3339("2026-11-02T10:00:00+01:00"), Some(NOW));
        assert_eq!(parse_rfc3339("2026-11-02T04:00:00-05:00"), Some(NOW));
        assert_eq!(parse_rfc3339("2026-11-02T09:00:00.250Z"), Some(NOW + 250));
        assert_eq!(parse_rfc3339("2026-11-02T09:00:00.5Z"), Some(NOW + 500));
        assert_eq!(parse_rfc3339("2026-11-02"), None);
        assert_eq!(parse_rfc3339("2026-11-02T25:00:00Z"), None);
        assert_eq!(parse_rfc3339("2026-11-02T09:00:00+0100"), None);
        assert_eq!(parse_rfc3339("2026-11-02T09:00:00ä"), None);
    }

    #[test]
    fn events_skip_all_day_declined_cancelled_ended_far_and_other_types() {
        let titles: Vec<String> = fixture_events().into_iter().map(|e| e.item.title).collect();
        assert_eq!(
            titles,
            [
                "Standup",
                "Design review",
                "Lunch with Ada",
                "(No title)",
                "Zoom sync"
            ]
        );
    }

    #[test]
    fn join_links_from_meet_conference_data_or_a_meeting_location() {
        let events = fixture_events();
        let find = |t: &str| events.iter().find(|e| e.item.title == t).unwrap();
        assert_eq!(
            find("Standup").join.as_deref(),
            Some("https://meet.google.com/abc-defg-hij")
        );
        assert_eq!(
            find("Design review").join.as_deref(),
            Some("https://example.zoom.us/j/123")
        );
        assert_eq!(
            find("Zoom sync").join.as_deref(),
            Some("https://us02web.zoom.us/j/999")
        );
        // An http link, a link with userinfo and a non-meeting host aren't Join links.
        assert_eq!(find("Lunch with Ada").join, None);
        assert!(!find("Lunch with Ada").item.has_join);
        assert!(find("Standup")
            .open
            .as_deref()
            .unwrap()
            .starts_with("https://www.google.com/calendar/"));
        assert_eq!(find("Standup").item.color.as_deref(), Some("#16a765"));
    }

    #[test]
    fn merge_dedupes_and_sorts() {
        let mut twice = fixture_events();
        twice.extend(fixture_events());
        let merged = merge(twice);
        assert_eq!(merged.len(), 5);
        assert!(merged
            .windows(2)
            .all(|w| w[0].item.start_ms <= w[1].item.start_ms));
    }

    #[test]
    fn phases_pending_and_boundaries() {
        let upcoming = Upcoming {
            events: merge(fixture_events()),
            tasks: vec![],
        };
        let soon = 15 * MINUTE_MS;
        // Standup runs 08:45–09:15: in progress at 09:00.
        let now = at(&upcoming, NOW, soon, local_day(NOW, |_| 0));
        assert_eq!(now.events[0].item.phase, EventPhase::Now);
        assert_eq!(now.events[1].item.phase, EventPhase::Later);
        assert!(pending(&now));
        // Next change: the design review (10:00) becomes soon at 09:45... but Standup ends at 09:15 first.
        assert_eq!(
            next_boundary(&now, NOW, &[soon], |_| 0),
            NOW + 15 * MINUTE_MS
        );
        // After the standup ends, nothing is pending until 09:45.
        let later = at(&upcoming, NOW + 20 * MINUTE_MS, soon, local_day(NOW, |_| 0));
        assert!(!pending(&later));
        assert_eq!(later.events[0].item.title, "Design review");
        let at_soon = at(&upcoming, NOW + 46 * MINUTE_MS, soon, local_day(NOW, |_| 0));
        assert_eq!(at_soon.events[0].item.phase, EventPhase::Soon);
    }

    #[test]
    fn the_next_boundary_falls_back_to_local_midnight() {
        let empty = Upcoming::default();
        // UTC+1: local midnight is 23:00 UTC.
        assert_eq!(
            format_rfc3339(next_boundary(&empty, NOW, &[0], |_| 60)),
            "2026-11-02T23:00:00Z"
        );
        assert_eq!(
            format_rfc3339(next_boundary(&empty, NOW, &[0], |_| -300)),
            "2026-11-03T05:00:00Z"
        );
    }

    #[test]
    fn tasks_due_today_or_overdue_in_the_local_zone() {
        let raw: TaskItems = serde_json::from_str(include_str!("fixtures/tasks.json")).unwrap();
        let list = TaskListEntry {
            id: "l1".into(),
            title: "My Tasks".into(),
        };
        let today = parse_date("2026-11-02").unwrap();
        let tasks = sort_tasks(tasks(raw.items, &list, today));
        let shown: Vec<(&str, bool)> = tasks
            .iter()
            .map(|t| (t.item.title.as_str(), t.item.overdue))
            .collect();
        assert_eq!(shown, [("Send invoice", true), ("Call the bank", false)]);
        assert_eq!(tasks[0].item.list, "My Tasks");
        assert_eq!(tasks[0].open, None);
        // The next day, today's task is overdue too.
        let tomorrow = at(
            &Upcoming {
                events: vec![],
                tasks: tasks.clone(),
            },
            NOW,
            0,
            today + 1,
        );
        assert!(tomorrow.tasks.iter().all(|t| t.item.overdue));
        assert!(pending(&tomorrow));
    }

    #[test]
    fn announcements_come_once_inside_the_lead_time() {
        let upcoming = Upcoming {
            events: merge(fixture_events()),
            tasks: vec![],
        };
        let lead = 5 * MINUTE_MS;
        let mut announced = std::collections::HashSet::new();
        // 09:00: the standup has started, the 10:00 review is an hour away.
        assert!(to_announce(&upcoming, NOW, lead, &announced).is_empty());
        // 09:56: the review is 4 min away.
        let at = NOW + 56 * MINUTE_MS;
        let due = to_announce(&upcoming, at, lead, &announced);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].item.title, "Design review");
        assert_eq!(minutes_until(due[0].item.start_ms, at), 4);
        assert_eq!(minutes_until(due[0].item.start_ms, at + 30_000), 4);
        assert_eq!(
            minutes_until(due[0].item.start_ms, due[0].item.start_ms + 1),
            0
        );
        announced.insert((due[0].item.id.clone(), due[0].item.start_ms));
        assert!(to_announce(&upcoming, at + MINUTE_MS, lead, &announced).is_empty());
        // The lead boundary is a wake-up time: 09:55 for the review once the standup has ended.
        assert_eq!(
            next_boundary(
                &upcoming,
                NOW + 20 * MINUTE_MS,
                &[15 * MINUTE_MS, lead],
                |_| 0
            ),
            NOW + 45 * MINUTE_MS
        );
        assert_eq!(
            next_boundary(
                &upcoming,
                NOW + 46 * MINUTE_MS,
                &[15 * MINUTE_MS, lead],
                |_| 0
            ),
            NOW + 55 * MINUTE_MS
        );
    }

    #[test]
    fn backoff_grows_and_caps() {
        let mins: Vec<u64> = (1..=8).map(|f| backoff(f).as_secs() / 60).collect();
        assert_eq!(mins, [1, 2, 5, 10, 20, 30, 30, 30]);
    }

    #[test]
    fn encoding_round_trips() {
        assert_eq!(encode("a b@c#d/é"), "a%20b%40c%23d%2F%C3%A9");
        assert_eq!(decode("a%20b+c%2Fd%zz%4"), "a b c/d%zz%4");
        assert_eq!(decode(&encode("4/0AeaY-x_y~z")), "4/0AeaY-x_y~z");
    }

    #[test]
    fn hosts_are_checked_strictly() {
        assert!(is_https_on("https://meet.google.com/x", &MEETING_HOSTS));
        assert!(is_https_on("https://a.zoom.us/j/1", &MEETING_HOSTS));
        assert!(!is_https_on("https://evilzoom.us/j/1", &MEETING_HOSTS));
        assert!(!is_https_on(
            "https://meet.google.com@evil.example/",
            &MEETING_HOSTS
        ));
        assert!(!is_https_on("http://meet.google.com/x", &MEETING_HOSTS));
        assert!(!is_https_on("https://meet.google.com/x y", &MEETING_HOSTS));
    }
}
