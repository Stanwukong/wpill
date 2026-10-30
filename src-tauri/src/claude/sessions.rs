//! Claude Code sessions keyed by Claude's `session_id`: status, and which activities to offer or withdraw.
//! Pure; unit-tested. The listener thread owns the only instance.

use crate::ipc::{
    Activity, ClaudeDoneActivity, ClaudeNeedsInputActivity, ClaudeSession, ClaudeStatus,
};

use super::protocol::{Event, Message, Proc};

/// A session with no events for this long is dropped (its terminal may be long gone).
pub const EXPIRE_MS: i64 = 12 * 60 * 60 * 1000;
/// More than this many open sessions drops the least recently active.
const MAX_SESSIONS: usize = 32;
/// Shown when `cwd` is empty or a drive root.
const FALLBACK_PROJECT: &str = "Claude Code";
const MAX_PROJECT_CHARS: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    /// Ours, for IPC and activity keys.
    pub id: u32,
    claude_id: String,
    pub project: String,
    pub status: ClaudeStatus,
    pub updated_at_ms: i64,
    last_event_ms: i64,
    /// The forwarder's ancestors, nearest first: used to find the window and check the session is alive.
    pub chain: Vec<Proc>,
}

impl Session {
    pub fn ipc(&self) -> ClaudeSession {
        ClaudeSession {
            session_id: self.id,
            project: self.project.clone(),
            status: self.status,
            updated_at_ms: self.updated_at_ms,
        }
    }

    /// The Claude Code process itself (nearest `claude.exe`, or `node.exe` for older npm installs).
    pub fn process(&self) -> Option<&Proc> {
        self.chain.iter().find(|p| {
            p.exe.eq_ignore_ascii_case("claude.exe") || p.exe.eq_ignore_ascii_case("node.exe")
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Offer(Activity),
    /// Withdraw this session's activity (it got its answer, started new work, or ended).
    Retract(u32),
}

#[derive(Debug, Default)]
pub struct Sessions {
    list: Vec<Session>,
    next_id: u32,
}

impl Sessions {
    pub fn list(&self) -> &[Session] {
        &self.list
    }

    /// Applies one hook. Returns whether the session list changed (to emit `claude:sessions`) and what to
    /// tell the activity queue.
    pub fn apply(&mut self, msg: Message) -> (bool, Vec<Effect>) {
        let found = self.list.iter().position(|s| s.claude_id == msg.session);
        if msg.event == Event::SessionEnd {
            return match found {
                Some(i) if msg.at_ms >= self.list[i].last_event_ms => {
                    let s = self.list.remove(i);
                    (true, vec![Effect::Retract(s.id)])
                }
                _ => (false, vec![]),
            };
        }
        // Sessions that started before the hooks were installed show up on their first event.
        let (i, created) = match found {
            Some(i) => (i, false),
            None => (self.create(&msg), true),
        };
        let s = &mut self.list[i];
        if msg.at_ms < s.last_event_ms {
            return (created, vec![]);
        }
        s.last_event_ms = msg.at_ms;
        if !msg.chain.is_empty() {
            s.chain = msg.chain;
        }

        let before = s.status;
        let needs_input = || {
            Effect::Offer(Activity::ClaudeNeedsInput(ClaudeNeedsInputActivity {
                session_id: s.id,
                project: s.project.clone(),
            }))
        };
        let done = |failed| {
            Effect::Offer(Activity::ClaudeDone(ClaudeDoneActivity {
                session_id: s.id,
                project: s.project.clone(),
                failed,
            }))
        };
        let (status, effect) = match msg.event {
            Event::SessionStart { compact: true } => (before, None),
            Event::SessionStart { compact: false } => {
                (ClaudeStatus::Idle, Some(Effect::Retract(s.id)))
            }
            // A new task makes a "done" from the last one old news.
            Event::Prompt => (ClaudeStatus::Working, Some(Effect::Retract(s.id))),
            Event::ToolDone => {
                let answered = before == ClaudeStatus::NeedsInput;
                (
                    ClaudeStatus::Working,
                    answered.then_some(Effect::Retract(s.id)),
                )
            }
            Event::NeedsInput => (ClaudeStatus::NeedsInput, Some(needs_input())),
            Event::Idle if before == ClaudeStatus::Working => (ClaudeStatus::Done, None),
            Event::Idle => (before, None),
            Event::Stop => (ClaudeStatus::Done, Some(done(false))),
            Event::StopFailure => (ClaudeStatus::Done, Some(done(true))),
            Event::SessionEnd => unreachable!("handled above"),
        };
        let changed = created || status != before;
        if status != before {
            s.status = status;
            s.updated_at_ms = msg.at_ms;
        }
        (changed, effect.into_iter().collect())
    }

    /// Drops sessions whose Claude Code process is gone (`alive` false) or that have been silent for
    /// `EXPIRE_MS`. Returns whether anything was dropped, and the retractions.
    pub fn prune(&mut self, now_ms: i64, alive: impl Fn(&Session) -> bool) -> (bool, Vec<Effect>) {
        let mut effects = Vec::new();
        self.list.retain(|s| {
            let keep = now_ms - s.last_event_ms < EXPIRE_MS && alive(s);
            if !keep {
                effects.push(Effect::Retract(s.id));
            }
            keep
        });
        (!effects.is_empty(), effects)
    }

    fn create(&mut self, msg: &Message) -> usize {
        if self.list.len() >= MAX_SESSIONS {
            if let Some(oldest) = (0..self.list.len()).min_by_key(|&i| self.list[i].last_event_ms) {
                self.list.remove(oldest);
            }
        }
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.list.push(Session {
            id: self.next_id,
            claude_id: msg.session.clone(),
            project: project(&msg.cwd),
            status: ClaudeStatus::Idle,
            updated_at_ms: msg.at_ms,
            last_event_ms: i64::MIN,
            chain: Vec::new(),
        });
        self.list.len() - 1
    }
}

/// The IPC view of `list`, most recently updated first.
pub fn snapshot(list: &[Session]) -> Vec<ClaudeSession> {
    let mut out: Vec<_> = list.iter().map(Session::ipc).collect();
    out.sort_by_key(|s| std::cmp::Reverse(s.updated_at_ms));
    out
}

/// The folder name of `cwd` (either slash), shortened for the island.
pub fn project(cwd: &str) -> String {
    let name = cwd
        .trim_end_matches(['\\', '/'])
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or("");
    if name.is_empty() || name.ends_with(':') {
        return FALLBACK_PROJECT.into();
    }
    name.chars().take(MAX_PROJECT_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(session: &str, at_ms: i64, event: Event) -> Message {
        Message {
            at_ms,
            session: session.into(),
            cwd: r"C:\Users\me\GitHub\wpill".into(),
            event,
            chain: vec![Proc {
                pid: 10,
                exe: "claude.exe".into(),
                created: 1,
            }],
        }
    }

    fn status(s: &Sessions, i: usize) -> ClaudeStatus {
        s.list()[i].status
    }

    #[test]
    fn a_task_from_prompt_to_done() {
        let mut s = Sessions::default();
        let (changed, fx) = s.apply(msg("a", 1, Event::SessionStart { compact: false }));
        assert!(changed);
        let id = s.list()[0].id;
        assert_eq!(fx, vec![Effect::Retract(id)]);
        assert_eq!(status(&s, 0), ClaudeStatus::Idle);
        assert_eq!(s.list()[0].project, "wpill");

        let (changed, fx) = s.apply(msg("a", 2, Event::Prompt));
        assert!(changed);
        assert_eq!(fx, vec![Effect::Retract(id)]);
        assert_eq!(status(&s, 0), ClaudeStatus::Working);

        // Tools while working change nothing visible.
        assert_eq!(s.apply(msg("a", 3, Event::ToolDone)), (false, vec![]));

        let (changed, fx) = s.apply(msg("a", 4, Event::Stop));
        assert!(changed);
        assert!(matches!(&fx[..], [Effect::Offer(Activity::ClaudeDone(d))]
            if d.session_id == id && d.project == "wpill" && !d.failed));
        assert_eq!(s.list()[0].updated_at_ms, 4);
    }

    #[test]
    fn a_permission_prompt_is_withdrawn_once_work_goes_on() {
        let mut s = Sessions::default();
        s.apply(msg("a", 1, Event::Prompt));
        let id = s.list()[0].id;
        let (_, fx) = s.apply(msg("a", 2, Event::NeedsInput));
        assert!(
            matches!(&fx[..], [Effect::Offer(Activity::ClaudeNeedsInput(n))] if n.session_id == id)
        );
        assert_eq!(status(&s, 0), ClaudeStatus::NeedsInput);
        let (changed, fx) = s.apply(msg("a", 3, Event::ToolDone));
        assert!(changed);
        assert_eq!(fx, vec![Effect::Retract(id)]);
        assert_eq!(status(&s, 0), ClaudeStatus::Working);
    }

    #[test]
    fn idle_prompt_only_corrects_a_stuck_working_status() {
        let mut s = Sessions::default();
        // Esc: Stop never fires, idle_prompt comes about a minute later.
        s.apply(msg("a", 1, Event::Prompt));
        let (changed, fx) = s.apply(msg("a", 2, Event::Idle));
        assert!(changed && fx.is_empty());
        assert_eq!(status(&s, 0), ClaudeStatus::Done);
        // After a real Stop it changes nothing.
        s.apply(msg("a", 3, Event::Prompt));
        s.apply(msg("a", 4, Event::Stop));
        assert_eq!(s.apply(msg("a", 5, Event::Idle)), (false, vec![]));
    }

    #[test]
    fn api_errors_finish_as_failed() {
        let mut s = Sessions::default();
        s.apply(msg("a", 1, Event::Prompt));
        let (_, fx) = s.apply(msg("a", 2, Event::StopFailure));
        assert!(matches!(&fx[..], [Effect::Offer(Activity::ClaudeDone(d))] if d.failed));
    }

    #[test]
    fn late_async_events_are_ignored() {
        let mut s = Sessions::default();
        s.apply(msg("a", 10, Event::Stop));
        // A PostToolUse forwarded before the Stop but delivered after it.
        assert_eq!(s.apply(msg("a", 9, Event::ToolDone)), (false, vec![]));
        assert_eq!(status(&s, 0), ClaudeStatus::Done);
        assert_eq!(s.apply(msg("a", 8, Event::SessionEnd)), (false, vec![]));
        assert_eq!(s.list().len(), 1);
    }

    #[test]
    fn compaction_keeps_the_status_and_end_removes_the_session() {
        let mut s = Sessions::default();
        s.apply(msg("a", 1, Event::Prompt));
        s.apply(msg("a", 2, Event::SessionStart { compact: true }));
        assert_eq!(status(&s, 0), ClaudeStatus::Working);
        let id = s.list()[0].id;
        assert_eq!(
            s.apply(msg("a", 3, Event::SessionEnd)),
            (true, vec![Effect::Retract(id)])
        );
        assert!(s.list().is_empty());
        assert_eq!(s.apply(msg("b", 4, Event::SessionEnd)), (false, vec![]));
    }

    #[test]
    fn sessions_are_separate_and_listed_most_recent_first() {
        let mut s = Sessions::default();
        s.apply(msg("a", 1, Event::Prompt));
        let mut other = msg("b", 2, Event::Prompt);
        other.cwd = "D:/work/api/".into();
        s.apply(other);
        assert_ne!(s.list()[0].id, s.list()[1].id);
        let snap = snapshot(s.list());
        assert_eq!(snap[0].project, "api");
        assert_eq!(snap[1].project, "wpill");
        s.apply(msg("a", 3, Event::Stop));
        assert_eq!(snapshot(s.list())[0].project, "wpill");
    }

    #[test]
    fn prune_drops_dead_and_silent_sessions() {
        let mut s = Sessions::default();
        s.apply(msg("a", 1_000, Event::Prompt));
        s.apply(msg("b", 2_000, Event::Prompt));
        let (a, b) = (s.list()[0].id, s.list()[1].id);
        assert_eq!(s.prune(3_000, |_| true), (false, vec![]));
        assert_eq!(
            s.prune(3_000, |x| x.id != a),
            (true, vec![Effect::Retract(a)])
        );
        assert_eq!(
            s.prune(2_000 + EXPIRE_MS, |_| true),
            (true, vec![Effect::Retract(b)])
        );
        assert!(s.list().is_empty());
    }

    #[test]
    fn finds_the_claude_process_in_the_chain() {
        let mut s = Sessions::default();
        let mut m = msg("a", 1, Event::Prompt);
        m.chain = vec![
            Proc {
                pid: 1,
                exe: "bash.exe".into(),
                created: 1,
            },
            Proc {
                pid: 2,
                exe: "Claude.EXE".into(),
                created: 1,
            },
            Proc {
                pid: 3,
                exe: "WindowsTerminal.exe".into(),
                created: 1,
            },
        ];
        s.apply(m);
        assert_eq!(s.list()[0].process().map(|p| p.pid), Some(2));
    }

    #[test]
    fn project_names() {
        assert_eq!(project(r"C:\Users\me\GitHub\wpill"), "wpill");
        assert_eq!(project(r"C:\Users\me\wpill\"), "wpill");
        assert_eq!(project("/home/me/api"), "api");
        assert_eq!(project(r"C:\"), FALLBACK_PROJECT);
        assert_eq!(project(""), FALLBACK_PROJECT);
        assert_eq!(project(&"x".repeat(100)).len(), MAX_PROJECT_CHARS);
    }
}
