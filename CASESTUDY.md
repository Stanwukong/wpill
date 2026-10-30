# Case Study: TopBar, a Dynamic Island for Windows 11

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
