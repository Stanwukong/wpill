Start Phase 2.7 (local automation agent) of TopBar.

Read PLAN.md first: Current status, Roadmap, "Phase 2.7: local automation agent" (goal, principles, stages 2.7a–e,
draft manual check), the IPC contract (Phase 2.6 table and the reserved 2.7 names) and Known limitations. Then read
the CASESTUDY.md decision log, especially D14 (activities are a hover phase), D17 and D20 (hook forwarder, per-user
pipe, shell-form hooks), D19 (sticky notices, `Signal::Retract`), D18 (motion-designer agent) and D21 (this phase's
direction and the alternatives already rejected). CLAUDE.md is loaded; follow its rules and Windows gotchas, above
all rule 2 (the island never takes keyboard input). Phase 2.6 is signed off (2026-10-30).

Goal: I press a hotkey, type an instruction and pick a folder; TopBar runs a headless Claude Code session for it,
shows progress in the island, asks me to Allow / Deny risky steps from the island, and tells me when it's done.
Later stages give Claude desktop tools (windows, screenshots, UI Automation, then clicking and typing) through an MCP
server inside TopBar.

## What Phase 2.6 gives you (read the code before designing)

- `src-tauri/src/claude/`: `forward.rs` (`topbar.exe hook`, run from `main.rs` before Tauri), `pipe.rs` (current-user
  pipe, overlapped server loop with a stop event), `process.rs` (ancestor chain, liveness by pid + creation time),
  `sessions.rs` (session state keyed by Claude's session id; pure, tested), `focus.rs`, `hooks.rs`, `mod.rs`.
- `src-tauri/src/activity/`: `Signal::{Offer, Retract}`, the `Watcher` harness (`Activities::sender`, `add_watcher`),
  and `queue.rs` with sticky notices (no end time, step aside for other notices, parked under the panel, withdrawn by
  key). Interactive activities take clicks only while hovered (`hover/logic.rs`).
- `src/features/claude/` and `ClaudeActivity.tsx`: the spinner, the orange edge (`data-claude`), the panel section.
- The hooks also fire for headless `claude -p` runs, so an agent run will show up in 2.6's session list by itself.

## Scope for this session: stages 2.7a and 2.7b

Design all of 2.7 at the level PLAN.md describes, but build only **2.7a (command window + headless runs)** and
**2.7b (approvals in the island)**. Stages c–e get their own sessions after I've used a and b.

Include the **Claude UI refresh** listed under 2.7a in PLAN.md: the status line ("Done", "Needs your input",
"Stopped") is too large next to the project name, so make it smaller and quieter; and give Claude content its own
typeface (a font that's on Windows or bundled under an open licence, with a fallback stack; no Anthropic brand fonts),
used consistently in the 2.6 activities, the panel section and the new agent UI. Have the `motion-designer` agent
render two or three font candidates side by side (plain and tinted island) so I can pick one before it's applied.

## Before coding

1. **Check the current Claude Code docs** (headless mode / CLI reference / permissions / MCP) and confirm against the
   installed binaries (`claude --version`; there are three installs: 2.1.123 on PATH, 2.1.159 via npm, the VS Code
   extension's 2.1.312). Pin down: every stream-json event type and the fields we'd use (init/session id, assistant
   tool use, result with turns/cost/error); how `--input-format stream-json` takes follow-up messages; the exact input
   and reply schema of a `--permission-prompt-tool` tool (allow with `updatedInput`, deny with a message); whether
   `--strict-mcp-config` should be used (it would hide my own MCP servers); how `--max-budget-usd` behaves on a
   subscription; what `--permission-mode default` does when my settings say otherwise; and whether hooks fire for
   `-p` runs with the same session id (so 2.6 can mark the run as an agent run). Note anything that differs from PLAN.md.
2. **Check the Win32/MCP pieces:** `RegisterHotKey` (needs a message-only window thread, like `power.rs`), Job
   Objects that kill the whole tree (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`), `CREATE_NO_WINDOW` for `claude.exe`, a
   second Tauri window that *is* focusable next to the unfocusable island, and the MCP stdio protocol (JSON-RPC 2.0
   messages, `initialize`, `tools/list`, `tools/call`) for the `topbar.exe mcp` bridge. Check the `windows` 0.62
   feature names for anything new (e.g. `Win32_System_JobObjects`, `Win32_UI_Input_KeyboardAndMouse` is already on).
3. **Propose, then wait for my OK:**
   - module layout on both sides (`src-tauri/src/agent/`, `src/features/agent/`, the `command` window's entry point);
   - the run lifecycle (states, limits, stop, resume, TopBar quitting mid-run) and how it joins 2.6's sessions without
     double activities;
   - the bridge protocol (pipe name, framing, the per-run token, size caps, what happens when TopBar isn't running);
   - the approval flow end to end (tool call → summary → `agentApproval` activity → click → reply), its timeout,
     "remember for this run", several runs waiting at once, and how approvals behave with the panel open and in
     fullscreen;
   - the risk classes (`read` / `change` / `input`) and how a Claude Code tool maps to one;
   - the exact `claude` command line TopBar runs, and the agent guidance for `--append-system-prompt`;
   - the IPC names (reserved in PLAN.md; adjust if research says so) and the audit log format;
   - the hotkey default, and the UI as a brief for the `motion-designer` agent (command window, agent badge and Stop
     in the panel, the approval activity with Allow / Deny, `agentFinished`).

## Rules for this phase

- Never run Claude Code with `bypassPermissions` or `--dangerously-skip-permissions`, and never weaken my real
  `~/.claude/settings.json`; pass everything per run on the command line or in a generated config under TopBar's
  app data folder.
- Approvals are clicks in the island; typing happens only in the `command` window (rule 2).
- Every UI change goes through the `motion-designer` agent; delegate it, then review what it returns.
- Tests and verification must not spend much of my usage: use small prompts, a cheap model (`--model haiku`), and low
  `--max-turns` for real runs. Unit-test the pure parts (stream-json parsing, run state machine, approval queue rules,
  risk mapping, MCP message handling).

## When done

cargo fmt / clippy / test and pnpm typecheck clean. Verify from outside: start runs through the command window and
directly through the commands, deny and allow an approval, stop a run that started a long-running child
(`ping -t localhost`) and check nothing is left, quit TopBar mid-run, and capture the island at each state. Finalize
the manual check list in PLAN.md (the draft is there), update PLAN.md, CASESTUDY.md (decision entries for whatever you
decide, challenges you hit) and CLAUDE.md (layout, commands, gotchas), and commit and push to main. Once I've signed
off 2.7a–b, write the kick-off prompt for 2.7c.
