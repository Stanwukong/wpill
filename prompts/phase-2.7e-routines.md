Start stage 2.7e (routines) of TopBar's Phase 2.7.

Read PLAN.md first: Current status, "Phase 2.7: local automation agent" (principles, what 2.7a–d built, stage 2.7e),
the Phase 2.7 (a–b), 2.7c and 2.7d IPC tables and Known limitations. Then read the CASESTUDY.md decision log,
especially D21 (the agent built on headless Claude Code), D22 (a run is `claude -p` in a Job Object, follow-ups
resume), D23 (approvals, "allow for this run", the 10-minute timeout), D25 and D27 (desktop looks and actions, the
one-time grant check, input never remembered) and C11. CLAUDE.md is loaded; follow its rules and Windows gotchas,
above all rule 1 (Rust owns state and timers), rule 2 (the island never takes focus or keyboard input) and "Agent
runs never weaken permissions". Phase 2.7a–d are signed off (2.7d on 2026-10-31, all 12 manual checks pass).

Goal: I can save an instruction I give TopBar often ("tidy my Downloads folder", "build the release installer and
copy it to Drafts", "summarise yesterday's git log in this repo") as a routine: a name, the prompt, a folder and its
limits (model, turns, budget, Claude Code install). I run it from the command window in one click, and optionally on
a schedule while TopBar is running. A routine run is an ordinary agent run: same island status, same approvals, same
Stop, same audit. Nothing about a routine makes a step pre-approved.

## What 2.7a–d give you (read the code before designing)

- `src-tauri/src/agent/`: `mod.rs` (managed `Agents`, `agent_start`, `agent_send`, `agent_stop`, `stop_all`, the
  `agent-timer` thread that expires approvals once a second, the debug `agent-test` commands), `run.rs` (command line,
  `GUIDANCE`, spawn into a Job Object), `lifecycle.rs` (run state, pure), `settings.rs` (the `agent` block of
  `%APPDATA%\dev.wpill.topbar\settings.json`: hotkeys, defaults, `approvalTimeoutSecs`), `installs.rs` (Claude Code
  installs and recent folders), `audit.rs` (run ids, jsonl trail), `approvals.rs` (pending, grants, "for this run").
- `src/command/`: the command window (`NewTask.tsx` = prompt, folder, model, turns, budget, install; `RunView.tsx` =
  transcript, pending cards, follow-ups; `CommandApp.tsx` = the run list). It's the only place with keyboard input.
- `agentFinished` (5 s notice) and `agentApproval` (sticky) already tell you how a run ends and when it waits.

## Scope for this session: stage 2.7e only

Routines: saving, editing, running, scheduling (while TopBar runs), and how their runs are shown and audited. Don't
change how approvals, desktop looks or actions work, and don't start Phase 4a/4b work (the settings window comes later;
routines are managed from the command window).

## Before coding

1. **Check what Claude Code offers** (2.1.123 on PATH, 2.1.159 via npm, 2.1.31x in VS Code): its own scheduling or
   "routines" features (cloud routines, `/loop`, cron triggers) and why TopBar's local routines are or aren't the
   same thing; whether a routine run should start fresh every time or `--resume` the last session (cost, context
   growth, stale state); and anything new in `-p` that matters for unattended runs (limits, `--max-budget-usd`).
2. **Decide the scheduling model:**
   - what kinds of schedule (daily at a time, weekdays, every N hours, at TopBar start, after login?) are worth it,
     and how they're stored (a small pure `schedule.rs` with next-run computation in local time, DST, unit-tested);
   - what happens to a run that was due while TopBar wasn't running, the PC was asleep or the screen was locked
     (skip, run once on wake/start, or ask); never a burst of catch-up runs;
   - the timer: Rust-owned (rule 1), one thread, sleeping until the next due time and woken on changes, sleep/resume
     and clock changes (`WM_POWERBROADCAST`/`WM_TIMECHANGE` via the existing power watcher, or re-checking each
     minute); never a JS timer;
   - overlap: a routine still running when it's due again (skip and say so), and a cap on concurrent runs.
3. **Decide what unattended means for approvals.** A scheduled run at 03:00 will hit approvals nobody answers (they
   deny after `approvalTimeoutSecs`). Options to weigh, and recommend one: leave it as is (the run waits, the
   approval is there in the morning, then times out); a per-routine "approvals wait until I'm back" that pauses the
   timeout while the session is locked or idle; a per-routine allow list of *non-input* steps the user ticks once
   when saving the routine (e.g. `Bash:git`, edits inside the folder), applied like "for this run", shown in the
   approval UI and the audit, never for `input` risk or compound shell commands, and never via
   `bypassPermissions`. Desktop actions must still be approved one at a time; say how a scheduled routine that needs
   them behaves (it waits, or desktop tools are disabled for scheduled runs).
4. **Propose, then wait for my OK:**
   - storage: where routines live (`settings.json` → `agent.routines`, or `%APPDATA%\dev.wpill.topbar\routines.json`),
     the schema (id, name, prompt, cwd, model, maxTurns, maxBudgetUsd, claudePath, schedule, enabled, allow list,
     lastRun), migration, and what's validated in Rust (folder exists and is long-path normalised, limits clamped);
   - commands and events (names fixed in `ipc.rs` / `ipc.ts`): e.g. `routines_list`, `routine_save`,
     `routine_delete`, `routine_run`, `routines:changed`; how a run records which routine started it (`AgentRun`
     gains `routineId` / `routineName`?), and how the audit marks it (`start` entry with `routine` and `trigger:
     manual | schedule`);
   - the command window UI: a routines list beside the run list (name, folder, schedule, last result, Run now),
     "Save as routine" from a new task or a finished run, editing and deleting, enabling a schedule; and what the
     island shows for a scheduled run (the usual spinner, approvals and `agentFinished`, perhaps the routine's name
     in place of the project). Write it as a brief for the `motion-designer` agent;
   - the tray: whether "Pause routines" belongs there;
   - limits and safety: scheduled runs only while TopBar runs (no Windows Task Scheduler entry), a sensible minimum
     interval, budgets always set, and the stop hotkey stopping scheduled runs like any other.

## Rules for this stage

- A routine never weakens permissions: TopBar still passes `--permission-mode default` and the prompt tool; nothing
  is written to the user's `~/.claude/settings.json`; desktop actions are never pre-approved.
- Every UI change goes through the `motion-designer` agent; delegate it, then review what it returns.
- **Testing:** use `topbar.exe agent-test …` in debug builds (add `agent-test routine …` commands if useful), a scratch
  folder, `haiku`, low max turns and small prompts. Test schedules with a short interval or a fake clock in the pure
  `schedule.rs` tests, not by waiting hours. Don't schedule anything that sends mouse or keyboard input; if a test
  needs a desktop action, ask me first and say which window it will touch.
- The C: drive is nearly full: watch `target\debug` and use `cargo clean --profile dev` (or clear stale
  `deps/topbar*` and `incremental`) before it fills.

## When done

cargo fmt / clippy / test and pnpm typecheck clean. Verify from outside with small haiku runs: save a routine, run it
now, edit it, a schedule a minute or two ahead that fires once, a due time missed while TopBar was closed (handled as
designed, no burst), an overlapping due time (skipped and said so), Stop and Win+Alt+Escape on a scheduled run, an
approval in a scheduled run (behaving as designed), deleting a routine with a schedule. Write the manual check list
for 2.7e in PLAN.md, update PLAN.md, CASESTUDY.md (decision entries and challenges) and CLAUDE.md (layout, gotchas),
and commit and push to main. Once I've signed off 2.7e, Phase 2.7 is complete: update the roadmap and say what's next
(Phase 4a, release readiness).
