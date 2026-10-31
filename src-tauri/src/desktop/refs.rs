//! What Claude's last looks at a window said, kept per run so an action can name things precisely: the elements
//! of the latest `ui_tree` (by their `[n]` ref) and where the latest screenshot sat on screen. Both go stale: a
//! new tree of the same window replaces the old refs, and everything expires after `MAX_AGE`. Pure; unit-tested.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::plan::{ActError, Element, View};

pub const MAX_AGE: Duration = Duration::from_secs(10 * 60);
/// Windows remembered per run (oldest forgotten first).
const MAX_WINDOWS: usize = 16;

#[derive(Debug, Clone)]
struct Seen {
    pid: u32,
    at: Instant,
    elements: Vec<Element>,
    view: Option<(View, Instant)>,
}

#[derive(Debug, Default)]
pub struct Refs {
    /// (run, hwnd) → what was seen.
    seen: HashMap<(String, i64), Seen>,
}

impl Refs {
    /// A `ui_tree` of `hwnd` (process `pid`) listed `elements`, in ref order (`[0]` first).
    pub fn put_tree(
        &mut self,
        run: &str,
        hwnd: i64,
        pid: u32,
        elements: Vec<Element>,
        now: Instant,
    ) {
        let view = self
            .seen
            .get(&(run.to_string(), hwnd))
            .filter(|s| s.pid == pid)
            .and_then(|s| s.view);
        self.insert(
            run,
            hwnd,
            Seen {
                pid,
                at: now,
                elements,
                view,
            },
        );
    }

    /// A screenshot of `hwnd` was taken.
    pub fn put_view(&mut self, run: &str, hwnd: i64, pid: u32, view: View, now: Instant) {
        let key = (run.to_string(), hwnd);
        match self.seen.get_mut(&key).filter(|s| s.pid == pid) {
            Some(s) => s.view = Some((view, now)),
            None => self.insert(
                run,
                hwnd,
                Seen {
                    pid,
                    at: now,
                    elements: Vec::new(),
                    view: Some((view, now)),
                },
            ),
        }
    }

    fn insert(&mut self, run: &str, hwnd: i64, seen: Seen) {
        let mine: Vec<_> = self
            .seen
            .keys()
            .filter(|(r, _)| r == run)
            .cloned()
            .collect();
        if mine.len() >= MAX_WINDOWS && !self.seen.contains_key(&(run.to_string(), hwnd)) {
            if let Some(oldest) = mine.iter().min_by_key(|k| self.seen[*k].at) {
                self.seen.remove(oldest);
            }
        }
        self.seen.insert((run.to_string(), hwnd), seen);
    }

    /// Element `n` of the latest tree of `hwnd`, if it's still current (same process, not too old).
    pub fn element(
        &self,
        run: &str,
        hwnd: i64,
        n: usize,
        now: Instant,
    ) -> Result<(Element, u32), ActError> {
        let seen = self
            .seen
            .get(&(run.to_string(), hwnd))
            .ok_or(ActError::ElementGone)?;
        if now.duration_since(seen.at) > MAX_AGE {
            return Err(ActError::ElementGone);
        }
        let e = seen.elements.get(n).ok_or(ActError::ElementGone)?;
        Ok((e.clone(), seen.pid))
    }

    /// The latest screenshot of `hwnd`, if it's not too old.
    pub fn view(&self, run: &str, hwnd: i64, now: Instant) -> Result<(View, u32), ActError> {
        let seen = self
            .seen
            .get(&(run.to_string(), hwnd))
            .ok_or(ActError::StaleView)?;
        match seen.view {
            Some((view, at)) if now.duration_since(at) <= MAX_AGE => Ok((view, seen.pid)),
            _ => Err(ActError::StaleView),
        }
    }

    pub fn end_run(&mut self, run: &str) {
        self.seen.retain(|(r, _), _| r != run);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::logic::Rect;

    fn el(name: &str) -> Element {
        Element {
            runtime_id: vec![42, 1],
            control_type: 50000,
            name: name.into(),
            ..Element::default()
        }
    }

    #[test]
    fn refs_follow_the_latest_tree() {
        let now = Instant::now();
        let mut r = Refs::default();
        r.put_tree("a", 5, 9, vec![el("window"), el("Save")], now);
        assert_eq!(r.element("a", 5, 1, now).unwrap().0.name, "Save");
        assert_eq!(r.element("a", 5, 1, now).unwrap().1, 9);
        assert_eq!(r.element("a", 5, 2, now), Err(ActError::ElementGone));
        assert_eq!(
            r.element("b", 5, 1, now),
            Err(ActError::ElementGone),
            "other runs don't see it"
        );
        assert_eq!(r.element("a", 6, 1, now), Err(ActError::ElementGone));
        r.put_tree("a", 5, 9, vec![el("Open")], now);
        assert_eq!(r.element("a", 5, 0, now).unwrap().0.name, "Open");
        assert_eq!(
            r.element("a", 5, 1, now),
            Err(ActError::ElementGone),
            "old refs are gone"
        );
        assert_eq!(
            r.element("a", 5, 0, now + MAX_AGE + Duration::from_secs(1)),
            Err(ActError::ElementGone)
        );
        r.end_run("a");
        assert_eq!(r.element("a", 5, 0, now), Err(ActError::ElementGone));
    }

    #[test]
    fn views_and_limits() {
        let now = Instant::now();
        let mut r = Refs::default();
        let view = View {
            origin: Rect::from_ltrb(0, 0, 10, 10),
            scale: 1.0,
            frame: Rect::default(),
        };
        assert_eq!(r.view("a", 5, now), Err(ActError::StaleView));
        r.put_view("a", 5, 9, view, now);
        r.put_tree("a", 5, 9, vec![el("x")], now);
        assert!(r.view("a", 5, now).is_ok(), "a new tree keeps the view");
        r.put_tree("a", 5, 10, vec![el("x")], now);
        assert_eq!(
            r.view("a", 5, now),
            Err(ActError::StaleView),
            "another process: forgotten"
        );
        for h in 0..40 {
            r.put_tree(
                "a",
                100 + h,
                1,
                vec![],
                now + Duration::from_millis(h as u64),
            );
        }
        assert!(r.seen.keys().filter(|(run, _)| run == "a").count() <= MAX_WINDOWS);
    }
}
