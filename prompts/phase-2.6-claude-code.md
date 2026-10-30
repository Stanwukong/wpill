Start Phase 2.6 (Claude Code activity) of TopBar.

Read PLAN.md first: Current status, Roadmap, "Phase 2.6: Claude Code activity", the IPC contract (Phase 2.5
table and the reserved 2.6 names) and Known limitations. Then read the CASESTUDY.md decision log, especially
D11 (watcher thread pattern), D14 (activities are a hover phase; the queue's feedback/notice classes), D15
(one watcher thread per source), D16 (drag-out from the non-activating island), D17 (why a named pipe and a
`topbar.exe hook` forwarder) and D18 (motion-designer agent). CLAUDE.md is loaded; follow its rules and
Windows gotchas. Phase 2.5 is signed off (2026-10-30).

Goal: the island tells me when a local Claude Code session finishes a task or needs a decision from me,
using Claude Code hooks, so I can work in other windows while it runs.

## What Phase 2.5 gives you (read the code before designing)

- `src-tauri/src/activity/mod.rs`: `Activities` managed state with a `Sender<Activity>` that every source
  uses, the `Watcher` harness (`Watcher::spawn(name, body, stop)`; `shutdown()` stops all watchers in parallel
  on `RunEvent::Exit`), the screenshot commands, and how `current` / `dismiss` are shared with the poller.
- `src-tauri/src/activity/queue.rs`: `rules()` (class, priority, duration, stale-after per kind), `key()`
  (same key = update in place, one id), `interactive()` (takes clicks while hovered; only screenshots today).
  Unit-tested; extend the tests with every rule you add.
- `src-tauri/src/hover/logic.rs`: `Machine` (the `Showing` phase) drives the queue; never shown over the panel.
- `src-tauri/src/ipc.rs`: the `Activity` enum (`#[serde(tag = "kind", content = "payload")]`). New kinds go
  here and in `src/lib/types.ts`. Reserved: `claudeDone`, `claudeNeedsInput`.
- `src-tauri/src/activity/settings.rs`: per-activity switches in `settings.json`; add one for Claude Code.
- `src/features/activities/`: `ActivityView` (one layout per kind), `icons.tsx`, `activities.css`.

## Scope: everything in PLAN.md's Phase 2.6 checklist

- **Hook forwarder.** `topbar.exe hook` reads the hook's JSON from stdin, writes it to a per-user named
  pipe, and exits. It checks its arguments in `main.rs` before `topbar_lib::run()` (so before Tauri, the
  single-instance plugin and WebView2), so it adds no noticeable delay to Claude Code. If TopBar isn't
  running it exits 0 silently. It never blocks Claude Code: short timeouts, and never a non-zero exit (that
  would surface as a hook error). The release build uses the windows subsystem, so check that a GUI-subsystem
  exe can read stdin from Claude Code and doesn't flash a console.
- **Pipe listener** (`src-tauri/src/claude/`, its own feature folder): a watcher thread started through the
  2.5 `Watcher` harness reads messages from the pipe and keeps session state (working / done / needs input)
  keyed by `session_id`. It sends activities on the `Activities` channel and emits a sessions event for the
  panel. The pipe's security descriptor allows only the current user. Messages are size-capped and parsed
  defensively, because any local process of mine could write to the pipe. Shutdown must unblock a pending
  `ConnectNamedPipe` (overlapped + stop event, like `screenshots.rs`).
- **Activities.** "<project>: done" on `Stop`. "<project>: needs your input" on `Notification` (permission
  prompt or waiting for input), at higher priority. The project name is the session's `cwd` folder name.
  Never show prompt or transcript text. Two queue questions to design, not hack around:
  - "Needs input" should stay until it's handled (the session's next event), but notices in `queue.rs` only
    time out. Propose how a sticky notice works (e.g. a long duration that a later event for the same key
    replaces or retracts) and how it behaves with the panel open, fullscreen, and multiple sessions.
  - Should clicking these activities focus the session's window (making them `interactive()`)? I'd like that,
    if it fits the hit-test rules (D14).
- **Panel section.** Open sessions with a status dot. Clicking one brings its terminal or VS Code window to
  the front, best effort: the forwarder records its ancestor process chain, and Rust picks the top-level
  window to activate through `apps/activate.rs`. With several VS Code windows open, fall back to matching the
  window title against the folder name.
- **Setup.** A tray item, "Connect Claude Code…", that shows exactly which entries it will add to
  `~/.claude/settings.json`, asks before writing, merges without clobbering existing hooks (back up the file
  first), and can remove them again. The island can't take keyboard input (rule 2), so the confirmation must be
  a native dialog or a separate window. It quotes the exe path (see the autostart gotcha). Also document the
  manual JSON in PLAN.md.
- **UI.** Every UI change (activity layouts, status dots, panel section, any animation) goes through the
  `motion-designer` agent (`.claude/agents/motion-designer.md`): delegate it, then review what it returns.
  Keep it minimal and consistent with the existing activities, and right on the plain and the tinted island.
  The look I want, in Claude's own visual language:
  - **Working: Claude Code's spinner.** The same orange glyph Claude Code shows in the terminal while it works,
    cycling through its frames (· ✢ ✳ ✶ ✻ ✽, then back), at Claude Code's pace. Find the exact frame
    characters, order, timing and color from the installed Claude Code (its CLI bundle) or its docs; it may
    use a different set on Windows terminals. It appears in the collapsed pill while any session is working
    (beside the clock, like the media equalizer) and next to each working session in the panel. Frame-step it
    in CSS (`steps()` keyframes on a pseudo-element's `content`, or a sprite), not a JS timer, and stop it
    when nothing is working. Use Unicode glyphs in a font that renders them on Windows (check Segoe UI Symbol /
    Cascadia), not Anthropic's logo.
  - **Done or needs input: the island's edges glow Claude orange.** Reuse the charging ring's technique
    (`.island::before` conic gradient masked to the border, `@property` angle, halo via `box-shadow`, D18) with
    its own token, e.g. `--claude: #d97757` (confirm the exact brand orange against Claude Code's terminal color).
    "Done" glows for the activity's duration; "needs input" keeps a calm, slow glow until the session's next
    event (the sticky notice above), with the spinner glyph still and solid in the activity. Reduced motion:
    static glyph and a static orange edge.

## Before coding

1. Check the current Claude Code hooks documentation for the exact event names and stdin fields (`Stop`,
   `Notification` and its type/message fields, `UserPromptSubmit`, `SessionStart`, `SessionEnd`,
   `SubagentStop`), the settings.json hook format, hook timeouts, and how exit codes and stdout are treated.
   Note anything that differs from PLAN.md. Also check whether the Claude Code desktop app and the VS Code
   extension run hooks.
2. Check the `windows` 0.62 crate sources in the cargo registry for named pipes (`CreateNamedPipeW`,
   overlapped `ConnectNamedPipe`, a current-user security descriptor) and for walking parent processes, with
   the feature names each needs (some are gated behind `Win32_Security`, as `CreateFileW` was).
3. Propose: the module layout on both sides, the pipe protocol (framing, size cap, version field), session
   lifecycle and expiry (e.g. no events for N hours, or a crashed terminal), the activity kinds, payloads,
   queue rules (class, priority, duration, stale-after, key) and the sticky "needs input" design, the new IPC
   names (to add to ipc.rs/ipc.ts and PLAN.md), how the window to focus is found, the settings.json merge
   strategy, and the UI (as a brief for the motion-designer agent).
4. Wait for my OK.

## When done

cargo fmt / clippy / test and pnpm typecheck clean. Unit-test the hook JSON parsing, the session state machine,
the new queue rules and the settings merge. Verify from outside the app by piping sample hook JSON into
`topbar.exe hook` and capturing the island (see CLAUDE.md "Testing activities from outside"), and by running a
real Claude Code session with the hooks installed. Don't modify my real `~/.claude/settings.json` without asking;
use a copy for tests. Give me a manual test list (task finishes while I'm in another app, a permission prompt,
two sessions in different folders, a session in VS Code and one in Windows Terminal, clicking a session to focus
it, the panel open when a session finishes, TopBar not running, hooks removed again). Update PLAN.md,
CASESTUDY.md and CLAUDE.md, write the next phase's kick-off prompt once 2.6 is signed off, then commit and push
to main.
