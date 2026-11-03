//! Tasks API v1: every task list's open tasks due today or earlier. Tasks have dates, not times.

use super::http::{Client, FetchError};
use super::logic::{self, encode, RawTask, Task, TaskItems, TaskListEntry, TaskLists};

const HOST: &str = "tasks.googleapis.com";
const MAX_LISTS: usize = 20;
const MAX_PAGES: usize = 3;
const TASK_FIELDS: &str = "nextPageToken,items(id,title,due,status,deleted,hidden,webViewLink)";
const AGENT_FIELDS: &str = "nextPageToken,items(id,title,due,status,deleted,hidden,notes)";

pub fn lists(http: &Client, token: &str) -> Result<Vec<TaskListEntry>, FetchError> {
    let path = format!(
        "/tasks/v1/users/@me/lists?maxResults={MAX_LISTS}&fields={}",
        encode("items(id,title)")
    );
    let lists: TaskLists = http.get_json(HOST, &path, token)?;
    Ok(lists.items)
}

/// `today` is the local date as a day number. `dueMax` (midnight starting tomorrow) keeps the reply small;
/// `logic::tasks` checks the date again.
pub fn due(
    http: &Client,
    token: &str,
    list: &TaskListEntry,
    today: i64,
) -> Result<Vec<Task>, FetchError> {
    let mut out = Vec::new();
    let mut page: Option<String> = None;
    let due_max = format!("{}T00:00:00.000Z", logic::format_date(today + 1));
    for _ in 0..MAX_PAGES {
        let mut path = format!(
            "/tasks/v1/lists/{}/tasks?showCompleted=false&maxResults=100&dueMax={}&fields={}",
            encode(&list.id),
            encode(&due_max),
            encode(TASK_FIELDS)
        );
        if let Some(p) = &page {
            path.push_str(&format!("&pageToken={}", encode(p)));
        }
        let items: TaskItems = http.get_json(HOST, &path, token)?;
        out.extend(logic::tasks(items.items, list, today));
        page = items.next_page_token;
        if page.is_none() {
            break;
        }
    }
    Ok(out)
}

/// For an agent run (6c): one list's open tasks, due on or before `due_max` (a local day), or all of them.
pub fn open(
    http: &Client,
    token: &str,
    list: &TaskListEntry,
    due_max: Option<i64>,
) -> Result<Vec<RawTask>, FetchError> {
    let mut out = Vec::new();
    let mut page: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let mut path = format!(
            "/tasks/v1/lists/{}/tasks?showCompleted=false&maxResults=100&fields={}",
            encode(&list.id),
            encode(AGENT_FIELDS)
        );
        if let Some(day) = due_max {
            let max = format!("{}T00:00:00.000Z", logic::format_date(day + 1));
            path.push_str(&format!("&dueMax={}", encode(&max)));
        }
        if let Some(p) = &page {
            path.push_str(&format!("&pageToken={}", encode(p)));
        }
        let items: TaskItems = http.get_json(HOST, &path, token)?;
        out.extend(items.items);
        page = items.next_page_token;
        if page.is_none() {
            break;
        }
    }
    Ok(out)
}
