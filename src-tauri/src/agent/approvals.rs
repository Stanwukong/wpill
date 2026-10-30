//! Approvals waiting for an answer, and what "allow for this run" remembers. Pure; unit-tested.
//!
//! A request either is covered by something remembered for its run (answered at once) or waits as `Pending`
//! until a click, its deadline, or the end of its run or bridge connection.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::describe;
use crate::ipc::{AgentApprovalActivity, AgentRisk};

/// Where the answer goes: the bridge connection and the JSON-RPC id of its `tools/call`.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplyTo {
    pub conn: u64,
    pub rpc_id: Value,
}

#[derive(Debug, Clone)]
pub struct Request {
    pub run_id: String,
    pub project: String,
    pub cwd: String,
    pub tool: String,
    pub input: Value,
    pub reply: ReplyTo,
}

#[derive(Debug, Clone)]
pub struct Pending {
    pub activity: AgentApprovalActivity,
    /// Sent back as `updatedInput` when allowed (unchanged).
    pub input: Value,
    pub reply: ReplyTo,
    key: Option<String>,
    deadline: Instant,
}

#[derive(Debug)]
pub enum Asked {
    /// Covered by an earlier "allow for this run".
    Remembered {
        reply: ReplyTo,
        input: Value,
    },
    Waiting(Pending),
}

#[derive(Debug, Default)]
pub struct Approvals {
    pending: Vec<Pending>,
    remembered: HashMap<String, HashSet<String>>,
    next_id: u32,
}

impl Approvals {
    pub fn ask(&mut self, req: Request, now: Instant, timeout: Duration) -> Asked {
        let key = describe::remember_key(&req.tool, &req.input);
        if let Some(key) = &key {
            if self
                .remembered
                .get(&req.run_id)
                .is_some_and(|keys| keys.contains(key))
            {
                return Asked::Remembered {
                    reply: req.reply,
                    input: req.input,
                };
            }
        }
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let pending = Pending {
            activity: AgentApprovalActivity {
                request_id: self.next_id,
                run_id: req.run_id,
                project: req.project,
                tool: describe::display_tool(&req.tool),
                summary: describe::summary(&req.tool, &req.input, &req.cwd),
                risk: describe::risk(&req.tool),
                remember_as: key.as_deref().map(describe::remember_scope),
            },
            input: req.input,
            reply: req.reply,
            key,
            deadline: now + timeout,
        };
        self.pending.push(pending.clone());
        Asked::Waiting(pending)
    }

    /// Answers `request_id`. With `remember`, later requests of the same kind in this run are allowed without
    /// asking (ignored where it isn't offered). `None` if it's no longer pending.
    pub fn decide(&mut self, request_id: u32, allow: bool, remember: bool) -> Option<Pending> {
        let i = self
            .pending
            .iter()
            .position(|p| p.activity.request_id == request_id)?;
        let p = self.pending.remove(i);
        if allow && remember && p.activity.risk != AgentRisk::Input {
            if let Some(key) = &p.key {
                self.remembered
                    .entry(p.activity.run_id.clone())
                    .or_default()
                    .insert(key.clone());
            }
        }
        Some(p)
    }

    /// Requests past their deadline (to deny).
    pub fn expired(&mut self, now: Instant) -> Vec<Pending> {
        self.take(|p| now >= p.deadline)
    }

    /// The run ended: its pending requests (to withdraw) and what it remembered.
    pub fn end_run(&mut self, run_id: &str) -> Vec<Pending> {
        self.remembered.remove(run_id);
        self.take(|p| p.activity.run_id == run_id)
    }

    /// The bridge went away: nobody is left to answer.
    pub fn drop_conn(&mut self, conn: u64) -> Vec<Pending> {
        self.take(|p| p.reply.conn == conn)
    }

    pub fn pending_for(&self, run_id: &str) -> Vec<AgentApprovalActivity> {
        self.pending
            .iter()
            .filter(|p| p.activity.run_id == run_id)
            .map(|p| p.activity.clone())
            .collect()
    }

    pub fn has_pending(&self, run_id: &str) -> bool {
        self.pending.iter().any(|p| p.activity.run_id == run_id)
    }

    fn take(&mut self, mut pred: impl FnMut(&Pending) -> bool) -> Vec<Pending> {
        let (taken, kept) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|p| pred(p));
        self.pending = kept;
        taken
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TIMEOUT: Duration = Duration::from_secs(600);

    fn req(run: &str, tool: &str, input: Value, conn: u64) -> Request {
        Request {
            run_id: run.into(),
            project: "wpill".into(),
            cwd: r"C:\w".into(),
            tool: tool.into(),
            input,
            reply: ReplyTo {
                conn,
                rpc_id: json!(7),
            },
        }
    }

    fn waiting(asked: Asked) -> Pending {
        match asked {
            Asked::Waiting(p) => p,
            Asked::Remembered { .. } => panic!("expected to wait"),
        }
    }

    #[test]
    fn remember_for_this_run_only() {
        let mut a = Approvals::default();
        let now = Instant::now();
        let git = || json!({ "command": "git status" });
        let p = waiting(a.ask(req("r1", "Bash", git(), 1), now, TIMEOUT));
        assert_eq!(p.activity.remember_as.as_deref(), Some("git"));
        assert_eq!(p.activity.summary, "git status");
        assert_eq!(p.activity.risk, AgentRisk::Change);
        assert!(a.decide(p.activity.request_id, true, true).is_some());
        assert!(matches!(
            a.ask(
                req("r1", "Bash", json!({ "command": "git push" }), 1),
                now,
                TIMEOUT
            ),
            Asked::Remembered { .. }
        ));
        // Another run still asks; so does a different program, or a compound command.
        waiting(a.ask(req("r2", "Bash", git(), 2), now, TIMEOUT));
        waiting(a.ask(
            req("r1", "Bash", json!({ "command": "npm i" }), 1),
            now,
            TIMEOUT,
        ));
        let compound = waiting(a.ask(
            req("r1", "Bash", json!({ "command": "git a && rm b" }), 1),
            now,
            TIMEOUT,
        ));
        assert_eq!(compound.activity.remember_as, None);
        // Ending the run forgets it.
        a.end_run("r1");
        waiting(a.ask(req("r1", "Bash", git(), 1), now, TIMEOUT));
    }

    #[test]
    fn deny_and_once_remember_nothing() {
        let mut a = Approvals::default();
        let now = Instant::now();
        let edit = || json!({ "file_path": r"C:\w\a.rs" });
        let p = waiting(a.ask(req("r1", "Edit", edit(), 1), now, TIMEOUT));
        a.decide(p.activity.request_id, false, true);
        let p = waiting(a.ask(req("r1", "Edit", edit(), 1), now, TIMEOUT));
        a.decide(p.activity.request_id, true, false);
        waiting(a.ask(req("r1", "Write", edit(), 1), now, TIMEOUT));
        assert!(a.decide(999, true, false).is_none());
    }

    #[test]
    fn input_is_never_remembered() {
        let mut a = Approvals::default();
        let now = Instant::now();
        let p = waiting(a.ask(req("r1", "mcp__topbar__click", json!({}), 1), now, TIMEOUT));
        assert_eq!(p.activity.remember_as, None);
        assert_eq!(p.activity.risk, AgentRisk::Input);
        a.decide(p.activity.request_id, true, true);
        waiting(a.ask(req("r1", "mcp__topbar__click", json!({}), 1), now, TIMEOUT));
    }

    #[test]
    fn timeouts_runs_and_connections() {
        let mut a = Approvals::default();
        let now = Instant::now();
        let p1 = waiting(a.ask(
            req("r1", "Bash", json!({ "command": "a" }), 1),
            now,
            TIMEOUT,
        ));
        let p2 = waiting(a.ask(
            req("r2", "Bash", json!({ "command": "b" }), 2),
            now + Duration::from_secs(5),
            TIMEOUT,
        ));
        assert!(a.has_pending("r1"));
        assert_eq!(a.pending_for("r2").len(), 1);
        let gone = a.expired(now + TIMEOUT);
        assert_eq!(gone.len(), 1);
        assert_eq!(gone[0].activity.request_id, p1.activity.request_id);
        assert!(!a.has_pending("r1"));
        let gone = a.drop_conn(2);
        assert_eq!(gone[0].activity.request_id, p2.activity.request_id);
        assert!(a.expired(now + TIMEOUT * 2).is_empty());
    }
}
