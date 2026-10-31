//! Approvals waiting for an answer, and what "allow for this run" remembers. Pure; unit-tested.
//!
//! A request either is covered by something remembered for its run (answered at once) or waits as `Pending`
//! until a click, its deadline, or the end of its run or bridge connection.
//!
//! Desktop looks (2.7c) are checked twice. Claude Code's `approve` call is the usual prompt; allowing it leaves a
//! one-time *grant* for that exact input. When the look's own `tools/call` arrives it must find a grant or a
//! remembered "for this run"; otherwise (a user allow rule skipped the prompt) TopBar asks itself, with
//! `Purpose::Look`, so every look is approved in the island whatever Claude Code decided.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::{actions, describe};
use crate::ipc::{AgentApprovalActivity, AgentRisk};

/// Grants kept per run at most (approved prompts whose call never came).
const MAX_GRANTS: usize = 32;

/// Where the answer goes: the bridge connection and the JSON-RPC id of its `tools/call`.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplyTo {
    pub conn: u64,
    pub rpc_id: Value,
}

/// What an approval answers: Claude Code's permission prompt, or TopBar's own check before a desktop look.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Purpose {
    Prompt,
    Look,
}

#[derive(Debug, Clone)]
pub struct Request {
    pub run_id: String,
    pub project: String,
    pub cwd: String,
    pub tool: String,
    pub input: Value,
    pub reply: ReplyTo,
    pub purpose: Purpose,
    /// For a desktop look or action on a window: its name ("Chrome · GitHub") and process, seen when asking.
    pub target: Option<String>,
    pub target_pid: Option<u32>,
    /// For an action on an element: how it's named (`"Save"`), from the run's latest `ui_tree`.
    pub element: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Pending {
    pub activity: AgentApprovalActivity,
    /// Sent back as `updatedInput` when allowed (unchanged).
    pub input: Value,
    pub reply: ReplyTo,
    pub purpose: Purpose,
    pub tool: String,
    pub target_pid: Option<u32>,
    key: Option<String>,
    deadline: Instant,
}

#[derive(Debug)]
// Short-lived and moved once; boxing `Pending` would only add an allocation per request.
#[allow(clippy::large_enum_variant)]
pub enum Asked {
    /// Covered by an earlier "allow for this run".
    Remembered {
        reply: ReplyTo,
        input: Value,
        target_pid: Option<u32>,
    },
    Waiting(Pending),
}

#[derive(Debug, Clone)]
struct Grant {
    run_id: String,
    tool: String,
    input: Value,
    pid: Option<u32>,
}

#[derive(Debug, Default)]
pub struct Approvals {
    pending: Vec<Pending>,
    remembered: HashMap<String, HashSet<String>>,
    grants: Vec<Grant>,
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
                if req.purpose == Purpose::Prompt {
                    self.add_grant(&req.run_id, &req.tool, &req.input, req.target_pid);
                }
                return Asked::Remembered {
                    reply: req.reply,
                    input: req.input,
                    target_pid: req.target_pid,
                };
            }
        }
        let action = describe::is_action(&req.tool);
        let desktop = action || describe::is_desktop(&req.tool);
        let summary = if action {
            describe::action_summary(
                &req.tool,
                &req.input,
                req.target.as_deref(),
                req.element.as_deref(),
            )
        } else if desktop {
            describe::desktop_summary(&req.tool, &req.input, req.target.as_deref())
        } else {
            describe::summary(&req.tool, &req.input, &req.cwd)
        };
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let pending = Pending {
            activity: AgentApprovalActivity {
                request_id: self.next_id,
                run_id: req.run_id,
                project: req.project,
                tool: describe::display_tool(&req.tool),
                summary,
                risk: describe::risk(&req.tool),
                remember_as: key.as_deref().map(describe::remember_scope),
                desktop,
                detail: actions::detail(&req.tool, &req.input),
            },
            input: req.input,
            reply: req.reply,
            purpose: req.purpose,
            tool: req.tool,
            target_pid: req.target_pid,
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
        if allow && p.purpose == Purpose::Prompt {
            let (run, tool, input) = (p.activity.run_id.clone(), p.tool.clone(), p.input.clone());
            self.add_grant(&run, &tool, &input, p.target_pid);
        }
        Some(p)
    }

    /// A desktop look's `tools/call` arrived: uses up the grant its approved prompt left. `Some(pid)` (the
    /// target's process when it was approved, if any) when there was one.
    pub fn take_grant(&mut self, run_id: &str, tool: &str, input: &Value) -> Option<Option<u32>> {
        let i = self
            .grants
            .iter()
            .position(|g| g.run_id == run_id && g.tool == tool && &g.input == input)?;
        Some(self.grants.remove(i).pid)
    }

    fn add_grant(&mut self, run_id: &str, tool: &str, input: &Value, pid: Option<u32>) {
        if !describe::is_desktop(tool) && !describe::is_action(tool) {
            return;
        }
        if self.grants.iter().filter(|g| g.run_id == run_id).count() >= MAX_GRANTS {
            if let Some(i) = self.grants.iter().position(|g| g.run_id == run_id) {
                self.grants.remove(i);
            }
        }
        self.grants.push(Grant {
            run_id: run_id.into(),
            tool: tool.into(),
            input: input.clone(),
            pid,
        });
    }

    /// Requests past their deadline (to deny).
    pub fn expired(&mut self, now: Instant) -> Vec<Pending> {
        self.take(|p| now >= p.deadline)
    }

    /// The run ended: its pending requests (to withdraw), what it remembered, and its grants.
    pub fn end_run(&mut self, run_id: &str) -> Vec<Pending> {
        self.remembered.remove(run_id);
        self.grants.retain(|g| g.run_id != run_id);
        self.take(|p| p.activity.run_id == run_id)
    }

    /// Claude Code cancelled the call (`notifications/cancelled`): withdraw its approval.
    pub fn cancel(&mut self, conn: u64, rpc_id: &Value) -> Vec<Pending> {
        self.take(|p| p.reply.conn == conn && &p.reply.rpc_id == rpc_id)
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
            purpose: Purpose::Prompt,
            target: None,
            target_pid: None,
            element: None,
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
        assert!(!p.activity.desktop);
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
        assert_eq!(
            a.take_grant("r1", "Edit", &edit()),
            None,
            "grants are for looks only"
        );
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
    fn desktop_looks_need_a_grant_or_a_remembered_kind() {
        let mut a = Approvals::default();
        let now = Instant::now();
        let shot = "mcp__topbar__screenshot";
        let input = || json!({ "hwnd": 42 });
        let p = waiting(a.ask(
            Request {
                target: Some("Chrome · GitHub".into()),
                target_pid: Some(9),
                ..req("r1", shot, input(), 1)
            },
            now,
            TIMEOUT,
        ));
        assert!(p.activity.desktop);
        assert_eq!(p.activity.risk, AgentRisk::Read);
        assert_eq!(p.activity.summary, "Screenshot of Chrome · GitHub");
        assert_eq!(p.activity.tool, "desktop · screenshot");
        assert_eq!(p.activity.remember_as.as_deref(), Some("screenshots"));
        // No grant before it's allowed, and none after a deny.
        assert_eq!(a.take_grant("r1", shot, &input()), None);
        a.decide(p.activity.request_id, true, false);
        assert_eq!(a.take_grant("r1", shot, &json!({ "hwnd": 43 })), None);
        assert_eq!(a.take_grant("r1", shot, &input()), Some(Some(9)));
        assert_eq!(a.take_grant("r1", shot, &input()), None, "used up");
        let denied = waiting(a.ask(req("r1", shot, input(), 1), now, TIMEOUT));
        a.decide(denied.activity.request_id, false, false);
        assert_eq!(a.take_grant("r1", shot, &input()), None);

        // A call without a prompt (a user allow rule) is asked about by TopBar itself; that leaves no grant.
        let own = waiting(a.ask(
            Request {
                purpose: Purpose::Look,
                ..req("r1", shot, input(), 1)
            },
            now,
            TIMEOUT,
        ));
        a.decide(own.activity.request_id, true, true);
        assert_eq!(a.take_grant("r1", shot, &input()), None);
        // Remembered for the run: another window's prompt is answered at once and leaves a grant.
        let remembered = a.ask(req("r1", shot, json!({ "hwnd": 7 }), 1), now, TIMEOUT);
        assert!(matches!(remembered, Asked::Remembered { .. }));
        assert_eq!(a.take_grant("r1", shot, &json!({ "hwnd": 7 })), Some(None));
        // Other kinds still ask; ending the run drops grants.
        waiting(a.ask(req("r1", "mcp__topbar__ui_tree", input(), 1), now, TIMEOUT));
        a.ask(req("r1", shot, json!({}), 1), now, TIMEOUT);
        a.end_run("r1");
        assert_eq!(a.take_grant("r1", shot, &json!({})), None);
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
