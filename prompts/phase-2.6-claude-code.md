Start Phase 2.6 (Claude Code activity) of TopBar.

Read PLAN.md first: Current status, Roadmap, "Phase 2.6: Claude Code activity", the IPC contract and
Known limitations. Then read the CASESTUDY.md decision log, especially D11 (watcher thread pattern), D17
(why a named pipe and a `topbar.exe hook` forwarder) and whatever Phase 2.5 recorded about the activity
queue. CLAUDE.md is loaded; follow its rules. Phase 2.5 must be signed off first; if it isn't, stop and tell me.

Goal: the island tells me when a local Claude Code session finishes a task or needs a decision from me,
using Claude Code hooks, so I can work in other windows while it runs.

Scope: everything in PLAN.md's Phase 2.6 checklist.
- **Hook forwarder.** `topbar.exe hook` reads the hook's JSON from stdin, writes it to a per-user named
  pipe, and exits. It checks its arguments in `main.rs` before Tauri, the single-instance plugin or
  WebView2 start, so it adds no noticeable delay to Claude Code. If TopBar isn't running it exits 0
  silently. It never blocks Claude Code: short timeouts, and never a non-zero exit (that would surface
  as a hook error).
- **Pipe listener** in Rust (`src-tauri/src/claude/`): a dedicated thread reads messages from the pipe
  and maintains session state (working / done / needs input) keyed by `session_id`. It pushes activities
  into the Phase 2.5 queue and emits a sessions event for the panel. The pipe's security descriptor allows
  only the current user. Messages are size-capped and parsed defensively, because any local process of
  mine could write to the pipe.
- **Activities.** "<project>: done" on `Stop`. "<project>: needs your input" on `Notification` (a
  permission prompt or waiting for input), with higher priority, staying up longer or until handled.
  The project name is the session's `cwd` folder name. Don't show prompt or transcript text.
- **Panel section.** Open sessions with a status dot. Clicking one brings its terminal or VS Code
  window to the front, best effort: the forwarder records its ancestor process chain, and Rust picks the
  top-level window to activate through `apps/activate.rs`. With several VS Code windows open, fall back
  to matching the window title against the folder name.
- **Setup.** A tray item, "Connect Claude Code…", that shows exactly which entries it will add to
  `~/.claude/settings.json`, asks before writing, merges without clobbering existing hooks (back up the file
  first), and can remove them again. It quotes the exe path (see the autostart gotcha). Also document the
  manual JSON in PLAN.md.

Before coding:
1. Check the current Claude Code hooks documentation for the exact event names and stdin fields
   (`Stop`, `Notification` and its type/message fields, `UserPromptSubmit`, `SessionStart`,
   `SessionEnd`, `SubagentStop`), the settings.json hook format, hook timeouts, and how exit codes and
   stdout are treated. Note anything that differs from PLAN.md. Also check whether the Claude Code
   desktop app and the VS Code extension run hooks.
2. Check the `windows` 0.62 APIs and features for named pipes (`CreateNamedPipeW`, overlapped or
   blocking `ConnectNamedPipe`, a security descriptor for the current user) and for walking parent processes.
3. Propose: the module layout on both sides, the pipe protocol (framing, size cap, version field),
   session lifecycle and expiry (e.g. a session with no events for N hours, or a crashed terminal), the
   activity kinds/payloads and priorities within the 2.5 queue, the new IPC names (to add to ipc.rs/ipc.ts
   and PLAN.md), how the window to focus is found, and the settings.json merge strategy.
4. Wait for my OK.

When done: cargo fmt / clippy / test and pnpm typecheck clean (unit-test the hook JSON parsing, the
session state machine and the settings merge). Verify from outside the app by piping sample hook JSON
into `topbar.exe hook` and screenshotting the island, and by running a real Claude Code session with the
hooks installed. Don't modify my real `~/.claude/settings.json` without asking; use a copy for tests.
Give me a manual test list (task finishes while I'm in another app, a permission prompt, two sessions
in different folders, a session in VS Code and one in Windows Terminal, clicking a session to focus it,
TopBar not running, hooks removed again). Update PLAN.md, CASESTUDY.md and CLAUDE.md, then commit and
push to main.
