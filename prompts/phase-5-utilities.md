Start Phase 5 of Cay: utilities (timer / stopwatch / focus, a drop shelf, clipboard history). Research and a design
proposal come first; no code until the author has agreed to the design.

Read PLAN.md first: Current status, the Roadmap table, "Phase 5: utilities", the IPC section (Reserved for later
phases), Known limitations and Open questions. Then CASESTUDY.md: D1–D3 (Rust owns hover state, one window, never
activated), D4 (IPC names fixed up front), D14–D15 (activities, their queue and one watcher thread per source), D18
(UI work goes through the motion-designer agent), D30 (Cay's own look), D44 (one scroller for the panel's lists), D27
(risky actions: one approval each, your input comes first), D49/D50 (Phase 3: a feature that costs nothing while the
panel is closed, Rust deciding what may be acted on) and C6 (a screenshot "dropped" back onto the island). CLAUDE.md is
loaded; follow its rules and Windows gotchas, above all rule 1 (Rust owns state and every timer: React never runs
timers to hide things, and WebView2 throttles JS timers in the background), rule 2 (the island never takes focus),
rule 3 (IPC names in one place), rule 4 (Win32 isolated, `// SAFETY:` per `unsafe`) and rule 7 (feature folders).

Where things stand (2026-11-05): Phase 3 (memory-heavy apps) is complete: the panel's Memory section and End Task, built
on branch `self/phase-3-memory-heavy` (PR #2). Goose with Ollama (2.9) waits for the author's other machine. Release
4a-5 (v0.2) stays deferred to the end of all phases. The settings window is Phase 4b, so Phase 5's switches live in
`settings.json` until then (like `activities.*`, `agent.*` and `processes.*`).

Goal: three small, high-value tools that feel native to the island, each opt-in or harmless when unused, each costing
nothing while idle.

## Research first (report findings, with sources, before proposing)

- **Timer / stopwatch / focus.** Where the countdown lives (the pill, as an activity, or both) and how it ticks without
  a JS timer: Rust owns the deadline and the hover poller or a watcher thread drives it; React animates from a
  deadline it was sent (a CSS animation on an `@property`, as the charging ring does), never a `setInterval`. Presets in
  the panel (clicks only: the island takes no keyboard input; typing a custom time would need the command window).
  What "focus mode" should mean on Windows 11: Windows' own Focus sessions / Do Not Disturb (is there a public API:
  `Windows.UI.Shell.FocusSessionManager`, `WNF` states, Settings links?) versus a plain Cay timer. What happens on
  sleep, lock and across a Cay restart (persist the deadline?). The sound at the end (`activity/sound.rs` exists).
- **Drop shelf.** The island is click-through (`WS_EX_TRANSPARENT`) while collapsed, so it can't receive a drop until
  Rust knows a drag is in progress over the hot zone: how to detect an OLE drag from outside (mouse button held +
  cursor in the zone, `DragDetect`, a low-level mouse hook as `desktop/guard.rs` uses?) and switch click-through off in
  time. Whether WebView2's own drop target (C6) or a native `IDropTarget` on our HWND should receive files. Dragging
  files back out reuses `activity/drag.rs` (the screenshot drag-out: a sync command, main thread, started while the
  button is down). What the shelf holds (paths only, never copies? links that break when a file moves?), how many, and
  whether it survives a restart.
- **Clipboard history.** Opt-in and off by default. Windows already has Win+V history: can Cay read it
  (`Windows.ApplicationModel.DataTransfer.Clipboard.GetHistoryItemsAsync`, its consent and policy rules) instead of
  keeping its own? If its own: `AddClipboardFormatListener` on a message-only window (as `power.rs` does), and the
  formats password managers set to stay out of history (`ExcludeClipboardContentFromMonitorProcessing`,
  `CanIncludeInClipboardHistory`, `Clipboard Viewer Ignore`): honour every one. Memory only or on disk (and then
  encrypted with DPAPI?), size and count caps, images. Pasting back: put the item on the clipboard and let the user
  paste (no focus stealing, no synthetic Ctrl+V), or something better.
- For each: whether agent runs should see it (almost certainly not the clipboard).

## Then propose (and wait for the author's OK)

1. Stages (e.g. 5a timer, 5b drop shelf, 5c clipboard), each useful on its own, in an order that puts the least risky
   first.
2. Per stage: the feature folders (`src-tauri/src/<name>/`, `src/features/<name>/`), the IPC (names in `ipc.rs` /
   `ipc.ts` / `types.ts`), the switch in `settings.json`, the threads and what they cost while idle.
3. The look (through `motion-designer`): the pill's countdown, the panel's presets, the shelf, the history list; their
   empty states; minimal and native, existing tokens, no new colours unless agreed.
4. Privacy and safety: what is stored, where, for how long; what is never logged (clipboard text, file names in
   logs only as counts).
5. Tests: the pure parts (deadlines across sleep and DST, shelf bookkeeping, history caps and exclusion rules) unit-
   tested with synthetic input; manual checks per stage.

## Rules

- Stages each end with their own commit, doc updates and a manual check list. Don't start a stage before the previous
  one is signed off.
- UI changes go through the `motion-designer` agent with a precise brief; review its renders and run `pnpm typecheck`.
- New dependencies are a decision: record each in CASESTUDY (D51 onwards; challenges from C16) with why and the size
  cost. Prefer the `windows` crate's bindings (add a feature) over a new crate.
- Never read or log the author's real clipboard contents in tests: use content the test put there itself, and restore
  what was there.
- Don't edit Rust while the author drives `pnpm tauri dev` from the same checkout (it restarts Cay): work in a worktree
  beside it, as Phase 3 did. Watch the disk: `target\debug` has filled the drive several times (C14; CLAUDE.md's
  Commands); about 13 GB was free on 2026-11-05.
- Commits: small, imperative, prefixed `phase5:`; push to `main` unless the author asks for a branch. Ask before pushing.

## When each stage is done

cargo fmt / clippy / test and `pnpm typecheck` clean; PLAN.md (tick items, IPC changes, Known limitations, Current
status), CASESTUDY.md (decisions, challenges, timeline) and CLAUDE.md (layout, commands, gotchas) updated in the same
change. When Phase 5 is signed off, write the next kick-off prompt (the spikes: notifications mirror and live window
previews, per the roadmap) so a new session can pick up without this conversation.
