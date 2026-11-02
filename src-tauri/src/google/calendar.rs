//! Calendar API v3: the calendar list, then each chosen calendar's events in a window around now.
//!
//! A windowed query (`timeMin`/`timeMax`, `singleEvents`, `orderBy=startTime`) every few minutes, not sync tokens:
//! Google refuses `syncToken` together with `timeMin`, `timeMax` and `orderBy` (D42).

use super::http::{Client, FetchError};
use super::logic::{self, encode, CalendarEntry, CalendarList, Event, EventList};

const HOST: &str = "www.googleapis.com";
const MAX_PAGES: usize = 4;
const MAX_CALENDARS: usize = 20;
const LIST_FIELDS: &str =
    "nextPageToken,items(id,summary,summaryOverride,backgroundColor,primary,selected,hidden)";
const EVENT_FIELDS: &str = "nextPageToken,items(id,status,summary,start,end,hangoutLink,\
                            conferenceData/entryPoints(entryPointType,uri),attendees(self,responseStatus),\
                            htmlLink,location,eventType)";

pub fn calendars(http: &Client, token: &str) -> Result<Vec<CalendarEntry>, FetchError> {
    let mut out = Vec::new();
    let mut page: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let mut path = format!(
            "/calendar/v3/users/me/calendarList?minAccessRole=reader&maxResults=250&fields={}",
            encode(LIST_FIELDS)
        );
        if let Some(p) = &page {
            path.push_str(&format!("&pageToken={}", encode(p)));
        }
        let list: CalendarList = http.get_json(HOST, &path, token)?;
        out.extend(list.items);
        page = list.next_page_token;
        if page.is_none() {
            break;
        }
    }
    Ok(out)
}

/// The calendars Cay reads: the ids in `google.calendars` if set, else the ones shown in Google Calendar (and the
/// primary one), never hidden ones. At most 20.
pub fn chosen(all: &[CalendarEntry], ids: Option<&[String]>) -> Vec<CalendarEntry> {
    all.iter()
        .filter(|c| match ids {
            Some(ids) => ids.iter().any(|id| id == &c.id),
            None => !c.hidden && (c.selected || c.primary),
        })
        .take(MAX_CALENDARS)
        .cloned()
        .collect()
}

pub fn events(
    http: &Client,
    token: &str,
    calendar: &CalendarEntry,
    now: i64,
) -> Result<Vec<Event>, FetchError> {
    let mut out = Vec::new();
    let mut page: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let mut path = format!(
            "/calendar/v3/calendars/{}/events?singleEvents=true&orderBy=startTime&maxResults=50\
             &timeMin={}&timeMax={}&fields={}",
            encode(&calendar.id),
            encode(&logic::format_rfc3339(now - logic::BEHIND_MS)),
            encode(&logic::format_rfc3339(now + logic::AHEAD_MS)),
            encode(EVENT_FIELDS)
        );
        for t in logic::EVENT_TYPES {
            path.push_str(&format!("&eventTypes={t}"));
        }
        if let Some(p) = &page {
            path.push_str(&format!("&pageToken={}", encode(p)));
        }
        let list: EventList = http.get_json(HOST, &path, token)?;
        out.extend(logic::events(list.items, calendar, now));
        page = list.next_page_token;
        if page.is_none() {
            break;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chosen_calendars_follow_google_or_the_settings() {
        let list: CalendarList =
            serde_json::from_str(include_str!("fixtures/calendars.json")).unwrap();
        let names =
            |c: Vec<CalendarEntry>| c.iter().map(|c| c.name().to_string()).collect::<Vec<_>>();
        assert_eq!(names(chosen(&list.items, None)), ["Work", "Family (mine)"]);
        let ids = vec!["holidays@group.v.calendar.google.com".to_string()];
        assert_eq!(names(chosen(&list.items, Some(&ids))), ["Holidays"]);
    }
}
