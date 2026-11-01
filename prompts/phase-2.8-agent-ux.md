Complete Phase 2.8 (agent UX) of Cay: stages 2.8b to 2.8f, in this one session.

Read PLAN.md first: Current status, "Phase 2.8: agent UX" (2.8a is done and signed off; its items and manual check show
what was built), the IPC section, Known limitations and Open questions. Then CASESTUDY.md: D2 (one window sized to
the largest panel), D21–D23 (agent runs, approvals), D29 (routines never pre-approve), D30 (own look; never Claude
Code's spinner, `✻` or orange), D34 (this phase, permission modes but never bypass) and D36 (how 2.8a holds terminal
sessions' requests). CLAUDE.md is loaded; follow its rules and Windows gotchas, above all rule 1 (Rust owns state, no
timers in React), rule 2 (the island is never activated; keyboard input goes in the command window), "Agent runs
never weaken permissions" and the hook forwarder's rules (it prints only a decision you made, every wait bounded).

Where 2.8a left things (2026-11-01): `cay.exe hook` holds a `PermissionRequest` on the `cay-claude-ask-<SID>` pipe
(`claude/ask.rs` on the wire, `claude/asks.rs` in Cay, `duplex.rs` shared with the agent pipe); the island shows a
`claudeApproval` card (`ApprovalCard` in `src/features/activities/AgentApproval.tsx`, shared with agent runs); only a
click decides, everything else replies `pass` and the terminal asks. `AskUserQuestion` and `ExitPlanMode` are
deliberately not held yet: that's 2.8c. Summaries come from `agent/describe.rs` through `agent::describe_tool`.

Goal: everything Claude asks of you, from a terminal session or an agent run, can be read and answered properly from
Cay, and you can see what it's doing, without Cay ever deciding or widening anything you didn't choose.

## Work (one stage at a time, in this order; each ends with its own commit and doc updates)

**2.8b See what you approve.** The card shows enough to decide: for `Edit` / `MultiEdit` / `Write` a compact diff
(first few changed lines, `+n −m`, Cascadia Mono), for shell the whole command wrapped at token boundaries, never cut
mid-token. Built in Rust (`describe.rs`, pure and unit-tested) and sent in the activity payload for both card kinds;
the card grows to a cap within D2's size limits, then says "+12 more lines". The full diff opens in the command
window on click (for a session, which has no run, decide with me where it shows). Mind 2.8a's 16 KB cut per string in
`ask.rs`: say "cut" rather than show a wrong count.

**2.8c Questions and plans.** Hold `AskUserQuestion` and `ExitPlanMode` too. Per the hooks docs, an allow for these
must carry the answer in `updatedInput` (for questions: the original `questions` plus an `answers` map of question
text → option label, multi-select joined by commas), so extend `ask::Decision` with an answer payload, never a bare
allow. Options become buttons on the card; a free-text answer, and the plan (Markdown, Cascadia Mono, Approve / Send
feedback), go in the command window, while the island shows "Plan ready" as an activity. Do the same for agent runs
through the prompt tool (`mcp__cay__approve`), which removes "Claude can't ask mid-run" from Known limitations. Fetch
the current hooks docs (code.claude.com/docs/en/hooks) for the exact shapes before coding; don't guess them.

**2.8d Live step line.** The current tool and target beside the wave ("Edit · hover/logic.rs") and `n/m` from
`TodoWrite`, in the pill and on panel rows, for sessions (add a `PreToolUse` hook: background, no output, so the
entries change and Connect must offer an update, like 2.8a) and agent runs (stream-json). Rust sends updates; no React
timers; nothing new is approved or allowed.

**2.8e Permission modes.** As specified in PLAN.md 2.8e: Ask → Accept edits → Plan for agent runs, chosen per task in
the command window (default Ask), shown as a chip, cycled by clicking it while hovered; never `bypassPermissions`;
routines stay at Ask; desktop tools, shell, web and MCP still ask in every mode; the audit records each change.
Start with a spike: whether a running `claude -p` stream-json session accepts a `control_request` to set the mode
(test with haiku in a scratch folder via `agent-test`); if not, the change applies from the next follow-up and the
chip says so. Terminal sessions show their mode (`permission_mode` from hooks) on their row; Cay never changes it.
Then update CLAUDE.md's "Agent runs never weaken permissions" gotcha (the user picks Accept edits; Cay never does).
Propose the design and the spike's result before building, and wait for my OK.

**2.8f Connect you can trust, and small things.**
- Connect already shows the change and keeps a dated backup (`claude/mod.rs` `run_dialog`, `hooks::preview`); check
  that the preview matches what's written, that Disconnect removes only Cay's entries, and that an out-of-date
  install says "update" rather than looking disconnected. Close the gaps; don't rebuild what works.
- Usage limits (5 h / 7 d) in the panel's Claude section, from Claude Code's status line input, no network call.
  That needs a `statusLine` entry in `~/.claude/settings.json`, and the user may already have one: propose how it
  coexists (never replace theirs silently) and wait for my OK.
- An optional soft sound for "needs you", off by default (a `settings.json` switch until Phase 4b).
- Spike: focus the Windows Terminal tab, not just the window. Report what's possible; build it only if it's reliable.

## Rules for this phase

- Every UI change goes through the `motion-designer` agent; delegate it with a precise brief, then review what it
  returns (renders, `pnpm typecheck`).
- Nothing in Cay may allow, deny or answer for you. Timeouts and errors hand back to the terminal or the run's own
  timeout; a held request you never saw is never refused for you. `updatedPermissions` is never sent.
- Don't write `~/.claude/settings.json` in tests: use hook JSON files written with the Write tool and
  `claude -p … --settings <file>` with a recent Claude Code (2.1.314 under the VS Code extension; the 2.1.123 on PATH
  doesn't run `PermissionRequest` hooks in `-p`). Use haiku, a scratch folder, low max turns.
- Don't edit Rust while I'm driving `pnpm tauri dev` tests without saying so (it restarts Cay); check `Get-Process
  cay` before `agent-test`. No synthetic input while I'm at the machine.
- Commits: small, imperative, prefixed `phase2.8:`; push to `main`, no new branches. Ask before pushing.

## When each stage is done

cargo fmt / clippy / test and `pnpm typecheck` clean; PLAN.md (tick the items, IPC changes, Known limitations),
CASESTUDY.md (decision entries from D37, challenges, timeline) and CLAUDE.md (layout, commands, gotchas) updated in the
same change; a short manual check list for anything only I can verify. When all of 2.8 is signed off, the next step is
4a-5 (release v0.2, set aside for this phase): say so and update its kick-off prompt
(`prompts/phase-4a-release-readiness.md`) for what 2.8 changed.
