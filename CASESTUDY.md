# Case Study: Cay, a Dynamic Island for Windows 11

> Cay was called **TopBar** until Phase 4a-3 (D32, 2026-11-01). Entries written before then use the old name, paths
> and identifiers (`topbar.exe`, `dev.wpill.topbar`) as they were at the time.

> A running record of how this project was built: the problem, the decisions, the hard
> parts and the results. Update it after every significant change (see [CLAUDE.md](CLAUDE.md)).
> Write entries so they still make sense to someone reading months later.

## 1. The problem

Windows 11 has no lightweight, glanceable surface for "what's happening right now":
running apps, media playback, memory pressure. The taskbar is busy and Alt+Tab is modal.
macOS and iOS solved a similar problem with a top-center surface that stays out of the way until
you reach for it.

**Goal:** a top-center pill that expands on hover into a useful panel, never steals focus,
never blocks clicks while idle, and gets out of the way for fullscreen apps.

## 2. Constraints

- Windows 11 only, so deep Win32/WinRT integration is fine.
- Must feel native: 60 Hz hover tracking, smooth spring animation, no focus theft, no taskbar entry.
- Small footprint, which ruled out Electron in favor of Tauri 2 (system WebView2 + Rust).

## 3. Stack

| Layer | Choice | Why |
|---|---|---|
| Shell | Tauri 2 | Small binaries, Rust backend with direct Win32 access, WebView2 already on Win11 |
| UI | React + TypeScript + Vite | Fast iteration, typed IPC |
| Motion | Framer Motion | Layout animations and springs give the pill-to-panel morph almost for free |
| System APIs | `windows` crate, `sysinfo` | Official Microsoft bindings; cross-process memory stats |

## 4. Decision log

Newest last. Format: **Decision**, then *Why*, then *Alternatives considered*.

### D1: Rust owns hover state, not the DOM (Phase 0)
*Why:* While collapsed, the window must be click-through (`set_ignore_cursor_events(true)`),
so the webview receives no mouse events and can't detect the hover itself. A Rust thread
polling `GetCursorPos` at ~60 Hz sees the cursor whatever the window state is, and can
apply the 150 ms open delay and leave debounce in one place.
*Alternatives:* a low-level mouse hook (`WH_MOUSE_LL`) is event-driven but adds global-hook
latency risk and AV/EDR suspicion. A permanently clickable thin strip window means two windows to keep in sync.

### D2: One window sized to the largest panel (Phase 0)
*Why:* Resizing a transparent WebView2 window on every expand causes flicker and fights
the animation. A fixed-size window lets Framer Motion animate purely in CSS.
*Cost:* transparent margins would swallow clicks while expanded. *Mitigation:* accept cursor
events only while the pointer is inside the panel rect that React reports.

### D3: `WS_EX_NOACTIVATE` + `WS_EX_TOOLWINDOW` (Phase 0)
*Why:* the island should behave like a shell surface: no focus steal, no taskbar or Alt+Tab
entry. *Trade-off:* no keyboard input in the island, so settings will get a separate window.

### D4: Fixed IPC names up front for all phases (Phase 0)
*Why:* reserving `media:*`, `list_heavy_processes` and the others now keeps later phases additive.
Each phase adds a feature folder on both sides and never rewires the core.

### D5: The hot zone is a thin strip, not the whole pill (Phase 1)
*Why:* the pill sits over the top-center of whatever app is maximized underneath, which is often
browser tabs. If the whole pill were a hover target, reaching for a tab would pop the panel
open. A 200×10 px strip on the top edge is easy to hit deliberately (the screen edge stops the
cursor) and hard to hit by accident. The 150 ms dwell filters out the rest.
*Alternatives:* the full pill rect (too many accidental opens). A click-to-open pill was rejected
because it breaks click-through while collapsed.

### D6: An empty synthetic mouse event instead of the Alt-key trick (Phase 1)
*Why:* both satisfy the "process that received the last input" foreground rule, but a
synthetic Alt can activate the target app's menu bar. A zero-movement `INPUT_MOUSE` has no
visible side effect. `AttachThreadInput` stays as the last resort.

### D7: React reports geometry in physical pixels (Phase 1)
*Why:* see challenge C1. Only the webview knows its real CSS-to-physical ratio, so the conversion happens
there, once. Rust compares physical cursor coordinates against physical rects and never has to
know about text-size zoom.
*Alternatives:* reading `TextScaleFactor` from the registry in Rust, which duplicates WebView2's logic and is fragile.

### D8: Self-healing visibility instead of fixing startup ordering (Phase 1)
*Why:* see challenge C2. The poller already runs every 250 ms and already owns "should the island
be visible?" (fullscreen). Making it enforce that invariant also covers future cases like
Explorer restarts, at no extra cost.

### D9: Reprioritized roadmap after Phase 1 (2026-10-28)
*Why:* Phase 1 proved the core, so the order now follows what makes the product feel like a
Dynamic Island and what blocks sharing it:
- Tray icon and autostart moved up from Phase 4 to a tiny Phase 1.5: testers can't quit the app otherwise.
- The media card (Phase 2) now also drives the *collapsed* pill. A widget that's only visible on hover undersells the concept.
- A new Phase 2.5, Live Activities, makes the island expand on its own for volume, Bluetooth,
  battery, Caps Lock and screenshots. This is the signature behavior, and it's cheap once Phase 2's pill plumbing exists.
- Multi-monitor/DPI and code signing (Phase 4a) come before any public release.
- Memory-heavy apps (Phase 3) moved behind a settings toggle: niche, and it has the riskiest action.
- New ideas (timer, drop shelf, clipboard) became Phase 5. Notifications, calendar and live previews became
  spikes, because their feasibility or sign-in requirements are unproven.

*Alternatives:* keeping the original 2 → 3 → 4 order. It would have shipped a niche process
killer before the features people notice, and left the app unquittable for testers.

### D10: The registry is the source of truth for "Launch at startup" (Phase 1.5)
*Why:* the setting lives as a per-user `Run` registry value, and Task Manager keeps its own
enable/disable switch under `StartupApproved\Run`. Keeping a third copy in `tauri-plugin-store` would let
them drift apart. `autostart::is_enabled` reads both, and the tray checkbox re-reads them after every
toggle, so it always shows the real state, even if a write fails or the user changed it in Task Manager.
*Alternatives:* a settings store, deferred until Phase 4b needs real settings.
*Revised (C5):* the Run value is written by our own `autostart.rs`, not `tauri-plugin-autostart`.
Single instance came along in the same phase, because autostart plus a manual launch would otherwise stack two islands.

### D11: One thread owns every media subscription; handlers only forward (Phase 2)
*Why:* GSMTC raises its events on arbitrary thread-pool threads, and a single track change fires all three
session events at once. Having handlers touch shared state would need locking around WinRT calls and would
re-read the same session three times. Instead, each handler sends a small message over a channel to a
dedicated `media-watcher` thread (MTA) that owns the manager, the sessions and the artwork cache. It batches
50 ms of messages and re-reads only what changed. Every registration token is held in a `Subscription`
guard whose `Drop` calls the matching `Remove…`, so dropping a vanished session's entry (or the whole
watcher, on `RunEvent::Exit`) unregisters everything without bookkeeping. Sessions are matched across
`GetSessions()` calls by COM identity (`IUnknown` pointer), because two browser sessions share an app id.
Commands don't go through the thread: GSMTC objects are agile, so the watcher publishes session handles to
managed state and async commands `.await` the `Try…Async` call directly.
*Alternatives:* polling every session on a timer (simple, but wakes the CPU while idle and lags behind
track changes); routing commands through the watcher thread (one owner, but a slow app would block every update).

### D12: Emit only what React can't work out itself; pass album art through (Phase 2)
*Why:* the payload carries album art as a data URL (tens to hundreds of kB), and some apps update their
timeline every second. Re-sending the whole state each time would push megabytes per minute through IPC.
The payload instead carries `positionMs` plus the time it was measured (`updatedAtMs`), React interpolates,
and Rust re-emits a timeline change only if it disagrees with that interpolation by more than 1.5 s (a seek,
a stall). Any other field changing always emits. Steady Spotify playback produced no emits between track
changes. Album art is passed through as the bytes the app supplied (JPEG/PNG, type sniffed from magic bytes)
rather than decoded and re-encoded: apps already send reasonably sized images, and re-encoding would
cost a `Graphics_Imaging` round trip per track for no visible gain.
The reserved single-session `media:state` shape became `{ sessions: [...] }` (most recent first) so the
switcher needs no extra event. The selection lives in React: a manual pick lasts until another session
becomes most recent.
*Alternatives:* a separate artwork command keyed by track (smaller events, but an extra round trip and
cache on both sides); downscaling art with `BitmapDecoder` (only worth it if large thumbnails become common).

### D13: The island takes its colors from the album art (Phase 2 polish)
*Why:* a black island with a colorful thumbnail in it looks pasted on; tinting the whole surface makes the
media state readable at a glance, like the hardware it imitates. The webview already has the art as a data URL,
so `artTheme.ts` draws it to a 24×24 canvas and picks the dominant *colorful* hue (pixels bucketed by hue, weighted
by saturation and mid-lightness, so a small vivid area beats a large grey one; mostly-grey art falls back to its
average). From that hue it derives a dark gradient (lightness 24 % → 9 %, so the white text keeps its contrast) and a
lighter accent for the equalizer and playing dot. The colors are `@property`-registered CSS custom properties, which
is what lets them transition (700 ms) instead of snapping. Results are cached per artwork. The tint applies whenever
the shown session has art, paused included, because the panel still shows that art.
*Alternatives:* extracting in Rust (would need JPEG decoding and a payload change for something purely visual);
blurring the art itself behind the island (heavier, and busy art hurts text legibility).

### D14: An activity is a hover phase, and the queue lives on the poller thread (Phase 2.5)
*Why:* an activity changes the island's shape, its hit area and whether it takes clicks: exactly what the hover state
machine already owns (D1). Making it a React-side expand would have split that ownership and reintroduced the
"webview gets no mouse events while click-through" problem. So `hover/logic.rs` gained a `Showing` phase, and a
`Machine` combines the phase with the activity queue: dwelling in the hot zone during an activity still opens the panel,
and nothing is ever shown over an open (or leaving) panel. The queue has two classes. *Feedback* (volume, brightness,
Caps Lock) answers a key just pressed: it shows at once, replaces what's showing, and is dropped if it can't show,
because feedback after the fact is noise. *Notices* (Bluetooth, power, screenshot) wait while the panel is open, resume
with at least 1.5 s if feedback interrupted them, and go stale after 10–60 s. Activities with the same key update in
place with the same id, so a burst of volume presses is one activity with a moving bar. Sources send finished
activities down one channel that the poller drains each 16 ms tick: no locks around the queue, no extra wakeups, and
all timing in one place. Only the screenshot is *interactive*: while the cursor is on it the island takes clicks and the
countdown pauses. Other activities stay click-through under the cursor, so a volume bar never eats a click meant for a
browser tab. `set_panel_rect` gained a `mode`, so a late report for the previous shape can't become the hit area (a
generalization of C3).
*Alternatives:* a separate activity window (two windows to keep in sync, D2 again); timing durations in React
(throttled when idle, and Rust needs to know anyway to restore click-through); one priority list without classes
(either volume feedback waits behind a screenshot, or notices get dropped like feedback).

### D15: One watcher thread per source; the cheapest reliable API for each (Phase 2.5)
*Why:* each source follows the D11 pattern (its thread owns the subscriptions, callbacks only forward, guards
unregister on drop and on `RunEvent::Exit`), but the APIs differ:
- **Volume:** `IAudioEndpointVolumeCallback` on the default output, re-registered when `IMMNotificationClient` reports a
  new default. Both are COM interfaces implemented in Rust with `#[implement]`. The current level at subscribe time is a
  baseline, not an activity.
- **Bluetooth audio:** endpoint state changes from the same `IMMNotificationClient` rather than a Bluetooth device
  watcher. It only fires for audio devices (the scope), and a headset's stereo and hands-free endpoints share a device
  container, so connects are deduplicated by container id. A `DeviceInformation` query on the container tells
  Bluetooth from wired (a `BTHENUM\`/`BTHLE\` node) and reads the battery from the undocumented
  `{104EA319-6EE2-4701-BD47-8DDBF425BBE5} 2` property, which is what Settings shows.
- **Brightness:** WMI's `WmiMonitorBrightnessEvent`, pulled semi-synchronously (`IEnumWbemClassObject::Next` with a
  500 ms timeout). No sink object to implement, nothing calls into the process from WMI threads, and the timeout doubles as
  the stop check. `GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS` power notifications were rejected: not on the documented list.
- **Power:** `RegisterPowerSettingNotification` for AC/DC and battery percentage. It needs a window, so the thread owns a
  message-only window (never the island's HWND).
- **Caps Lock:** polled with `GetKeyState` in the existing poller. Verified with SendInput that a long-lived thread with
  no message queue sees the toggle, so no keyboard hook is needed (D1).
- **Screenshots:** an overlapped `ReadDirectoryChangesW` on the Screenshots known folder, waited on together with a stop
  event; the thumbnail comes from the shell (`IShellItemImageFactory`), so any image type works and Explorer's cache helps.

*Alternatives:* a low-level keyboard hook for volume, brightness and Caps Lock (sees key presses but not the resulting
level, misses changes made by other apps, and is the hook D1 avoided); polling `GetSystemPowerStatus` (simple, but
wakes the CPU for something Windows will push); clipboard monitoring for screenshots (an image with no file can't be
dragged out as a file).

### D16: Drag-out with the shell's own drag loop, from the non-activating window (Phase 2.5)
*Why:* a screenshot is most useful dropped straight into a chat or folder. HTML5 drag can't carry a real file out of
WebView2, so the webview only detects the gesture (press, then 4 px of movement) and calls a *sync* command. Tauri runs
those on the main thread, which Tao has OLE-initialized and which owns the island window. The command runs
`SHDoDragDrop` with the file's shell-item data object and no drop source of its own (the shell supplies the default one
and the drag image), while the button is still down. `WS_EX_NOACTIVATE` doesn't matter: drop targets never check whether
the source is active. Verified with a synthetic drag: the shell's drag image and "+ Copy" cursor appeared and the
foreground window never changed. While the drag runs, the poller holds the activity. A successful drop elsewhere ends
it; a click (no movement) opens the file instead.
*Alternatives:* click-to-open plus a "copy file" button putting `CF_HDROP` on the clipboard (the fallback planned in
case the drag failed); `tauri-plugin-drag` (same mechanism, an extra dependency for ~20 lines).

### D17: Claude Code activity through hooks, a named pipe and a self-forwarding exe (Phase 2.6)
*Why:* Claude Code already runs hooks at the moments that matter (`Stop` when a turn finishes,
`Notification` when it needs a decision) and hands each hook a JSON description on stdin. The cheapest
reliable bridge is to make `topbar.exe` its own hook command: `topbar.exe hook` reads stdin, writes it to a
per-user named pipe, and exits before Tauri or WebView2 start, so Claude Code isn't slowed down. A pipe
needs no network port, can be locked to the current user, and simply isn't there when TopBar isn't running
(the forwarder then exits quietly, so the hook never fails). It's scheduled after 2.5 because it's just
another activity source for 2.5's queue.
*Alternatives:* the single-instance plugin's argument forwarding (no stdin, so the JSON would have to go on
the command line); a localhost HTTP server (opens a port any local process can find); a PowerShell or `curl`
hook script (slow to start, and an extra file to install); watching transcript files (fragile, and reads
far more than needed).

### D18: Charging gets an orbiting green outline; UI work goes through a motion-designer agent (Phase 2.5 polish)
*Why:* plugging in is the one activity that's about something *happening* (energy flowing in), so it earns motion
where the others don't. The island's outline glows green and a comet of light orbits it once every 2.4 s, over a
soft green halo. It's a conic gradient on `.island::before`, masked to a 2 px ring with `border-radius: inherit`, so
it follows the island through the Framer morph without extra DOM. The angle is a registered `@property` animated
by CSS (no JS timers), and the animation is paused unless `data-charging` is set. Green is its own token
(`--charge`), not `--accent`, because charging should read the same whatever album art is playing. Reduced
motion gets a static ring. Visuals that can't be triggered from outside are checked by rendering the built CSS in
headless Edge. To keep later UI changes consistent, the motion and visual rules (and that verification method)
now live in a project agent, `.claude/agents/motion-designer.md`, which CLAUDE.md routes all UI changes through.
*Alternatives:* a rotating element behind the island (needs a wrapper outside `overflow: hidden` and fights the
layout animation); a pulsing glow (reads as an alert, not progress); keeping the rules only in CLAUDE.md (they'd be
mixed into backend guidance and easy to skip).

### D19: "Needs your input" is a sticky notice that steps aside (Phase 2.6)
*Why:* a permission prompt stays unanswered until you look, so a notice that times out after a few seconds is easy to
miss, and one that stays up forever would starve every other activity. The queue gained a third behaviour next to
feedback and notices: a *sticky* notice has no end time and never goes stale, and leaves only when its source withdraws
it (`Signal::Retract(key)` on the activity channel, sent on the session's next event) or the user acknowledges it by
clicking it. It steps aside for any other notice and comes back after it, and waiting notices go before it, so a
screenshot or Bluetooth notice never goes stale behind a question. Opening the panel or a fullscreen app *parks* it
(back to the front of the line) instead of dropping it, which is the one place a sticky notice differs from the D14
rules. Sessions share one key each, so a `Stop` turns that session's "needs input" into "done" in place, and several
sessions take turns instead of stacking. `durationMs` became `null` for sticky notices; React never read it anyway.
Both Claude activities are *interactive* like the screenshot: clickable only while the cursor is on them (D14), so a
question sitting at the top of the screen never eats a click meant for a browser tab beneath it.
*Alternatives:* a long fixed duration (still times out on a slow coffee break, and still blocks other notices while
up); one combined "2 sessions need you" activity (needs a count payload and loses which project; deferred).

### D20: Shell-form hook entries, async, and only what's needed crosses the pipe (Phase 2.6)
*Why:* Claude Code's newer exec form (`"command": exe, "args": ["hook"]`) avoids shells entirely, but 2.1.123, still
the first `claude` on PATH here, doesn't know `args`, drops it, and would run a bare `topbar.exe`: a second TopBar
instead of the forwarder. So the entry is shell form with an explicit `"shell"` (Git Bash when Claude Code would find
it, else PowerShell) and a single-quoted path, which survives spaces in both shells. Every hook except `SessionEnd`
is `async: true`, so Claude Code never waits for TopBar. The price is ordering: background hooks can land out of order,
so the forwarder stamps the time it started and a session ignores anything older than what it already applied.
`PostToolUse` is subscribed only because it's the one event that says a permission prompt was answered; `idle_prompt`
corrects the status after an Esc (where `Stop` never fires) without showing anything. The forwarder parses stdin itself
and forwards five fields plus its ancestor chain, so prompt, tool output and Claude's answers never reach TopBar. The
chain (one ToolHelp snapshot, creation times checked against pid reuse) finds both the window to focus (nearest
ancestor owning an Alt+Tab window; title match on the folder for several VS Code windows) and the `claude.exe` whose
exit ends the session.
*Alternatives:* exec form (breaks older Claude Code, see above); synchronous hooks (adds ~100 ms to every tool call);
forwarding the raw stdin (simpler, but sends whole answers through the pipe for nothing); `GetConsoleWindow` in the
forwarder (it has no console: Claude Code starts hooks windowless).

### D21: A local automation agent built on headless Claude Code, approved from the island (planned, Phase 2.7)
*Why:* after 2.6 the island already knows what Claude Code sessions are doing; the next step is to start them. Claude
Code already plans, runs commands, edits files and keeps its own permission rules, so TopBar shouldn't become an agent
framework. It launches `claude -p` with stream-json in and out, shows progress through the existing activity queue,
and answers Claude Code's permission prompts through `--permission-prompt-tool`, pointed at a tool on an MCP server
that TopBar provides. The same MCP server then gives Claude the desktop tools Claude Code lacks on Windows (window
lists, screenshots, UI Automation, and later input), built from code `apps/` already has. One server for both keeps
one trust boundary: a per-user pipe like D17's, reached through a `topbar.exe mcp` stdio bridge that runs before
Tauri, so no network port is opened. Approvals are D19 sticky notices with Allow / Deny, so the island stays
click-only (rule 2) and typing moves to a separate focusable command window. Staged (a: runs and status, b:
approvals, c: read-only desktop tools, d: input actions, e: routines) so the risky part, synthetic input into other
apps, comes last and only after the rest has been lived with. The flags were checked in all three Claude Code
versions installed here (2.1.123, 2.1.159, 2.1.312).
*Alternatives:* the Claude Agent SDK in a Node or Python sidecar (richer API, `canUseTool` callbacks, but a second
runtime to ship and keep alive next to a 1.5 MB Rust app); a synchronous `PermissionRequest` hook for approvals (works
for interactive sessions too, but blocks Claude Code inside a hook with a timeout, and mixes "watch my sessions" with
"drive my sessions"); an HTTP MCP server on localhost (simpler transport, but opens a port any local process can find,
which D17 already rejected); screenshot-and-click as the only way to act (slow and brittle; the guidance tells Claude to
prefer the shell and UI Automation, and to use raw clicks last); `bypassPermissions` with an allow list (no way to see
or stop a bad step before it happens).

### D22: An agent run is `claude -p` in a Job Object, one process per stretch of work (Phase 2.7a)
*Why:* research at kick-off (2.1.123, 2.1.159, 2.1.312, plus small haiku runs) settled the shape. With stream-json in
and out, `system/init` opens every turn with the session id, `result` closes it (`num_turns` for that turn,
`total_cost_usd` for the session, `subtype` `error_max_turns` / `error_max_budget_usd` for the limits), and a user
message written while a turn runs is queued by Claude Code. So TopBar keeps stdin open while there is unanswered work
and closes it when every message has its `result`: the process exits, and a later follow-up resumes the same session
with `--resume` in a new process. There's no idle `claude.exe` per finished task and no "is it still listening" state.
`claude.exe` starts suspended with `CREATE_NO_WINDOW`, goes into a Job Object with `KILL_ON_JOB_CLOSE`, then resumes,
so nothing it starts can escape before it's assigned. Stop, the stop hotkey, quitting TopBar and TopBar dying all end
the whole tree (checked with `ping -t localhost` under three shells). The session's own transcript is already on disk,
so a stopped run still resumes. Runs started by TopBar fire the user's hooks too; 2.6's listener recognises them
because TopBar's own pid (and creation time) is in the hook's ancestor chain, and leaves them to `agent/`. The
`claude` on PATH is often an npm shim that can't be spawned without a shell, so TopBar finds real `claude.exe` files
(PATH, npm, `~/.local/bin`, VS Code's extension) and uses the newest version unless `agent.claudePath` says otherwise.
*Alternatives:* one long-lived process per run waiting on stdin for follow-ups (an idle process per task, and an
extra state to explain); `--permission-mode` from the user's settings (TopBar always passes `default`, which all
three versions accept; 2.1.312 lists it as `manual`); SIGTERM/SIGINT semantics from the docs (POSIX only; on Windows
Stop is TerminateJobObject, and the transcript is already saved); `AskUserQuestion` (nobody can type an answer mid-run,
so it's disallowed and the guidance says to ask in the final message instead).

### D23: Approvals: a token-checked pipe, Rust-built summaries, and narrow "allow for this run" (Phase 2.7b)
*Why:* `--permission-prompt-tool mcp__topbar__approve` calls TopBar's MCP tool with `{ tool_name, input,
tool_use_id }` and expects a text block holding `{"behavior":"allow","updatedInput":…}` or `{"behavior":"deny",
"message":…}` (verified by hand; the docs don't give the schema). Claude Code starts `topbar.exe mcp` from a per-run
`mcp.json` whose `env` carries the run id and a 128-bit random token; the bridge relays JSON-RPC over a second per-user
pipe (`topbar-agent-<SID>`, same DACL as D17), and its first message must name a live run with that token, so a stray
local process can't attach to the approval channel. TopBar answers MCP itself; the bridge only relays, caps messages at
4 MiB and answers oversized ones with an error so Claude Code never waits forever. Each request becomes a D19 sticky
notice keyed per request, so several runs' approvals take turns and park under the panel and fullscreen; the panel
also lists them with Allow / Deny. Unanswered for 10 minutes means deny, with a message telling Claude nobody
answered. Risk classes: reads (Read, Grep, WebFetch…) are `read`; everything else (shell, edits, unknown and other
MCP tools) is `change`; 2.7d's desktop actions will be `input`. "Allow for this run" is remembered per program for
shell commands (`Bash:git`), as one group for edits, per host for fetches, and never for `input` or for a shell
command containing an operator (`&`, `|`, `;`, backtick, `$(`, redirects, newlines), since remembering `cd` would also
allow `cd x && rm -rf y`. Defaults live in `settings.json` (`agent` block): `--strict-mcp-config` on (only TopBar's
server; the user's own servers can be turned back on), the user's settings kept (their allow rules, hooks and plugins
still apply, so steps they already allow aren't asked about), 25 turns and $1 per run, Win+Alt+A / Win+Alt+Escape.
*Alternatives:* trusting any pipe client (a local process could answer approvals); remembering per tool (allowing one
`git status` would allow every command); `--setting-sources project,local` to send everything to the island (drops the
user's hooks and plugins too; available as `agent.userSettings: false`); a short timeout (too short for a coffee
break, while the run holds nothing but a pending tool call).

### D24: Claude content in Cascadia Mono, a quieter status, and a separate command window (Phase 2.7a)
*Why:* after 2.6 the status ("Done", "Needs your input") competed with the project name. The status is now 11 px at
48 % white on the project's baseline, and Claude content (activities, panel rows, approvals, the command window's
transcript) uses `--font-claude`: Cascadia Mono, which ships with Windows 11 and echoes the terminal Claude Code runs
in. Candidates were rendered side by side (Cascadia Mono, Sitka Text, IBM Plex Sans); Plex and Sitka read as "slightly
different Segoe" at 14 px. Typing happens in a separate, normal `command` window (rule 2): frameless, created on first
use, hidden on Esc, opened by a global hotkey registered on its own thread without any window (`WM_HOTKEY` goes to the
thread's queue, and receiving it lets TopBar take the foreground). It's sized by Windows' Text size factor so the page
keeps the same CSS room at 120 % and 150 %.
*Alternatives:* typing in the island (breaks rule 2 and steals focus); a message-only window for the hotkey (not needed
without a window target); Anthropic's brand fonts (not licensed for this); bundling an OFL font (no visible gain).

### D25: Desktop looks: four read-only tools, approved twice, cheapest view first (Phase 2.7c)
*Why:* Claude in a TopBar run can now look at the desktop through four tools on TopBar's own MCP server (D21):
`list_windows`, `focused_window`, `screenshot`, `ui_tree`. They live in a `desktop/` feature folder so the Win32 stays
isolated and 2.7d can reuse it. A window is named by its HWND from `list_windows`, checked again on every call
(alive, not TopBar's, not cloaked, responding, not elevated, screen not locked), and its process id from approval
time must still match, so a reused HWND can't redirect an approved look. Windows are captured with
`PrintWindow(PW_RENDERFULLCONTENT)` (works when covered, no capture border). The screen is a `BitBlt` of the primary
monitor with TopBar's windows marked `WDA_EXCLUDEFROMCAPTURE` only for that moment, so your own screenshots and screen
shares still see the island. Images go through WIC (part of Windows: no new crate): at most 1568 px on the long edge
(what the API keeps anyway) and 500 KB (under Claude Code's own 512,000-byte re-compression), PNG first, JPEG when a
photo or video makes PNG too big. `ui_tree` is a capped text outline (depth, nodes, 60 KB, a 3 s budget) rather
than JSON: fewer tokens, and the guidance tells Claude to use it before a screenshot. Password fields never carry a
value. Approvals are risk `read`, remembered per kind ("Allow screenshots for this run"), with summaries naming the
window. Because a user's allow rule would let a call skip Claude Code's prompt, allowing the prompt leaves a one-time
grant for that exact input, and a call that arrives without one is asked about by TopBar itself: every look is
approved in the island whatever Claude Code or the user's settings decide. The audit records what was looked at and
how big the result was, never the pixels or the tree.
*Alternatives:* Windows.Graphics.Capture (D3D11 setup, and a yellow border on Windows 11 unless access is requested);
`WDA_EXCLUDEFROMCAPTURE` permanently on the island (it would vanish from your own screenshots and calls); blacking out
the island's rect (hides what's under it); the `image` crate (size, when WIC is already there); `readOnlyHint` on the
tools (Claude Code reads it as "read-only", which a future mode might not ask about); `--allowedTools` per run
(approves looks before anything happens); JSON trees (2–3× the tokens for the same content).

### D26: A glow on the screen's edges while Claude looks (Phase 2.7c)
*Why:* the approval says Claude *may* look; the glow says it *is* looking, right now, in peripheral vision. It's a
native layered window, not a webview: a full-screen transparent WebView2 would cost tens of MB and bring back the
surface-drop blink of C4. It's painted once per monitor with `UpdateLayeredWindow` (premultiplied BGRA, Claude's
accent, a 32 px band falling off as `0.6·(1−t)²`, corners combined as light) and faded by stepping its constant alpha
on a timer that only runs during fades. It's click-through, never activated, kept just below the island, and always
excluded from capture, so it's never in a screenshot (Claude's or yours). It holds 1.5 s after the last look so a
50 ms window list still registers, never pulses (a pulse reads as "recording"), and skips fades when Windows'
animation effects are off. The spec came from the motion-designer agent.
*Alternatives:* only the island's approval (gone by the time the look runs); a pulsing or red border (alarming);
four thin edge windows (less overdraw, four times the window management for one effect).

### D27: Desktop actions: UI Automation first, one approval each, and your hands come first (Phase 2.7d)
*Why:* in a TopBar run Claude can now act on the desktop through seven more tools on TopBar's MCP server:
`focus_window`, `invoke`, `set_value`, `click`, `type_text`, `press_keys`, `launch`. They're ordered by how little can
go wrong. `invoke` (Invoke, Toggle, SelectionItem, ExpandCollapse, ScrollItem, focus, and LegacyIAccessible's default
action) and `set_value` (Value pattern) go through UI Automation: no cursor, no keyboard focus, and the window can
stay behind. `click`, `type_text` and `press_keys` use `SendInput` only when UIA can't do it. `launch` opens
https/http/`ms-settings:` links and Start-menu apps by name (`shell:AppsFolder\<AUMID>`), never paths or arguments,
which stay with the shell tool and its approvals. Claude names an element by the `[n]` ref its latest `ui_tree` of
that window printed. TopBar keeps each run's last tree per window (runtime id, type, name, automation id, bounds) and
finds the element again at act time: by runtime id under its old centre, then by `FindAll` on type + automation id
(or name). The match must agree on all three, or the reply is `elementGone` ("call ui_tree again"). It never acts
on a guess. A click can also be given in image px of the run's last screenshot of the window, refused as
`staleView` if the window has moved since. Before every `SendInput` batch TopBar checks that the target is still in
front, the screen is unlocked, and the point is on the target (`WindowFromPoint`). Text is typed as real key presses
from the target's keyboard layout, one paced event at a time (C11). Keys that went down are always released.
*Approvals:* every action is risk `input`. It's asked about once per call and never remembered for the run,
with a summary that says exactly what will happen: "Press "Save" in Notepad · notes.txt", "Type "hello" into …",
"Press Ctrl+A, then Delete in …", "Open https://github.com in your browser". The one-time grant check from D25
covers a user allow rule. Typed and set text shows short in the island and in full in the command window's card.
The audit keeps only its size (`{chars, lines}`). Batches were left out: what step two does depends on what step one
did, so approving both at once is approving blind. `press_keys` takes up to five chords for the common short
sequences. Win-key chords, Alt+Tab, Alt/Ctrl+Esc and Ctrl+Shift+Esc are refused, because they act on the shell rather
than the approved window. Text that looks like a secret (known token shapes, PEM keys, JWTs, long high-entropy words)
is refused, and so are password fields (checked before typing and between chunks).
*Your hands come first:* while an action runs, low-level keyboard and mouse hooks on their own thread watch for
events without `LLKHF_INJECTED`/`LLMHF_INJECTED`. `GetLastInputInfo` can't tell yours from ours. The callback only
stores atomics, because Windows silently drops slow low-level hooks. The Allow click is itself your input, so an
action first waits until you've let go for 500 ms (giving up as `aborted` after 10 s), then any input of yours stops
it before its next event. Stop, Win+Alt+Escape (a key press, so it aborts anyway) and `notifications/cancelled`
abort it too. While acting, the island stays collapsed and click-through (`hover::set_hands_off`), so a synthetic
click near the top centre lands on the app. An `agentActing` activity says "Waiting for you to let go…" then "Move
the mouse or press a key to stop", and the glow gets stronger with a crisp rim (motion-designer spec). One action
runs at a time across all runs.
*Alternatives:* raw coordinates only (brittle with DPI, scrolling and layout changes; refs plus a UIA re-find are
exact); runtime ids alone (some providers regenerate them, so automation id + type + name is the fallback, and must
be unique); short listed batches (approving blind); aborting the moment input arrives after Allow (the mouse moving
off the Allow button would abort every action); `GetLastInputInfo` polling (counts injected input); a permanent
low-level hook (system-wide cost while idle); arbitrary `ShellExecute` targets (would bypass the shell tool's
approvals).

### D28: Routines are local, fresh runs on a small schedule TopBar owns (Phase 2.7e)
*Why:* a routine is a saved instruction (name, prompt, folder, model, turns, budget, install) that runs from the
command window in one click or on a schedule while TopBar runs. Claude Code's own scheduling doesn't fit: `/loop` and
`CronCreate` live inside one open interactive session (in memory, idle-only, expiring after 7 days), and cloud
routines run on Anthropic's servers against a repo clone, with no local folders, desktop or island approvals. So a
routine run is an ordinary TopBar run started by `start_run`, the same path as a new task: same command line
(`--permission-mode default`, the prompt tool), same island status, Stop, hotkey and audit. It adds `--name` with the
routine's name and records `routine` and `trigger` in `AgentRun` and the audit's `start` line. Each run starts a fresh
session: resuming yesterday's would carry a growing, stale history at full price (the cache has long expired), and a
follow-up on a routine's run still resumes that run. Schedules are deliberately small: days at a time (covers daily
and weekdays), every N hours (1–24), and at TopBar start (which, with launch at startup, also covers "after login");
no cron syntax. `schedule.rs` is pure and tested with fake time zones: local times go through a `Zone` (Windows'
`SystemTimeToTzSpecificLocalTime` in production), a time skipped by DST runs at the first minute after it, a repeated
time runs once, and "every" counts real hours. One `agent-routines` thread sleeps until the next due time but never
more than a minute, so sleep, resume and clock changes need no power-broadcast hook: a wall-clock jump of more than
two minutes starts a two-minute settle period. A due time older than two minutes was missed; with "Run once when back"
(the default) only the latest one runs, at most 24 h late and after the settle period, so a PC asleep all weekend
never produces a burst. A routine still running when it's due again, more than two scheduled runs at once, a missing
folder and "Pause routines" all skip that time and say why, on the routine's row and in `routines.jsonl`. Routines
live in their own `routines.json`, which TopBar writes (temp file + rename); `settings.json` stays hand-edited.
Saving resets the schedule's clock when the schedule changes or is turned on, so saving never runs a time that already
passed.
*Alternatives:* Windows Task Scheduler entries (run without TopBar, but then nothing shows status or answers
approvals, and they outlive an uninstall); cron expressions (powerful, unreadable in a list); catching up every missed
time (bursts after a holiday); resuming one long session per routine (context and cost growth, stale state); storing
routines in `settings.json` (TopBar would rewrite a hand-edited file on every save, and `lastRun` churns).

### D29: Unattended runs: approvals wait for you; nothing is pre-approved (Phase 2.7e)
*Why:* a run at 03:00 hits approvals nobody answers, and they deny after 10 minutes. A per-routine allow list ticked
when saving (`Bash:git`, edits) was weighed and rejected: it is exactly the pre-approval the goal rules out, and its
scope drifts as the prompt or the folder changes. Instead, a scheduled routine's approvals are *patient* ("Approvals
wait for me", on by default): while the screen is locked or there has been no input for 5 minutes, the agent timer
pushes their deadline back by the time that passed, so the 10 minutes only run while you're there; a hard limit of
12 h from asking still denies eventually. The approval itself is unchanged: the same sticky island notice (with the
routine's name), Allow / Deny, and "for this run" where it was offered before. Desktop tools are off by default per
routine (all eleven passed in `--disallowedTools`), since most routines are shell and file work and nobody is there
to watch an action; when a routine allows them, each look and action is still approved one at a time and actions
still wait for you to let go of the mouse and keyboard. Budgets are required for routines ($0.05–$20).
*Alternatives:* leaving the timeout as is (a 03:00 run is always denied by 03:10); no timeout for routines (a run
could hold a tool call for days); a per-routine allow list (pre-approval); disabling approvals' "for this run" in
routine runs (unneeded: you still click it yourself).

### D30: TopBar's own look: a wave and amber, not Claude Code's spinner and orange (Phase 4a-1)
*Why:* until now the Claude UI copied Claude Code (the `· ✢ * ✶ ✻ ✽` glyph spinner, the `✻` mark, `#d77757` orange).
Anthropic's guidance for products built on Claude Code says they should keep their own branding and not look like
Claude Code, and TopBar is going public. The new accent is the amber of the author's portfolio site (`femiajanaku`,
dark-mode `--accent` `38 92% 55%` = `#f6a823`, deeper `#f59e0b`, ink `#0f0f0f` on it: white on amber is 2.0:1), as a
fixed token `--agent` (like `--charge`, never the album-art `--accent`). Amber on the black island is 10.5:1; the
weakest spot is 4.0:1 in the lightest corner of a warm amber album tint, away from any text.
*Working indicator (`WorkingIndicator`):* a sine wave (1.6 px, round caps) scrolling sideways by one wavelength every
1.6 s (linear, `translateX` only, so compositor-only), its ends faded with a mask; 22×10 in the pill, 14×6 in rows.
The mark (`AgentMark`) is the wave at rest, two periods. Reduced motion: the still wave. The edge glow (and the native
`glow.rs` band) moved to amber; the needs-input edge breathes on the site's `pulse-amber` timing (1 → 0.5 over 2 s).
With no album art, the equaliser is now soft white instead of `--accent`'s initial green, which sat beside the amber
wave and matched the charging green.
*How it was picked:* the motion-designer agent rendered candidates in headless Edge (pill, activities, approval card,
panel rows, command window; plain black, cool and warm tints; dpr 1.2 and 1.5; frame strips and GIFs). First round:
an orbit around a capsule (the island's shape), a shuttle in a pill track, and a rotating arc. The author asked for a
wavy line instead; second round: a scrolling ripple, a plucked "string" whose amplitude swings, and a "signal" squiggle
travelling along a flat line. The ripple won on the black island: the string moved on the same axis and tempo as the
equaliser (a second visualiser) and went flat twice per cycle; the signal read as an arrow or a heartbeat trace at 1×.
A breathing pill was deliberately not used: breathing already means "waiting on you".
*Alternatives:* keeping Claude Code's look (not allowed for a public product); the orbit (strong, but the author
preferred a wave); a breathing indicator (confuses working with waiting).

### D31: Display changes are followed by polling, not by listening for messages (Phase 4a-2)
*Why:* only the island's geometry was fixed at startup; the glow, screen capture and command window already looked up
their monitor on each use. The proposal was a `display-watcher` thread with a hidden top-level window (a message-only
window never gets the `WM_DISPLAYCHANGE` / `WM_SETTINGCHANGE` broadcasts) plus a polling backstop. Writing it showed the
backstop alone does the whole job: the poller already wakes every 250 ms for the fullscreen check, and reading the
primary monitor's rect (`GetMonitorInfoW`) and DPI (`GetDpiForMonitor`) costs microseconds. Polling can't miss a change,
whatever Windows does or doesn't tell a hidden window, and it fixes an ordering problem messages would have: Tao handles
`WM_DPICHANGED` on the island itself and resizes it to Windows' suggested rect, possibly *after* TopBar placed it.
So each tick compares the window's actual rect with where it belongs (`window::place`, pure and tested) and puts it
back (`SWP_NOACTIVATE | SWP_NOZORDER | SWP_ASYNCWINDOWPOS`, so the poller never waits on the main thread), the same
self-healing as D8. When the geometry itself changes, the poller drops React's last rect (measured at the old scale),
falls back to the defaults and emits `monitor:changed`; `usePanelSizeReporter` then measures again. The hook also
re-reports when `devicePixelRatio` changes (a `matchMedia` on the current resolution), which covers Text size changes,
a case that was already stale before: the CSS size, and so the `ResizeObserver`, can stay the same while the physical
size changes. Fullscreen hiding now compares with the island's monitor rather than the primary flag. The island stays
on the primary monitor: "the monitor with the cursor" would make it jump between screens as the mouse moves, and a
fixed choice belongs in Phase 4b's settings.
*Alternatives:* the watcher window from the proposal (a thread and a window for ~250 ms less latency); reacting only to
Tao's `ScaleFactorChanged` (misses resolution and primary-monitor changes); following the cursor's monitor.

### D32: The name is Cay, the licence MIT, and TopBar's data moves over once (Phase 4a-3)
*Why:* "TopBar" was a working name: generic, and shared with many existing tools. The niche turned out to be crowded:
GitHub, winget (which covers the Microsoft Store) and web trademark searches showed that Atoll, Islet and Isle (Isle
Notch) are Dynamic Island apps, BrimDeck and a project called Skerry are Dynamic Islands for Windows, Lintel is a Windows
"notch toolkit for AI coding agents, approvals" (nearly this app), Islo is Incredibuild's sandbox for coding agents,
Orla is both an AI agent tool and a small Windows top bar, and Spindrift is a registered software mark. **Cay** (a small,
low island, said "key") had no clash in the niche, keeps the island idea without copying a name, and is short; Wavepill
and wpill were the other finalists. The bundle identifier is `io.github.stanwukong.cay`, a namespace the author owns
(their GitHub Pages domain). The licence is MIT: the simplest permissive licence, usual for small desktop tools.
Everything that carried the name changed together: product and window titles, crate `cay` / `cay_lib` and so `cay.exe`,
the two pipes (`cay-claude-<SID>`, `cay-agent-<SID>`), the MCP server (`mcp__cay__approve`; the approval path is
otherwise identical, so permissions are unchanged), the bridge's environment variables, the Run value, window classes.
*Migration:* the new identifier is a different app to Windows, with its own `%APPDATA%` folder and single-instance key,
so `migrate.rs` runs first in `setup`. It *copies* `%APPDATA%\dev.wpill.topbar` (settings, routines, the agent audit;
not per-run files or WebView2's cache) into a temporary folder and renames it into place, only while Cay's folder
doesn't exist, so a failed copy is retried and an installed TopBar keeps working until it's uninstalled. A `TopBar` Run
value becomes `Cay` if it was on, in release builds only (a dev build would register `target\debug`, C5's rule). Claude
Code hooks are not rewritten automatically, because `~/.claude/settings.json` is only ever written after the user clicks
Connect: TopBar's entries count as ours to replace or remove, but not as connected, so the tray offers "Connect Claude
Code…" again and one click swaps them. The ownership check now also needs a path separator before the exe name, so a
`decay.exe` hook isn't mistaken for ours.
*Alternatives:* keeping TopBar (generic, not ownable); renaming silently with no migration (settings, routines and
startup would vanish for the one existing user); rewriting the hooks on first start (writes the user's Claude Code
settings without asking); moving instead of copying the old folder (a failed move half-loses data, and an installed
TopBar would start from nothing); GPL-3.0 or Apache-2.0 (the author chose MIT).

### D33: v0.2 ships unsigned, with a published checksum; signing comes in v0.3 (Phase 4a-4)
*Why:* the comparison (Microsoft's own "code signing options" page, checked 2026-11-01) changed the premise of this
stage. Since 2024 no certificate gives a new app instant SmartScreen trust, EV included: every signed app shows the
"Windows protected your PC" prompt until its publisher builds reputation over downloads and releases. Signing still
matters (a verified publisher name, reputation that carries across releases instead of per file, and enterprise
policies that block unsigned exes), but it doesn't remove the first-release warning. The cheapest route, Azure Artifact
Signing (~$9.99 a month, no hardware), only accepts individuals in the US or Canada, which excludes the author.
The remaining choices were SignPath Foundation (free for open-source projects, but the certificate is theirs, so the
publisher reads "SignPath Foundation", and releases must be built in GitHub Actions), Certum's Open Source certificate
(~€69, worldwide, the author's own name, a cloud key through their SimplySign app) and a standard OV certificate
($150–300 a year, a hardware token or cloud key store since 2023). The author chose to ship v0.2 unsigned rather than
wait days for identity checks, and to sign from v0.3. To keep an unsigned download checkable, each release lists the
installer's SHA-256 and the README shows how to compare it.
*Alternatives:* Azure Artifact Signing (not available to individuals outside the US and Canada); EV (no SmartScreen
advantage since 2024, $400+ a year); waiting for SignPath or Certum before releasing (delays v0.2 for a warning that
signing wouldn't remove yet).

### D34 (proposed): Agent UX after a competitor survey; permission modes, but never bypass (Phase 2.8)
*What:* a 2026-11-01 survey of comparable tools (Nook on Windows; Vibe Island, Open Island, Agent Notch, Notch, Vibe
Notch, FavTray, Crest on macOS) found that all of them watch and approve terminal sessions of one or more coding CLIs,
and none starts runs, has routines or gives the agent desktop tools. Their shared core, deciding from the island for a
terminal session, is what Cay lacks: it only focuses the window. Phase 2.8 adds that, diffs and full commands on the
card, questions and plans, a live step line and a Connect that previews its change. The author also asked for a
permission-mode toggle like Claude Code's Shift+Tab: Ask → Accept edits → Plan, chosen per task and cycled from a chip
on the island. *Why not bypass:* Claude Code itself hides `bypassPermissions` unless it's enabled by a flag, and Cay's
rule that runs never weaken permissions behind the user's back stays; Accept edits is the user's explicit choice per
run, covers file edits in the run's folder only, and leaves desktop tools, shell, web and MCP asking. Routines stay at
Ask (D29). *Skipped:* keyboard approval (the author's call; it would also need global hotkeys while a card shows,
since the island is never activated), mascots, sound packs and widget shelves (they clash with the minimal look).

### D35 (proposed): Google Calendar and Gmail through Cay's own Google sign-in (Phase 6)
*What:* upcoming events and tasks in the island (the Google Calendar icon in the pill while something is pending) and
Gmail/Calendar tools for agent runs, "out of the box". *Why Cay's own OAuth client:* the pill needs the data in Cay
itself, not inside a Claude session, and tools served by `cay.exe mcp` work for every backend (2.9) and go through
Cay's approvals. A desktop client with PKCE and a loopback redirect needs no server, so data only goes from Google to
the PC. *Costs:* Calendar and Tasks scopes need Google's verification; Gmail's read/compose scopes are restricted and
may need a yearly security assessment, so Gmail comes last and stays opt-in (drafts only, never send). *Alternatives:*
Google's official Workspace MCP servers (developer preview, and each user needs their own Cloud project and OAuth
client: not out of the box); the claude.ai Gmail/Calendar connectors (no work for Cay and kept as a day-one switch for
Claude Code runs, but they can't feed the pill and don't reach other agents); the calendar's iCal secret address
(no sign-in, kept as a fallback, but read-only and without tasks); Windows' `AppointmentStore` (fed by the old Mail and
Calendar app, which the new Outlook replaced; likely empty on current machines, not checked). *Icon:* Google's API brand guidance allows showing a product's icon for an
integration without asking, unaltered; Cay's own mark stays separate.

### D36: Terminal sessions' permission requests are held on their own pipe and never decided without a click (Phase 2.8a)
*What:* a `PermissionRequest` hook (sync, 75 s) makes `cay.exe hook` send the tool and its input to a second per-user
pipe, `cay-claude-ask-<SID>`, and wait for the answer, which it prints as the hook's `decision`. Cay shows a
`claudeApproval` card and holds the request until the first of: your Allow / Deny, the forwarder going away (answered in
the terminal), the session moving on, Cay shutting down, or 55 s. Only the click decides; every other ending replies
`pass`, the forwarder prints nothing, and Claude Code's own prompt decides, exactly as without Cay. Started at the
author's call ahead of the v0.2 release (4a-5), which resumes after 2.8. *Why a second pipe:* the hook pipe is
inbound-only with one instance and one message per connection, so a held request would block every other hook. The
agent pipe's duplex, multi-instance code moved to `duplex.rs` and now serves both. *Why `pass` and not deny on
timeout:* a request you never saw must not be refused for you, and an allow would be worse; handing it back costs a
terminal prompt at most. *Why no "for this run":* the terminal session belongs to Claude Code and its settings; Cay
widening it would be a permission change behind the user's back (`updatedPermissions` is never sent). *Requests from
Cay's own agent runs* pass at once: they also fire the hook, and with `--permission-prompt-tool` "whichever decides
first applies", so holding them would race Cay's own approval. *Not held yet:* `AskUserQuestion` and `ExitPlanMode`,
whose allow must carry the answer in `updatedInput` (2.8c). *Connect:* an install without the new entry counts as not
connected, so the tray offers Connect to update it. *Alternatives:* holding on the existing pipe (blocks other hooks);
a `PreToolUse` hook (fires for every tool, including those that never ask); auto-deny on timeout (FavTray hands back
too). *Checked:* forwarder fed hook files for allow, deny and a killed forwarder, and a real `claude -p` run (2.1.314)
that waited for Cay and ran its command. Claude Code 2.1.123 never ran the hook in `-p` mode.

### D37: Approval previews are built in Rust, capped on the card and whole in the command window (Phase 2.8b)
*What:* `agent/preview.rs` turns an `Edit` / `MultiEdit` / `Write` into a line diff (common prefix and suffix, then an
LCS table up to 1 M cells, else old then new) and a shell command into lines wrapped to 48 columns between tokens. Each
approval gets two copies: the card's (6 lines, one line of context, then "+n more lines") inside the activity, and the
whole one (≤2000 lines) kept in Rust and fetched by the command window. *Why Rust:* one tested place decides what you
see, for both card kinds, and React renders it as-is (the card's block is 48ch of Cascadia Mono, so Rust's wrap holds).
*Why not the whole diff in every event:* `agent:runs` repeats the pending approvals on every change; a 16 KB `Write` in
each would be wasteful. *Honest counts:* 2.8a's forwarder cuts strings at 16 KB; it now records which fields it cut, a
cut preview says "cut" with no counts, and a cut `Write` shows its new text alone (diffing a cut copy would show the old
file's tail as deleted). *Write against disk:* Cay reads the file being replaced (≤256 KB) so an overwrite reads as a
diff, not a new file. *Session requests in the command window* (the author's choice over focusing the terminal or a temp
`.diff`): a "Waiting in sessions" list with the whole preview and Allow / Deny, your click as on the card; the card's
title now focuses the terminal. *Alternatives:* a diff crate (`similar`; a dependency for ~80 lines), CSS-only wrapping
(it breaks inside tokens or overflows, and can't be tested), growing the card past D2's window.

### D38: Questions and plans are answered in Cay, but never with a bare allow (Phase 2.8c)
*What:* `AskUserQuestion` and `ExitPlanMode` are held like permission requests, for terminal sessions (the same
`PermissionRequest` hook) and agent runs (the prompt tool). The island shows a question's options as buttons and a plan
as "Plan ready"; the command window has every question with an Other field, and the plan as Markdown with Approve /
Send feedback / Reject. *Shapes, from a spike rather than guesses* (Claude Code 2.1.314, `-p` with a stand-in prompt
tool): the hook fires for both tools; an allow carrying `updatedInput` = the original `questions` plus an `answers` map
(question text → label, multi-select joined by ", ") reached Claude, and a plan's allow with its input unchanged ended
plan mode. *Never a bare allow:* `ask::Decision` gained `Answers`, `ApprovePlan` and `Feedback`; the forwarder builds the
output from the input as Claude Code sent it (not Cay's trimmed copy) and prints nothing for a plain allow or for answers
that don't match the questions, and both decide commands refuse Allow for these tools. Feedback is a deny whose message
is your words, which keeps Claude planning. *Mode after approval:* Cay sends no `updatedPermissions`, so approving a plan
never picks Accept edits or auto for you. *Answers kept in Rust:* partial answers live with the held request and the card
is re-offered in place (same activity key), so React holds no state beyond a multi-select's toggles. *Held 10 minutes:*
the author confirmed the terminal prompts while the card is up, so a long hold blocks nothing; permissions keep 55 s.
The entry's timeout went to 660 s, and `installed` now compares timeouts, so Connect offers the update. *Agent runs:*
`AskUserQuestion` is no longer disallowed, which removes "Claude can't ask mid-run". *Alternatives:* a `PreToolUse` hook
(documented for these tools, but it would answer before the terminal's own prompt and fires for every call without a
matcher), answering from the island only (no room for free text or a plan), a Markdown dependency (an in-house renderer
of React elements is enough and can't inject HTML).

### D39: The live step line comes from a background `PreToolUse` hook, summarized by the forwarder (Phase 2.8d)
*What:* the pill shows the current tool and its target beside the wave ("Edit · src/hover/logic.rs") and `n/m` from the
latest `TodoWrite`; panel rows and the run header show the same. Sessions get it from a new `PreToolUse` hook entry
(async, no output), runs from stream-json; both use `describe::step_line` and `todo_count`. *Why the forwarder
summarizes:* the pipe only ever carries a ≤64-character line and two counts, never the tool's input (file contents,
commands' full text), keeping 2.6's rule that the hook pipe holds no content. *Why async:* the hook fires before every
tool; a blocking hook would slow every call and could be mistaken for a decision. It never prints. *Width:* the step
sits in a fixed 180 px box so the island grows once when work starts, not on every tool. *Costs:* one short-lived
`cay.exe hook` per tool call in connected sessions; Connect must be run again (new entry). *Alternatives:* reading the
session transcript (`transcript_path`) for the latest tool (file I/O and parsing a format that isn't public), showing
only `PostToolUse` (the step would always lag one tool behind).

### D40: Agent runs use Claude Code's permission modes, Auto by default (Phase 2.8e; reverses D34's rule and D29)
*What:* the author asked for automation that flows like Claude Code itself rather than asking about every step. Runs
start in **Auto** (Claude Code's classifier approves routine actions and blocks risky ones) and can be switched to Ask
(`default`), Accept edits or Plan, per task, per routine (Auto for new ones) and mid-run from a chip, cycled like
Shift+Tab. Questions and plans still come to the island in every mode. *Supersedes:* D34's "Cay's runs never weaken
permissions; every prompt goes to the island" and D29's "routines stay at Ask". *What stays:* `bypassPermissions` and
`dontAsk` are never offered or passed; whatever Claude Code asks about still goes to the island, and only a click
decides; Cay never changes a terminal session's mode or writes the user's settings; the audit records every mode
change. *Desktop tools follow the mode* (the author's call): in Auto, Cay's own second check before a look or an action
is skipped, so they're judged by Claude Code's classifier like any tool; the screen glow and the your-input-aborts guard
stay, and in the other modes they ask as before. *How switching works (spiked, 2.1.314):* a stream-json
`control_request` `set_permission_mode` gets a `control_response` and a `system/status` with the new
`permissionMode`; between messages the choice is kept and passed as `--permission-mode` with the next follow-up. The
chip shows what Claude Code reports, so it can't drift from the real mode. *Alternatives:* the earlier proposal (Ask
default, per-run `ask` rules keeping `rm`/`mv`/`cp` asking under Accept edits; the author preferred Claude Code's own
semantics); offering Bypass (Claude Code itself hides it); a fixed per-run mode (no mid-run switch).

### D41: Connect has three states, and the status line is chained, not replaced (Phase 2.8f)
*What:* Connect reads a settings file as connected, out of date or not connected, so an install from an earlier version
says "Update Claude Code connection…" (with Update or Disconnect) instead of looking disconnected. Usage limits come
from Claude Code's status line input (`rate_limits`, no network call): Connect points `statusLine.command` at `cay.exe
statusline`, keeps the user's own `statusLine` in Cay's `settings.json`, and `cay.exe statusline` forwards only the
limits, then runs the user's command with the same input and prints its output unchanged. Disconnect puts their status
line back in place. *Why chain:* the author already has a HUD status line; replacing it silently was ruled out, and
"only if you have none" would have left this author without limits (the author's choice of three proposals). *Why keep
the original in Cay's file:* the user's `settings.json` stays readable (no encoded command in it) and Disconnect can
restore the exact object. *Same shell as Claude Code:* Git Bash when found, else PowerShell, per the statusline docs, so
quoting behaves the same (checked: identical output through Cay). *Sound:* Windows' own notification sound, off by
default, played when the island first shows something that needs you, so fullscreen and duplicates are handled by the
activity queue. *Windows Terminal tabs:* not built; nothing maps a process to a tab, and titles are Claude Code's own.
*Alternatives:* polling claude.ai's usage page (network, sign-in), reading Claude Code's transcripts
(no limits there), a bundled WAV (a system sound respects the user's sound scheme).

### D42: Google through Cay's own OAuth client, on Windows' own HTTP stack, polled in a 24 h window (Phase 6a)
*What:* the research D35 asked for, then the design the author agreed to (2026-11-02, all five points). *Sign-in:*
Google's installed-app flow as its current docs require: PKCE (S256), a loopback redirect on `http://127.0.0.1:<random
port>` (custom schemes and copy/paste codes are no longer supported), the system browser (embedded webviews fail with
`disallowed_useragent`). Refresh tokens are always issued to installed apps; `invalid_grant` means Connect again, and
revoking removes every grant of the project. Google calls desktop apps "public clients [that] cannot keep secrets
confidential" and makes `client_secret` optional, so the client id and secret are baked in at build time from
`CAY_GOOGLE_CLIENT_ID` / `CAY_GOOGLE_CLIENT_SECRET` (never in the repo; `settings.json` can name your own client).
*Verification:* sensitive scopes need a homepage and privacy policy on a verified domain, a demo video and a
justification per scope; unverified apps show a warning and are capped at 100 users, and *Testing* projects get 7-day
refresh tokens. So the project goes *In Production* unverified for the author and testers, and verification is done
before the public release (already the last step). Gmail's restricted scopes need a yearly assessment (CASA), and
Google's page names no exemption for local-only apps: asked before 6c. *No iCal fallback:* it has no tasks and would
need a recurrence engine (RRULE, exceptions, time zones), all thrown away once sign-in exists. *No sync tokens:* Google
refuses `syncToken` with `timeMin`, `timeMax` and `orderBy`; one windowed query per calendar (now − 12 h to now + 24 h,
`singleEvents`, `eventTypes=default,fromGmail`) every 5 minutes is a few KB and needs no local store. *No new crates
(dependency decision):* HTTP is WinHTTP through the `windows` crate (Windows' TLS, certificate store and proxy
settings), PKCE uses BCrypt's SHA-256 and RNG, tokens go to Credential Manager (`CredWriteW`, a generic credential
`Cay/google`, this user, not roamed), the loopback listener is `std::net`. Only `windows` features were added
(`Win32_Networking_WinHttp`, `Win32_Security_Credentials`, `Win32_NetworkManagement_IpHelper`, `…_Ndis`,
`Win32_Networking_WinSock`). reqwest or ureq would have added rustls and ring (about 1 MB) or native-tls on top of a
1.6 MB installer. *Rust owns time:* `logic.rs` gives each event a phase (later, soon, now) and the watcher wakes at the
next boundary (soon, start, end, local midnight), so React never runs a timer. It also wakes after a sleep (the wall
clock jumping more than 2 minutes past the monotonic one, as in D28) and when `NotifyNetworkConnectivityHintChange` says
the internet is back, and backs off 1 → 30 minutes while Google can't be reached. Unlock isn't watched: polling goes on
while locked, so the data is fresh anyway, and it saves a message-only window. *Join links:* Meet's `hangoutLink`,
then a `video` entry point, then a location that is an https link on a known meeting host; never http, never userinfo
(`https://meet.google.com@evil.example`). Links stay in Rust; React asks by id. *The pill (agreed):* art · wave · step
line · clock · calendar icon · equaliser, nothing dropped. *6c (agreed):* Calendar and Tasks reads follow Claude
Code's classifier in Auto; Gmail reads and drafts always ask in Cay, and once a run has read mail, desktop actions and
drafts ask again. *Alternatives:* the iCal secret address first (above); sync tokens (above); reqwest/ureq (size);
the claude.ai connectors only (can't feed the pill); Testing status (weekly sign-ins).

### D43: "Starting soon" is a timed notice timed by Rust, and Join is neutral white (Phase 6b)
*What:* the island's three calendar surfaces. *Activity:* `calendarSoon` is an ordinary D14 notice (priority 2, 8 s,
stale after 2 min, interactive while hovered), offered once per event and start time when the watcher reaches the
event's lead time; `next_boundary` now takes every offset it should wake at (soon, lead), so there's no polling and no
React timer. The minutes are Rust's ("in 5 min"), fixed at offer time; a moved event is announced again. Join and the
row's click retract the card through `Signal::Retract`, the same channel sessions use. *Icon (reverses D35's
"the Google Calendar icon"):* the author first picked Google's product icon, then, with it in hand, kept Cay's own
plain line calendar: a full-colour mark beside the monochrome clock fought D30's minimal look, and no Google artwork
or brand terms come into the repo. The panel can still name Google Calendar in words. *Join:* neutral soft white, not
`--accent`: on the plain island that token is the charging green, and amber is the agent's (D30). *Pill width:* 375 px
with everything at once; the mark adds 24 px to the 180 px step line's 351 px, still within the narrowest viewport (453
CSS px at 150 %). *Alternatives:* a sticky notice until the event starts (a calendar reminder isn't a question waiting on
you, D19); a second notice at the start (noise if you've already joined); recounting minutes in React (needs a clock).

### D44: One scroller for the panel's lists, and a quieter Calendar section (Phase 6b)
*What:* the island window grows to 680×600 logical px (was 440; it's transparent and click-through outside the
reported rect, so the room is free). In the panel, the clock and the media card stay fixed and everything after them
(Calendar, Claude Code, Running apps) sits in one `overflow-y: auto` column at natural heights. The per-section
shrink-and-scroll rules (Calendar's `min-height` + inner scroller, Claude's 180 px list, the apps grid's 300 px) are
gone: three nested scrollers competing for one flex column is what made "Claude Code" draw over "Running apps" and
rows disappear at 120 % Text size. The scroller reaches into the panel's padding (6 px left, 12 px right) so the rows'
−6 px hover bleed survives and a 6 px gutter (always reserved, so rows line up with the header) sits 6 px in from the
edge; its 4 px thumb shows only while the pointer is over the list. The island's box, which Rust hit-tests, stays the
capped panel height. Calendar: at most 3 of today's events, tomorrow's first only when today has nothing left, Join
only while an event is Soon or Now (it showed on tomorrow's), and the tasks as one line that opens the first task.
*Alternatives:* keeping per-section caps tuned per Text size (fragile, still overlapping in edge cases); a fade mask
at the scroller's edges (decoration; a hard clip is what Windows lists do).

### D45: The command window is Cay's, and ended runs can be removed (after Phase 6b)
*What:* the command window's title says "Cay" instead of "Claude agent"; the window is the user's Cay console, and
Claude stays named only where it is what runs (the prompt's placeholder, transcripts). Each ended run in the sidebar
gets a × (shown on hover or focus, in place of its time) and the Runs list a "Clear ended" action, both through a new
`agent_remove` command: Rust refuses live runs, drops the slot, retracts its "finished" notice and emits `agent:runs`
(the list never removes a row ahead of Rust). Runs are only in memory, so removing one forgets nothing on disk (the
audit trail stays). The window was restyled to read lighter (motion-designer): sentence-case section labels, filled
rather than outlined controls, a soft accent focus ring instead of a permanent amber border, a Windows 11 style
selected row with a short accent bar, and Segoe instead of Cascadia for sidebar names and prompt snippets.
*Alternatives:* removing live runs too (Stop already exists, and a hidden live run would be a process nobody sees);
persisting a "hidden" flag (runs don't outlive Cay anyway).

### D46: Google tools for agent runs, with Gmail asked in Cay and mail treated as someone else's words (Phase 6c)
*What:* five tools in Cay's own MCP server (`calendar_list_events`, `tasks_list`, `gmail_search`, `gmail_read`,
`gmail_draft`), served by `cay.exe mcp` like the desktop tools so every backend gets them (2.9). *Approvals (D42's
agreement, made concrete):* Calendar and Tasks reads behave like desktop looks: Claude Code's prompt or, in Auto, its
classifier; a call that arrives without a grant is asked in Cay; "for this run" covers each kind. Gmail reads never
follow Auto: without a grant Cay asks itself, in every mode; searches and reads share one "for this run" (reading
mail). A draft is a change, asks every time, is never remembered, and its card shows the recipients, subject and the
whole text. *Mail is untrusted:* the reply opens with a line saying the text is other people's and comes inside
`<email-data>`, and a run that has read mail (Cay's tools, or a claude.ai Gmail connector's tool seen in stream-json)
has every desktop action asked about in Cay, even in Auto. Calendar descriptions aren't returned (anyone can send an
invitation). *Scopes:* Gmail is opt-in (`google.gmail`), which adds `gmail.readonly` and `gmail.compose` to Connect.
Google has no drafts-only scope: compose also permits sending, so the dialog says Cay never sends, and Cay has no send
call at all. *Token:* the watcher stays the only holder of the access token and lends it per call over its channel
(`Wake::Token`); a 401 hands back the stale one for a fresh one, once. *Privacy:* the audit keeps counts and ids, the
draft body as its size, and the stream's tool lines are redacted the same way (they weren't for `type_text`, fixed
here). *Reversed: the `agent.claudeConnectors` switch.* Tested with Claude Code 2.1.314 (`--debug-file`):
`--strict-mcp-config` drops the claude.ai connectors and without it they load, and there's no "connectors only" flag,
so the switch would have been a second name for `agent.strictMcpConfig: false`. *Alternatives:* listing the tools only
when Google is connected (tools/list happens once per run, before you might connect); Gmail following Auto (its
classifier can't know a mail is someone else's instructions); switching a run to Ask once it reads mail (stronger, but
changes your mode behind your back; noted as a limitation instead); HTML replies for drafts (plain text is what Cay
can show you exactly on the card).

### D47: Other agents: Codex sessions through hooks, other runs through ACP, Ask only, and a watchdog (Phase 2.9)
*What (agreed with the author 2026-11-03, after research):* **sessions** of other CLIs reach the island like Claude
Code's, through the same forwarder (`cay.exe hook --agent codex`); **runs** of other agents go through one second
backend, the Agent Client Protocol (JSON-RPC over stdio, protocol version 1), first with Goose on a local Ollama model.
The native Claude Code path is untouched. *The rename:* `claude/` becomes `sessions/` (Claude-only parts in
`sessions/claude/`, Codex's in `sessions/codex/`), and the IPC names follow (`claude:sessions` → `sessions:changed`,
`claude_*` → `session_*`, `claudeDone` → `sessionDone`, `ClaudeSession` → `AgentSession`, `ClaudeSection` →
`AgentsSection`, `--font-claude` → `--font-agent`), in a commit of its own with no behaviour change. The pipe names and
the hook marker stay, so installed entries keep working and `hook` without `--agent` still means Claude. *Codex:*
hooks in `~/.codex/hooks.json` (`CODEX_HOME` for tests; no TOML crate). Its input uses Claude Code's field names; a
`PermissionRequest` is answered with `hookSpecificOutput.decision.behavior` allow/deny, and printing nothing hands it
back to Codex's own prompt, so Cay's `pass` costs nothing. `updatedInput`/`updatedPermissions` fail closed in Codex and
are never sent. `Interrupt` gives the Esc signal Claude Code lacks. Two catches: Codex skips a new or changed hook until
you trust it once with `/hooks` (Connect says so; status reads "waiting for trust" until a Codex hook arrives), and the
shell running a hook on Windows has changed between versions (cmd at 0.142, PowerShell in reports up to 0.164), with a
quoted path running nothing under PowerShell. So `commandWindows` is the exe's 8.3 short path, unquoted (on a volume
without 8.3 names, PowerShell's `& '…'`, which 0.166 runs; amended after the check below). A minimum Codex version is required (the author's call), checked by Connect.
`codex exec` has no approval channel, so Codex runs, if any, go through ACP (`codex-acp`). *OpenCode sessions:* a spike
of a plugin file piping into the same forwarder; attaching to the TUI's server was rejected (random port, no auth by
default). *Marks (D30 holds):* one amber wave for every agent; the agent is named by a small lowercase text tag
(`codex`, `goose`, …) in `--font-agent` and the secondary text colour, on cards and panel rows, never in the pill, and
only once a second agent kind has been seen. No glyphs, monograms or vendor colours (Vibe Island uses brand colours).
*ACP runs:* `agent::backend` over launch, encode, decode and stop, with `ClaudeCode` wrapping today's code unchanged;
`agent::acp` is a pure state machine. `session/request_permission` arrives on Cay's own stdio and becomes an
`agentApproval` card; Allow/Deny pick `allow_once`/`reject_once`, never `allow_always`. Cay's MCP server is handed over
in `session/new.mcpServers` (stdio `cay.exe mcp`, the run's id and token in `env`), verified per agent, since the spec
only says SHOULD; an allowed card for one of Cay's tools leaves the one-time grant (D25), so it isn't asked twice.
Calendar and Tasks stay on (the author's call); desktop tools and Gmail are off for non-Claude backends until a
scripted look-and-act test passes. *Ask only:* Goose's `auto` has no classifier at all (bypass in all but name), so
every non-Claude backend runs in its asking mode (`GOOSE_MODE=approve`, which the environment forces over the config;
OpenCode `"*": "ask"`). *The watchdog:* Goose's own docs call its write classification best-effort and model-made, so
a test alone isn't enough: a `tool_call` of kind edit, delete, move, execute or fetch that reaches `in_progress`
without a permission request for its id stops the run, is audited and is shown as "ran a step without asking". Each
backend also has a unit test that its launch forces asking and an ignored live test that a write and a shell command
each ask first. *Local models:* agent work needs ~32 GB RAM or a 24 GB GPU (Qwen3-Coder 30B-A3B, Devstral Small);
16 GB runs gpt-oss-20b slowly; the 8 GB dev machine can't, so the live Goose test runs on another machine (the
author's). *Gemini CLI:* no sessions in 2.9; later as an ACP backend if it passes the same check.
*Checked on Codex 0.166.0 (2026-11-03, scratch `CODEX_HOME`, a logging hook exe in a folder with a space):* hooks run
through `powershell.exe -NoProfile -Command "<command>"`. The unquoted 8.3 path and `& '<long path>'` both run; a quoted
long path runs nothing and the TUI shows "Hook failed · exited with code 1" on every session start. In the TUI a
`PermissionRequest` arrives as `{tool_name: "Bash", tool_input: {command}, permission_mode: "default", turn_id}`, with
no `description` and no `tool_use_id`, and its commands are PowerShell (`Set-Content -LiteralPath …`). Deny shows
"Blocked by hook · <message>" and the step doesn't run; allow runs it without Codex's prompt. `codex exec` never fires
`PermissionRequest` (it reports `bypassPermissions` and turns down what it can't ask about), so Codex is tested in the
TUI. 0.166 dropped `approval_policy = "untrusted"` (only `on-request` and `never`). `SessionEnd` has a 1 s default
timeout (3 s at most), so the forwarder must answer it well within that. The minimum Codex version is 0.166.0, the
one checked.
*OpenCode spike (1.19.5, 2026-11-03, scratch `OPENCODE_CONFIG_DIR`, a free `opencode/…` model):* a plugin file in
`plugins/` loads and gets every bus event (`session.status` busy/idle, `session.idle`, `permission.asked` with the
command or file, `permission.replied`) plus `tool.execute.before` with the tool's arguments. The `permission.ask` hook
is declared in `@opencode-ai/plugin`'s types but **never called** (the request went straight to `permission.asked`),
so a plugin can't decide through it. It can answer through its own in-process client:
`client.postSessionIdPermissionsPermissionId({ path: { id, permissionID }, body: { response: "once" | "reject" } })`
was accepted (HTTP 200, `permission.replied`) and the command ran or didn't. `opencode run` rejects every request
within ~10 ms, so the TUI and `opencode serve` are where approvals matter. OpenCode's defaults allow edits and shell
commands, so without the user's own `"ask"` rules there's nothing to approve and the island only shows status.
*Verdict:* OpenCode sessions are worth doing as a plugin that pipes Claude-shaped hook JSON into the same forwarder
(`cay.exe hook --agent opencode`) and, for `permission.asked`, waits for the forwarder's decision and replies through
its client; attaching Cay to OpenCode's server stays out (random port, no auth by default).
*Alternatives:* keeping `claude/` and the IPC names (cheaper now, misleading forever); `config.toml` hooks (needs a
TOML writer); a quoted or PowerShell-only `commandWindows` (breaks on the other shell); `codex exec --json` runs (no
approvals); per-agent native run paths (more code for the same events); Goose's `smart_approve` or `auto` (the model
decides what asks); brand colours or monograms per agent (vendor look, D30); Gmail and desktop tools for local models
from day one (the worst prompt-injection case).

### D48: Other agents' runs use the same four modes as Claude Code, Auto by default (Phase 2.9b; reverses part of D47)
*What:* the author's call (2026-11-04): runs on Codex, OpenCode and Goose shouldn't ask about almost every step, which
would undo the point of an automated agent; they follow the same protocol as Claude Code's runs (D40). Every backend
offers Auto (default), Accept edits, Ask and Plan, per task, per routine and switchable like Claude Code's, each
mapped to the agent's own nearest setting:

| Cay mode | Claude Code | Codex (`codex-acp`) | Goose | OpenCode (per-run rules) |
|---|---|---|---|---|
| Auto | classifier | workspace sandbox; leaving it or the network asks | `smart_approve` | edits in the folder and safe commands run; other shell, outside the folder and web fetches ask |
| Accept edits | `acceptEdits` | edits run, the rest asks | `smart_approve` | edits allowed, shell asks |
| Ask | `default` | read-only, asks | `approve` | `"*": "ask"` |
| Plan | `plan` | read-only | `chat` | its read-only `plan` agent |

*Reversed from D47:* "Ask only" for non-Claude backends; the watchdog in every mode (now Ask only, where it checks the
agent really asks; in the other modes steps running unasked is the point); desktop tools and Gmail off until a
scripted look-and-act test (now available to every backend from the start). *Desktop actions follow the mode* (the
author's call, chosen over asking in Cay for agents without a classifier): in Auto they run without a card, as for
Claude Code, but no classifier judges them; the screen glow and the your-input-aborts guard (D27) stay. *What stays:*
the agents' approve-everything settings (Codex full access, Goose `auto`, OpenCode `allow` for everything) are never
used, as `bypassPermissions` isn't (D40); Gmail asks in Cay in every mode, and a run that has read mail has its desktop
actions asked in Cay even in Auto (D46); only a click decides what reaches the island. *Order:* OpenCode first (built-in
ACP, free models, testable on the dev machine), then Codex through `codex-acp`, then Goose on the author's other
machine. *Honest difference:* "Auto" protects less on OpenCode (pattern rules) and Goose (the model's own judgement)
than on Claude Code (a classifier) or Codex (an OS sandbox); the command window will say which agent a run uses, and
the README says what each Auto means. *Alternatives:* Ask only (D47; the author found it defeats automation); asking in
Cay for desktop actions on agents without a classifier (recommended, declined); OpenCode Auto allowing all shell with
a risky-pattern list (a list can't catch everything) or edits only (too many cards).
*Built (2.9b, OpenCode 1.19.5, 2026-11-04):* a run records its `AgentBackend`; Claude Code keeps its code path
(`run::spawn`, `stream.rs`), and ACP agents get an `acp::Client` per process whose `StreamEvent`s feed the same
`lifecycle.rs` (an enum on the run, not the trait PLAN once named: two shapes, and the lifecycle is already shared).
Both start through `run::spawn_command` (suspended, Job Object). Checked against `opencode acp` before writing the
parser: `initialize` offers `loadSession`, `session/new` returns `configOptions` (model, effort, `mode` build/plan,
set with `session/set_config_option`), and a shell step goes `tool_call` → `in_progress` → `session/request_permission`.
So the watchdog fires on a risky step that *completes* unasked (in Ask and Plan), not on `in_progress`. Per-run rules
go in `OPENCODE_CONFIG_CONTENT` (checked: it beats the user's own `"ask"`), so no OpenCode file is written. Mode changes
for ACP runs apply from the next message (the rules are fixed per process). Cay's MCP server now offers each connection
only its tools (`mcp::Offer`): no `approve` for ACP agents, and a routine's desktop/Google switches enforced by Cay.
OpenCode names MCP tools `cay_<tool>`; they read as `mcp__cay__<tool>` so grants and cards match. ACP has no system
prompt, so the first message carries Cay's guidance. *Checked live through Cay:* Ask (your Allow from the island, a
Deny, a follow-up resuming the session in a new process), Auto (a write and `git status` with no card), Stop.
*Codex (2.9b):* through `@agentclientprotocol/codex-acp` 2.5.0 (Zed's `@zed-industries/codex-acp` is deprecated in
its favour), a Node script Cay starts as `node.exe <script>`; it bundles Codex 0.165 and uses your Codex login.
Its modes turned out better than D48's table assumed: `agent` is "Auto review" (it asks only about actions it
judges unsafe), so Codex's Auto has a reviewer like Claude Code's. Cay maps Auto → `agent`, Accept edits →
`workspace-write`, Ask → `read-only`, Plan → `read-only` + `collaboration_mode: plan`; `agent-full-access` is
never used. The mode is set at startup (`INITIAL_AGENT_MODE`) and as session config, and the first message now
waits for every config answer; a mode the agent refuses or lacks ends the run instead of running in another mode.
Codex's "reject once" option is `cancel`: declining one step ends the turn (`stopReason: cancelled`), which Cay now
closes as done with a note, not as an error. *Checked live through Cay:* Ask (a card with the full PowerShell
command; Deny wrote nothing), Auto (a write with no card).

## 5. Challenges & resolutions

### C1: The panel collapsed while the cursor was visibly on it
*Symptom:* moving sideways inside the expanded panel collapsed it, though only near the edges and
only on the dev machine. *Diagnosis:* logging `set_panel_rect` showed the panel reported as
568 px wide instead of 640. Windows **Text size** was 120 %. WebView2 applies it as page zoom
(`devicePixelRatio` 1.2), but the monitor scale factor Rust uses was 1.0. React was sending CSS px
and Rust was treating them as logical px, so the hit area came out about 17 % too small. The CSS
width had also been silently squeezed by flexbox into the 567-CSS-px viewport.
*Fix:* React sends physical px (D7). The CSS width became `min(640px, 100vw − 32px)`.
*Lesson:* "logical px" isn't one thing on Windows. Display scaling and text scaling are separate,
and only the webview sees both.

### C2: The island sometimes never appeared on first launch
*Symptom:* on the very first run (cold WebView2 profile) the window existed with the right
styles but wasn't visible. External `ShowWindow` calls didn't stick either.
*Root cause:* the window builder's `visible(false)` raced with our `ShowWindow(SW_SHOWNOACTIVATE)` in
`setup()`. When webview creation was slow, Tauri's hide landed second.
*Fix:* the poller enforces visibility (D8). This was verified by hiding the window externally: it came
back within 250 ms with the foreground app unchanged.

### C3: The first measurement after expanding was the pill
*Symptom:* the first `set_panel_rect` after an expand sometimes carried the pill's size (201×50),
so the leave test briefly used a tiny rect. *Fix:* React only reports once the element's DOM state
says it's expanded, and Rust forgets the previous rect on every expand, falling back to a default
until a fresh report arrives.

### C4: The island blinked out right before collapsing
*Symptom (user report):* moving off the expanded panel made it flash before it shrank.
*Diagnosis:* I captured screen frames every ~20 ms from outside the app. For ~140 ms after the cursor
left, the island was **entirely absent**. It then reappeared at full size and only then animated
closed. The gap started exactly when the poller switched the window to click-through
(the *Leaving* state). Tauri's `set_ignore_cursor_events` toggles `WS_EX_LAYERED` as well as
`WS_EX_TRANSPARENT`, and my re-assert of our own style bits forced `SWP_FRAMECHANGED`. Changing
the layered bit makes DWM drop the WebView2 surface until it repaints.
*Fix:* stop using Tauri's toggle. The window is created layered and stays layered (the collapsed
configuration, already known to render correctly), and click-through is a single
`WS_EX_TRANSPARENT` flip with no frame change. The re-capture showed the island staying intact through
the leave delay and shrinking smoothly. Hit-testing was confirmed with `WindowFromPoint`: clicks pass through
when collapsed and land on the island when expanded. A smaller related issue: Framer's `popLayout` positions
exiting content against the nearest positioned ancestor, so `.island` became `position: relative`
to keep that content clipped during the morph.
*Lesson:* a frame-burst capture plus a pixel count per frame is a cheap, objective way to diagnose
"it flickers" reports.

### C5: "Launch at startup" worked but TopBar was missing from Startup apps
*Symptom (user report):* after ticking Launch at startup in the installed build, Task Manager's
Startup apps didn't list TopBar. *Diagnosis:* the registry entry existed and Windows' own flag said
enabled, but the value was `C:\Users\DELL 7300\AppData\Local\TopBar\topbar.exe ` (**unquoted, with a
trailing space**). The `auto-launch` crate behind `tauri-plugin-autostart` formats `"{path} {args}"`
and never quotes. Any profile path with a space (common: "Firstname Lastname") gets an entry
that Task Manager can't resolve. It's also the classic unquoted-path hijack vector (`C:\Users\DELL.exe`).
*Fix:* dropped the plugin for a ~90-line `autostart.rs` on `windows-registry`. It writes a quoted path,
honors and clears Task Manager's `StartupApproved` switch, and repairs an existing unquoted entry for
the same exe on launch, so already-installed users get fixed.
*Lesson:* test paths with spaces. The dev machine happened to have one, and that caught it.

### C6: A screenshot "dropped" back onto the island counted as shared
*Symptom:* a synthetic drag released over the island itself logged `dropped: true`, which would have ended the activity
as if the file had gone somewhere. *Diagnosis:* the island window had been built with `disable_drag_drop_handler()` (to
avoid Tauri file-drop events), which left WebView2's own drop target in charge. It accepted the file with a copy effect.
It didn't navigate this time, but a webview deciding what to do with a dropped file is not something to rely on.
*Fix:* keep Tauri's drop handler (it intercepts drops before WebView2 and only emits an event nobody listens to), and
count a drop as shared only if the cursor is no longer over the island (`WindowFromPoint` → root == island). The same
drag then logged `dropped: false` and the activity stayed up.
*Lesson:* "disable the handler" can mean "hand the event to the next handler in line", not "ignore it".

### C7: Sample hooks reached the forwarder but never the island
*Symptom:* while testing, piping hand-written hook JSON into `topbar.exe hook` produced nothing in the log, while the
same envelope written to the pipe from .NET worked. *Diagnosis:* a temporary trace build showed stdin arriving and the
ancestor chain resolving, then no envelope: the shell had collapsed `C:\\x` in the test string to `C:\x`, an
invalid JSON escape, so the forwarder (correctly) dropped the input. *Fix:* generate the samples as files with a
script and pipe those. *Lesson:* test the boundary with bytes you've inspected (`od -c`); a quiet forwarder is the
design, so "nothing happened" needs a trace build, not guesses.

### D49: Memory-heavy apps from one kernel snapshot, grouped by exe path, refreshed only while the panel is open (Phase 3)
*What:* the panel's "Memory" section (off by default, `processes.enabled`) lists the apps using the most memory.
One `NtQuerySystemInformation(SystemProcessInformation)` call reads every process (3.5 ms for 339 on the dev PC) with
no process handles, so elevated and system processes are read too; the figure is the *private working set*, Task
Manager's "Memory" column. Processes group by exe path, with helper processes (WebView2, crash handlers) folded into the
app that started them, so Teams and WhatsApp carry their WebView2 and Chrome is one row; other children (a VS Code
terminal's shell) keep their own row. A `processes-watcher` thread waits on a condvar until the hover poller says the
panel is open, then refreshes every 2 s and pushes `processes:heavy`; while closed it costs nothing. The protected list
is enforced in Rust (PLAN's list plus Defender, `audiodg`, `fontdrvhost`, `sihost`, `ctfmon`, `conhost`, Memory
Compression, every session-0 process and Cay itself), and a whole group is protected if any member is. End Task (3b)
will confirm in a native message box, close windows before terminating, and revalidate `(pid, creation time)`.
*Why:* the author approved the routine's draft as recommended (2026-11-04).
*Alternatives:* `sysinfo` (a new dependency, and it reports the working set and commit, neither of which is Task
Manager's figure); grouping by exe name (one giant "msedgewebview2" row) or by process tree (would end a terminal the
user started from VS Code); the command window as the confirm; a read-only agent tool (later, maybe).
*Costs:* no new crate, one `windows` feature (`Wdk_System_SystemInformation`). The entry's layout is undocumented
(phnt's names behind the crate's `Reserved*` fields, stable since Vista); a live test pins the private working set and
the creation-time offset against `GetProcessTimes`. Exe paths are cached by `(pid, created)` (opening every process
took ~3 s from PowerShell), display names (`FileDescription`) by path.

### D50: Ending an app: Rust confirms, checks twice, and never ends an app that's asking to save (Phase 3b)
*What:* `end_process_group(key, pids)` shows the native confirm box itself ("End WhatsApp? 3 processes, 1.2 GB…",
OK / Cancel with Cancel the default) rather than leaving the confirm to the page. It only touches pids Rust listed for
that app in its latest push (anything else reads `changed`), checks them against a fresh snapshot before the box and
again after it (the box can stay open a while), and refuses the whole app if any member is protected. Each
`TerminateProcess` goes through a handle opened first and checked for the listed creation time, so the pid can't be
reused between the check and the call. One addition to D49: if an app's window is disabled after `WM_CLOSE`, it is
showing a dialog ("Save changes?"), so Cay leaves it open (`prompted`) instead of ending it under the prompt after 5 s.
The reserved `ok` outcome became `ended`, and `prompted` and `failed` were added.
*Why:* the island page is untrusted as far as ending processes goes: a confirm in React could be skipped by a bug, and
pids from the page could be stale or arbitrary. Ending an app that just asked "Save changes?" would lose exactly the
work `WM_CLOSE` was meant to protect.
*Alternatives:* confirm in React and keep the command a bare "end these pids" (rejected: the page decides); a
TaskDialog with an "End" button (needs Common Controls 6 in Cay's manifest; MessageBox matches Connect); always
terminating after 5 s as the design said (rejected for the reason above).

### C8: The agent pipe refused every bridge's hello
*Symptom:* the first real agent run failed with "MCP tool mcp__topbar__approve not found"; TopBar's log said it had
refused two connections "without a valid run token". *Diagnosis:* the bridge writes its hello as soon as it connects,
so by the time TopBar reads, the message is already waiting and the overlapped `ReadFile` completes at once. On that
path the byte count wasn't fetched (no count pointer is passed with overlapped I/O), so the message read as empty and
failed to parse. 2.6's pipe never hit this because it always calls `GetOverlappedResult` afterwards. *Fix:* fetch the
count from `GetOverlappedResult` on the immediate path too. *Lesson:* with overlapped I/O, "completed synchronously" is
a normal outcome, not a fast path to skip; and the audit trail made the user-visible symptom obvious on the first run.

### C9: Paths and disks on the test machine
Two environment problems surfaced while verifying 2.7. The scratch folder came in as an 8.3 short path
(`C:\Users\DELL73~1\…`), and Claude Code then asked before reading files inside its own working folder, because it
compares paths as text; run folders are now normalised with `GetLongPathNameW`. And the debug `target` folder had
grown to ~20 GB of stale builds, filling the drive mid-link (`os error 112`); `cargo clean --profile dev` freed 14 GB.

### C10: Proving the island isn't in a screen capture
*Symptom:* the first whole-screen screenshot from a real run showed no island, but the user was watching a
fullscreen video, and the island hides itself during fullscreen, so the image proved nothing. *Resolution:* a
debug-only `agent-test look <tool>` runs a look directly (no Claude, no approval, glow included) and saves the image;
a small script waited until the island window was visible again and took the capture then. The pill wasn't in it.
The glow, being excluded from capture, can't be screenshotted at all; it was checked through its window instead
(visible over 1920×1080 with display affinity 17 during a look, hidden ~2.3 s later). Extracting that first image
also showed that Claude Code keeps every tool image in its own session transcript under `~/.claude/projects`,
which is now a documented limitation. *Lesson:* check the precondition (is the thing you expect to be hidden
actually on screen?) before trusting an absence.

### C11: Typed text arrived as dots, then scrambled
*Symptom:* the first `type_text` into Windows 11 Notepad turned " typed by TopBar." into a space and sixteen dots.
Sending one character per `SendInput` call gave "Typed y opBar,aaaaaaaaaat ......."; real key presses (from
`VkKeyScanExW`) gave "ped y opBar 2.7d;" (Shift lost, keys dropped); one event per call with an 8 ms gap gave
"@>&D" for "2.7d" (Shift stuck) and a "." after the next line's Enter (reordered). The UI Automation reply said "Typed
82 characters" every time. *Diagnosis:* the same code typed the same text exactly into WordPad (classic RichEdit), so
the input was right and Notepad's XAML input stack was the problem. A `KEYEVENTF_UNICODE` (`VK_PACKET`) character is
only read back when the app translates the message, so in an app that handles input late, every queued packet comes
out as the latest one: the dots were the final ".". Real keys carry their state with the message, but the new
Notepad still drops keys and reads Shift late when events arrive close together. *Fix:* characters are typed as
real keys from the target window's keyboard layout (Shift added, Caps Lock allowed for), Unicode packets only for
what the layout can't type, every key event sent on its own 25 ms apart, and 60 ms after a modifier goes down. Two
Shift-heavy strings (80 and 152 characters) then arrived exactly in Notepad. Typing is now about 10 characters a
second, so `type_text` takes at most 500 and the guidance prefers `set_value`. *Lesson:* "the API accepted the
input" says nothing about what the app saw; read the result back (UIA or a screenshot) and try a second app before
blaming either side.

### C12: Driving routines from outside: arguments and a run that "finished" but was still running
*Symptom:* `agent-test routine save '{…}'` from Windows PowerShell 5.1 failed with "key must be a string", then "EOF
while parsing a string": PowerShell drops the inner double quotes when it passes a string to a native exe, and
escaping them still split the JSON at spaces. Later a routine whose run had reported "done" was skipped as
`stillRunning`. *Diagnosis:* the first is PowerShell's native argument passing (the same family as C7); Git Bash
passes the argument whole. The second was real: Claude had started `sleep 90` with `run_in_background`, so
`claude.exe` stayed alive after its `result` until the shell ended, and TopBar counts a run with a live process as
going. *Fix:* the tests drive `agent-test` from Git Bash; the skip is correct (the process still holds the Job
Object, and Stop is what ends it) and is listed as a known limitation. *Lesson:* "done" in the transcript and "no
process" are different facts; scheduling decisions use the process.

### C13: The island "didn't resize" at 150 %
*Symptom:* while testing 4a-2, TopBar's log said it had moved the island to 1020×660 at x=450 after the scale went to
150 %, but a PowerShell check of its window rect still read 680×440 at x=300. *Diagnosis:* PowerShell isn't DPI-aware,
so Windows reports other windows' rects to it scaled down to 96 DPI: 1020 / 1.5 = 680 and 450 / 1.5 = 300, an exact
match. *Fix:* the test helpers call `SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2)` first; the same check then read
1020×660 at x=450, and screen captures at 150 % show the pill centred. *Lesson:* at anything but 100 %, a measuring tool
needs the same DPI awareness as the app it measures, or it reports a consistent but scaled answer that looks like a bug.

### C14: The drive filled up mid-build (again)
*Symptom:* while building 6a, `cargo test` failed with "no space on device" and the running `pnpm tauri dev` Cay
vanished (its rebuild died the same way). *Diagnosis:* `target\debug` had grown to 16 GB: 3.8 GB of incremental cache,
376 MB of stale `cay_lib` test binaries and 2.3 GB of `topbar*` artifacts left from before the rename (D32), with
1.4 GB free on the drive. *Fix:* deleted `target\debug\incremental`, Cay's own stale artifacts and every `topbar*` file,
keeping the third-party crates (no full rebuild): 6.4 GB free, and Cay rebuilt in about 20 s. *Lesson:* after a rename,
the old crate's artifacts are never reused or cleaned by cargo; remove them by hand.

### C15: Testing Codex hooks: a blocked flag, an expired login, a leftover CODEX_HOME (2.9a)
*Symptom:* checking Codex 0.166's hooks took four tries. Claude Code's auto mode refused
`--dangerously-bypass-hook-trust` (a safety-bypass flag) and refused writing its own allow rule (self-modification);
the author added the rule. Then the copied `auth.json` was five months stale (`invalid_jwt`), `gpt-5.5` from the
author's config isn't offered to ChatGPT accounts on 0.166, and `codex exec` never fired `PermissionRequest`. In the
manual check, `/hooks` listed nothing: Connect had written Cay's entries into the scratch `CODEX_HOME`, still set in the
window that started the dev build. *Fix:* a scratch `CODEX_HOME` with its own `codex login` (never a shared refresh
token), a logging hook exe built into a folder with a space, the TUI for permission requests, and Connect's dialog now
says when `CODEX_HOME` sends it away from `~\.codex`. *Lesson:* agent CLIs read their home from the environment,
so tests and Connect must both say which folder they touch; and a hook only proves itself in the mode that uses it.

### Still open
- **Foreground-lock rules** in practice: the fallback chain is implemented but not yet exercised
  against elevated or minimized targets (manual test 7).
- **Testing approach that worked:** with no UI test harness, driving the real cursor with
  `SetCursorPos` and reading the island's `GWL_EXSTYLE` (click-through bit = expanded state) from
  PowerShell gave reliable end-to-end checks of the Rust state machine without touching the app.
  For Phase 2 the same cursor drive plus a `CopyFromScreen` capture of the expanded panel confirmed the
  media card (art, title, progress) and the pill's art + equalizer against a live Spotify session.
- **GSMTC from PowerShell 5.1 is misleading:** a script using `GetSessions()` listed nothing while Spotify had
  a session (Windows PowerShell doesn't enumerate the projected `IVectorView`). Trust the debug log instead.
- **Media session identity:** matching sessions by `IUnknown` pointer held up with Spotify and a
  browser at the same time (manual check 6, 2026-10-30).

## 6. Timeline

| Date | Milestone |
|---|---|
| 2026-10-28 | Phase 0: environment audit, Tauri 2 API research, architecture + IPC contract drafted |
| 2026-10-28 | Phase 1 built: island window, hover state machine, pill ⇄ panel animation, running apps with icons, activation, fullscreen hiding. Fixed text-size DPI mismatch and the startup visibility race. |
| 2026-10-28 | Fixed collapse flash (own click-through toggling, permanently layered window); size-optimized release profile. |
| 2026-10-28 | Phase 1 signed off: all 11 manual tests pass. Roadmap reprioritized (D9). |
| 2026-10-28 | Phase 1.5 built: tray menu, launch at startup, single instance (verified: second launch exits). |
| 2026-10-28 | Replaced `tauri-plugin-autostart` with `autostart.rs` (quoted Run path; C5). |
| 2026-10-30 | Phase 1.5 signed off: tray, startup launch, Task Manager toggle and single instance all pass by hand. |
| 2026-10-30 | Phase 2 built: GSMTC media watcher (D11), filtered `media:state` (D12), media card with seekable progress, pill art + equalizer, session switcher. |
| 2026-10-30 | Phase 2 signed off: all 10 manual media checks pass (Spotify, YouTube, two sessions, nothing playing, text sizes). |
| 2026-10-30 | Island background themed from the album art (D13). |
| 2026-10-30 | Phase 2.5 kick-off prompt written (`prompts/`), so each phase can start in a fresh session. |
| 2026-10-30 | Phase 2.5 built: activity queue as a hover phase (D14), six sources on four watcher threads plus the poller (D15), screenshot drag-out (D16, C6). Verified from outside; manual checklist pending. |
| 2026-10-30 | Phase 2.6 (Claude Code activity) added to the roadmap after 2.5 (D17), with its kick-off prompt. |
| 2026-10-30 | Charging activity: orbiting green outline + halo; `motion-designer` agent added for all UI changes (D18). |
| 2026-10-30 | Phase 2.5 signed off: all 12 manual checks pass (volume, brightness, Bluetooth headphones, charger, Caps Lock, screenshot drag-out, panel and fullscreen behavior). |
| 2026-10-30 | Phase 2.6 built: hook forwarder + per-user pipe (D17, D20), sticky "needs input" notices (D19), Claude spinner, orange edge and panel section (via the motion-designer agent). Verified from outside and with real Claude Code 2.1.123/2.1.159 sessions; manual checklist pending. |
| 2026-10-30 | Phase 2.6 signed off: all 12 manual checks pass (Windows Terminal, VS Code, two sessions, permission prompts, Esc, fullscreen, TopBar not running, disconnect). |
| 2026-10-30 | Phase 2.7 (local automation agent) planned and placed next on the roadmap (D21), with its kick-off prompt. |
| 2026-10-30 | Phase 2.7a–b built: headless runs in Job Objects (D22), approvals in the island through `topbar.exe mcp` and a token-checked pipe (D23), the command window and the Cascadia Mono refresh (D24, via the motion-designer agent). Verified from outside with haiku runs (deny, allow, allow for this run, turn limit, follow-up resume, Stop with `ping -t`, TopBar killed and quit mid-run); manual checklist pending. |
| 2026-10-30 | Phase 2.7a–b signed off: all 12 manual checks pass (hotkey, read-only task, deny, allow and "for this run", Stop and the stop hotkey, limits, two runs, panel and fullscreen, follow-up, quit mid-run, audit, text sizes). Stage 2.7c kick-off prompt written. |
| 2026-10-30 | Stage 2.7c built: read-only desktop tools with a second, TopBar-side approval check (D25), a glow on the screen's edges while Claude looks (D26), an eye in the approval's `read` badge (motion-designer). Verified from outside with haiku runs and direct looks (C10); manual checklist pending. |
| 2026-10-30 | Stage 2.7c signed off: all 12 manual checks pass (window list, glow, UI tree, password field, covered window, island not captured, deny, for this run, gone/minimized/elevated, locked screen, audit, text sizes). |
| 2026-10-31 | Stage 2.7d built: seven desktop actions, UI Automation first with `[n]` refs, one approval each, aborted by your input (D27); pointer badge, acting activity and acting glow (motion-designer). Verified with haiku runs; manual checklist pending. |
| 2026-10-31 | Stage 2.7d signed off: all 12 manual checks pass (set a field, invoke behind, raw input, command window text, deny, abort, hands off, gone/elevated/locked, password and secrets, launch, audit, text sizes). Stage 2.7e kick-off prompt written. |
| 2026-10-31 | Stage 2.7e built: routines saved in `routines.json`, run now or on a schedule by one Rust thread (D28), patient approvals and desktop tools off by default for unattended runs (D29), the routines list and editor in the command window (motion-designer). Verified from outside with haiku runs (run now, a schedule firing once, approval and deny, overlap, Stop and stop-all, pause, a missed time caught up once and one skipped, delete); manual checklist pending. |
| 2026-10-31 | Stage 2.7e signed off: all 12 manual checks pass (save as routine, run now, edit, schedule, approval, away, overlap, missed while closed, pause, stop, delete, text sizes). **Phase 2.7 complete.** Phase 4a (release readiness) starts. |
| 2026-11-01 | Stage 4a-1: TopBar's own look (D30). The Claude Code-style spinner, `✻` and orange are replaced by an amber wave, its mark and an amber edge, chosen from two rounds of rendered candidates (motion-designer); checked live with a haiku run. |
| 2026-11-01 | Stage 4a-2: the island follows display changes (D31): the poller re-reads the primary monitor every 250 ms and re-places the island; `monitor:changed` makes React measure again. Verified at 100/125/150 %, a temporary 1280×720 and a fullscreen window (C13); the dock check is the user's. |
| 2026-11-01 | Stage 4a-3: renamed **Cay** (D32) after name checks; MIT licence; README with install, privacy and agent-safety sections; one-time migration of TopBar's data, startup entry and hooks. Verified with the dev build (41 files copied, a haiku run approved through `mcp__cay__approve`). |
| 2026-11-01 | Name kept although `cay.app` / `cay.dev` are taken. Stage 4a-4 decided: v0.2 ships unsigned with a published SHA-256 (D33); signing from v0.3 (Certum Open Source or SignPath). |
| 2026-11-01 | 4a-5 (v0.2) set aside at the author's call to start Phase 2.8 (D34). Stage 2.8a: terminal sessions' permission requests answered from the island (D36), checked with a real `claude -p` run, then signed off by hand. Kick-off prompt for 2.8b–f written. |
| 2026-11-01 | Stage 2.8b built: diffs and whole commands on approval cards, built in Rust (D37); the whole preview and session requests in the command window (motion-designer). Manual checklist pending. |
| 2026-11-01 | Stage 2.8c built: questions and plans from sessions and agent runs, answered in the island and the command window, never with a bare allow (D38); spiked first with Claude Code 2.1.314. Manual checklist pending. |
| 2026-11-02 | Stage 2.8d built: the live step line and to-do progress in the pill and panel, from a background `PreToolUse` hook and stream-json (D39). Stage 2.8e spiked: a running stream-json run accepts `set_permission_mode`, and per-run ask rules keep filesystem shell commands asking under Accept edits. Manual checklist pending. |
| 2026-11-02 | Stage 2.8e built: the author reversed "every step asks" (D40). Runs use Claude Code's own modes, Auto by default, switched mid-run from a chip (`set_permission_mode`) or for the next message; routines get a mode; desktop tools follow Auto. Manual checklist pending. |
| 2026-11-02 | Stage 2.8e signed off by the author. Kick-off prompt for 2.8f (`prompts/phase-2.8f-connect.md`) written so it starts in a new session. |
| 2026-11-02 | Stage 2.8f built: Connect's three states and an Update dialog, usage limits through a chained status line (D41; the author's HUD unchanged through Cay), an optional "needs you" sound; the Windows Terminal tab spike said no. Manual checklist pending. |
| 2026-11-02 | Stage 2.8f signed off by the author: **Phase 2.8 complete.** Release v0.2 (4a-5) deferred to the end of all phases at the author's call. Phase 6 (Google Calendar and Gmail) kick-off prompt written. |
| 2026-11-02 | Phase 6 research and design agreed (D42): Cay's own OAuth client (In Production, unverified until release), no iCal fallback, no sync tokens, WinHTTP and Credential Manager instead of new crates. Stage 6a built: the `google/` module, the `google-watcher` thread, `google:state` / `calendar:upcoming`, tray Connect/Disconnect Google. Checked with fixtures, a live WinHTTP test without an account and a debug Cay (C14: the drive filled mid-build); the manual check waits for the author's Google client. |
| 2026-11-02 | Stage 6a signed off by the author. Google wouldn't publish the External project without a home page and privacy policy, so it runs in Testing with the author as a test user (7-day sign-ins) until those exist before release. |
| 2026-11-02 | Stage 6b built (D43): the calendar mark in the pill (Google's icon once the author adds it), "starting soon" with Join, the panel's Calendar section (motion-designer, renders at 100/120/150 %). Manual checklist pending. |
| 2026-11-02 | Command window restyled as Cay (D45): ended runs removable from the sidebar (`agent_remove`, "Clear ended"), lighter controls. |
| 2026-11-03 | Stage 6b signed off by the author. |
| 2026-11-03 | Stage 6c built (D46): Calendar, Tasks and Gmail tools in Cay's MCP server, Gmail asked in Cay and opt-in, mail marked untrusted, desktop actions asked again after mail; `claudeConnectors` dropped after testing. The drive filled twice mid-build (C14 again): 4.5 GB of `.rcgu.o` leftovers from crashed builds in `target\debug\deps`. Manual checklist pending. |
| 2026-11-03 | Stage 6c signed off by the author (all 11 manual checks passed). Phase 6 is complete; next is 2.9 (other agents and open models). |
| 2026-11-03 | Phase 2.9 research and design agreed (D47): the full `claude/` → `sessions/` rename, Codex sessions through hooks with a minimum version, ACP runs (Goose first, tested on another machine), Ask only plus a watchdog, Calendar and Tasks kept on for ACP runs. |
| 2026-11-03 | Stage 2.9a built: Codex sessions through the same forwarder (`--agent codex`), Codex Connect in the tray (unquoted 8.3 path, Codex ≥ 0.166.0, `/hooks` trust), `apply_patch` diffs, the agent tag and the "Agents" heading (motion-designer). Codex 0.166's hooks checked in a scratch `CODEX_HOME` and its TUI (C15). All 12 manual checks passed (the author). OpenCode spike next. |
| 2026-11-03 | OpenCode spike (1.19.5): `permission.ask` is never called, but a plugin can answer through OpenCode's own client. The author chose to build OpenCode sessions in 2.9a: Cay's `cay.js` plugin feeds the same forwarder (`--agent opencode`, tools renamed in Rust), answers permissions through the client, fails open (checked with Cay closed). Manual checks pending. |
| 2026-11-04 | OpenCode's manual checks passed (the author): **stage 2.9a signed off**, Codex and OpenCode sessions in the island. Next: 2.9b (ACP runs). |
| 2026-11-04 | 2.9b started. The author reversed D47's "Ask only": other agents' runs get Claude Code's four modes, Auto by default, desktop actions following the mode (D48); OpenCode first, then Codex, then Goose. |
| 2026-11-04 | 2.9b step 1: ACP runs on OpenCode (`acp.rs`, `backend.rs`, `run::spawn_command`, `mcp::Offer`), checked live through Cay in Ask and Auto; 265 Rust tests. A free model stalled one run until Stop (no inactivity limit yet). |
| 2026-11-04 | 2.9b step 2: Codex runs through `@agentclientprotocol/codex-acp` (Auto = its "Auto review"), checked live in Ask and Auto. The first message now waits for the mode to be set; a declined step that cancels Codex's turn ends it as done. 268 Rust tests. |
| 2026-11-04 | 2.9b step 3: both ACP agents call Cay's own tools (`list_windows`, Auto, no card); Codex's MCP calls and Auto-review steps now read as `mcp__cay__…` and "Review"; ACP runs silent for 10 minutes are stopped. Two of my background dev sessions left an orphaned Vite and Cay behind, and an `agent-test quit` with no Cay running started one (CLAUDE.md's gotcha); cleaned up. 269 Rust tests. |
| 2026-11-04 | **Stage 2.9b signed off** by the author (OpenCode and Codex runs); Goose deferred to the other machine. 2.9c started. |
| 2026-11-04 | Stage 2.9c built: `agent.defaultBackend` and `agent.backends` in settings.json, routines with an agent, `agent_backends` (installed agents and their models: `opencode models`, Codex's model cache), the command window's agent and model picker with turns and budget for Claude Code only, agent tags on runs and routines (motion-designer), and the README's section on other agents. Manual checks pending. |
| 2026-11-04 | Stage 2.9c signed off by the author (all 6 manual checks passed): **Phase 2.9 complete**, Goose deferred to the other machine. Phase 3 (memory-heavy apps) kick-off prompt written. |
| 2026-11-04 | Phase 3 research and design drafted by the daily routine and approved by the author (D49). Stage 3a built on branch `self/phase-3-memory-heavy` in a separate worktree (the live `pnpm tauri dev` would have restarted on Rust edits): kernel snapshot, grouping, the panel-open-only watcher, `processes:heavy`, the Memory section (motion-designer). Manual check pending. |
| 2026-11-04 | **Stage 3a signed off** by the author (all 6 manual checks passed). In the live log the first refresh took ~577 ms (the exe-path cache filling: one `OpenProcess` per process), every later one 14–60 ms for ~340 processes, and none while the panel was closed. 3b (End Task) next. |
| 2026-11-04 | Stage 3b.1 built (D50): `processes/end.rs` and `end_process_group`, the confirm box in Rust, two checks by creation time, `WM_CLOSE` then `TerminateProcess` children first, `prompted` for an app asking to save. 290 Rust tests; a live test ended a `cmd` → `ping` tree it started. Built with `CARGO_INCREMENTAL=0` and `cargo test --lib` (8.4 GB free, C14). |
| 2026-11-03 | Stage 2.9a, step 1: the rename (`claude/` → `sessions/`, session IPC names, `features/sessions/`, `--font-agent`, `data-agent`), no behaviour change; 230 Rust tests, clippy and typecheck clean. Codex updated to 0.166.0. |

## 7. Results & metrics

Measured on 2026-10-28, Phase 1 release build (`opt-level="s"`, fat LTO, `codegen-units=1`, stripped, `panic="abort"`):

| Metric | Value |
|---|---|
| Installer download (NSIS, `TopBar_0.1.0_x64-setup.exe`) | **1.47 MB** |
| Installed app (`topbar.exe`, frontend embedded) | **4.13 MB** |
| Frontend bundle | 362 kB JS (116 kB gzipped), 2.4 kB CSS |
| RAM, idle collapsed (topbar.exe + its WebView2 processes, private bytes) | ~136 MB (topbar.exe itself: 6 MB) |
| RAM, expanded with apps and icons loaded | ~165 MB |
| Idle CPU (60 Hz cursor poll + 4 Hz fullscreen check) | ~1.4 % of one core |
| Hover-to-open | 150 ms dwell (by design) + one 16 ms poll tick |

*Takeaway:* the download is tiny because Windows 11 ships the WebView2 runtime. Nearly all the
memory is WebView2 (7 processes: browser, GPU, renderer, utilities); the Rust side is 6 MB. If memory
becomes a selling point, the levers are WebView2 browser arguments and a lighter frontend bundle, not the Rust code.

## 8. What I'd do differently

*(Fill in as lessons emerge.)*
