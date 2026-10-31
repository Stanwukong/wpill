# PLAN.md

> Keep this file current: see "Keep the docs current" in [CLAUDE.md](CLAUDE.md).

## Current status

**Phase 1 is complete (2026-10-28): built, smoke-tested, and all 11 manual tests passed.**
**Phase 1.5 is complete (2026-10-30): tray menu, launch at startup and single instance, with all manual checks passed.**
**Phase 2 is complete (2026-10-30): media card, pill art + equalizer and session switcher, with all 10 manual checks passed.**
After sign-off, the island also took its colors from the album art (D13).
**Phase 2.5 is complete (2026-10-30): Live Activities for volume, brightness, Bluetooth audio, power, Caps Lock and
screenshots, with all 12 manual checks passed.** The charging activity also got an orbiting green outline (D18).
**Phase 2.6 is complete (2026-10-30): Claude Code activity through hooks (spinner, done / needs-input activities with
an orange edge, panel section, click to focus, tray Connect/Disconnect), with all 12 manual checks passed.**
**Phase 2.7a–b are complete (2026-10-30): headless agent runs from a command window (Win+Alt+A), progress in the
island, approvals (Allow / Deny) in the island, Cascadia Mono for Claude content, with all 12 manual checks passed.**
**Phase 2.7c is complete (2026-10-30): read-only desktop tools (`list_windows`,
`focused_window`, `screenshot`, `ui_tree`), each approved in the island with an eye in the `read` badge, and a glow on
the screen's edges while Claude looks, with all 12 manual checks passed.**
**Phase 2.7d is complete (2026-10-31): seven desktop actions (`focus_window`, `invoke`, `set_value`, `click`,
`type_text`, `press_keys`, `launch`), UI Automation first with `[n]` refs from `ui_tree`, each approved on its own
and never for the run, stopped by any input of yours, with all 12 manual checks passed.**
See [Phase 2.7](#phase-27-local-automation-agent). **Next: stage 2.7e (routines)**, kick-off prompt
[prompts/phase-2.7e-routines.md](prompts/phase-2.7e-routines.md).
Tauri 2.12.5 · `windows` 0.62 · React 19 · Framer Motion 14 · Vite 8 · TypeScript 7.

Verified automatically (by driving the cursor and inspecting the live HWND from outside):
- Window is 680×440, top-center, with `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST`, and
  click-through while collapsed. The no-activate bits survive click-through toggles.
- Hot zone → expand after ~150 ms; staying over the panel keeps it open; leaving collapses it
  after 250 ms; 5/5 runs including the first open after launch.
- The foreground window never changed during any of this (no focus stealing).
- An externally hidden island is re-shown within 250 ms without taking focus.
- Panel renders the clock, date and running apps with icons (Win32 + UWP apps, e.g. WhatsApp).
- `cargo test` (5 tests), clippy, `tsc` and `vite build` are clean.

Verified by hand (manual checklist, all pass): app activation (including minimized and elevated
windows), closed-window error, fullscreen hiding without hiding for maximized windows, display
scaling and text size, and repeated relaunches.

---

## Environment

Checked 2026-10-28 on Windows 11 Pro 23H2 (build 22631). **All requirements are now installed**
(rustc 1.99.1 msvc, VS Build Tools 2022, Windows SDK 10.0.26100). The table below is the original audit.

| Requirement | Status | Action |
|---|---|---|
| Rust toolchain (rustup, cargo, msvc target) | ❌ missing | `winget install --id Rustlang.Rustup`, then `rustup default stable-msvc` |
| MSVC C++ build tools + Windows SDK | ⚠️ VS Build Tools 2019 present, no VC++ toolset | Install VS 2022 Build Tools with the C++ workload (command below), or add "Desktop development with C++" to the 2019 install via Visual Studio Installer |
| Windows SDK | ✅ 10.0.19041 (the C++ workload adds a newer Win11 SDK) | none |
| Node.js | ✅ v24.21.0 | none |
| pnpm / npm | ✅ pnpm 10.34.8, npm 10.9.0 | none |
| Tauri CLI | ❌ not installed globally | No global install needed. It's added as the `@tauri-apps/cli` dev dependency at scaffold time |
| WebView2 Runtime | ✅ 155.0.4283.62 | none |
| Git | ✅ 2.44 | none |

```powershell
# 1. MSVC C++ build tools (includes the Windows 11 SDK); takes ~5-10 min
winget install --id Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"

# 2. Rust
winget install --id Rustlang.Rustup
# open a NEW terminal so PATH picks up cargo, then:
rustup default stable-msvc
rustc --version; cargo --version

# 3. Re-verify everything
.\scripts\check-env.ps1
```

---

## Tauri 2 API notes (verified against docs for tauri 2.9.x)

| Need | API |
|---|---|
| Transparent window | `transparent: true` in `tauri.conf.json` / `WebviewWindowBuilder::transparent(true)`. The page itself must also have a transparent `html, body` background. |
| Frameless | `decorations: false`. Set `shadow: false` too, or Windows 11 draws a shadow and rounded frame around the invisible window. |
| Always on top | `alwaysOnTop: true` / `.always_on_top(true)` |
| Hidden from taskbar | `skipTaskbar: true` / `.skip_taskbar(true)`. We also set `WS_EX_TOOLWINDOW` so it stays out of Alt+Tab. |
| No focus stealing | `.focusable(false)` + `.focused(false)`, plus `WS_EX_NOACTIVATE` applied manually via `SetWindowLongPtrW(GWL_EXSTYLE)` to be sure |
| Click-through | `WebviewWindow::set_ignore_cursor_events(bool)` |
| Native handle | `WebviewWindow::hwnd() -> Result<HWND>` (Windows only) |
| Position/size | `set_position`, `set_size`, `current_monitor`/`primary_monitor` (physical px plus `scale_factor`) |
| Rust → JS | `use tauri::Emitter; app.emit("name", payload)` with a `Clone + Serialize` payload |
| JS → Rust | `#[tauri::command]` (async where it blocks), registered in `invoke_handler(generate_handler![...])`, called with `invoke()` from `@tauri-apps/api/core` |
| Permissions | `src-tauri/capabilities/default.json` needs `core:event:default` and `core:window:default`. Custom commands are allowed by default (they can be restricted with `AppManifest::commands` in `build.rs`). |
| Win32/WinRT from Rust | Call the `windows` crate directly inside commands or a background thread. Enable only the `Win32_*` / `Media_*` features we use. Blocking WinRT calls go in `async` commands or `spawn_blocking`. |

---

## Architecture

```
            ┌──────────────── Rust (src-tauri) ─────────────────┐
cursor ───▶ │ hover poller (60 Hz thread)                       │
            │   state machine: Collapsed → Arming(150ms)        │──emit──▶ island:hover-enter / -leave
            │                  → Expanded → Leaving(250ms)      │
            │   toggles set_ignore_cursor_events                │
            │ fullscreen watcher (4 Hz, same thread)            │──emit──▶ island:visibility
            │ commands: list_windows, activate_window,          │◀─invoke── React
            │           set_panel_rect, request_collapse        │
            └───────────────────────────────────────────────────┘
React: <Island> (Framer Motion pill ⇄ panel) → <AppsList> …later <MediaCard>, <ProcessList>
```

**Hover flow.**
1. *Collapsed:* the window ignores the cursor. The poller checks `GetCursorPos` against the
   hot zone (≈ 220×6 logical px at the top center of the primary monitor).
2. *Arming:* the cursor stays in the zone ≥ 150 ms (leaving cancels). Then emit
   `hover-enter`, set ignore = false, state = Expanded.
3. *Expanded:* React measures the panel and calls `set_panel_rect` with its width and bottom edge
   in **physical px**. The poller tests the cursor against that rect plus an 8 px margin. Outside the
   rect it goes to *Leaving* (already click-through); back inside cancels. After ≥ 250 ms
   outside it emits `hover-leave` and returns to Collapsed.
4. The window is sized to the maximum panel, so transparent margins exist. Accepting
   cursor events only in the Expanded state (cursor inside the panel) keeps them click-through.
5. *Suppressed:* after `request_collapse` (e.g. an app was clicked), the island won't re-arm
   until the cursor leaves the hot zone.
6. Every 250 ms the poller checks for a fullscreen foreground app. If it finds one, it hides the island;
   otherwise it makes sure the island is visible.

---

## Folder structure (proposed)

```
wpill/
├─ CLAUDE.md  PLAN.md  CASESTUDY.md
├─ scripts/check-env.ps1
├─ package.json  pnpm-lock.yaml  vite.config.ts  tsconfig.json  index.html
├─ src/                          # React
│  ├─ main.tsx  App.tsx
│  ├─ island/
│  │  ├─ Island.tsx              # pill ⇄ panel container (Framer Motion layout)
│  │  ├─ useIslandState.ts       # subscribes to island:* events
│  │  └─ motion.ts               # spring/timing constants
│  ├─ features/
│  │  ├─ apps/                   # Phase 1: AppsList.tsx, useRunningApps.ts
│  │  ├─ media/                  # Phase 2: MediaCard, PillMedia, useMedia, useProgress
│  │  └─ processes/              # Phase 3
│  ├─ lib/
│  │  ├─ ipc.ts                  # typed invoke/listen wrappers (single source of names)
│  │  └─ types.ts                # mirrors Rust DTOs
│  └─ styles/global.css          # transparent html/body, tokens
└─ src-tauri/
   ├─ Cargo.toml  build.rs  tauri.conf.json
   ├─ capabilities/default.json
   ├─ icons/
   └─ src/
      ├─ main.rs                 # calls lib::run()
      ├─ lib.rs                  # builder, setup(), command registration
      ├─ ipc.rs                  # event-name consts + payload structs
      ├─ error.rs                # AppError (thiserror + Serialize)
      ├─ window/mod.rs           # create/position island, apply ex-styles
      ├─ hover/mod.rs            # poller thread + state machine (pure logic unit-tested)
      ├─ fullscreen.rs           # foreground-covers-monitor + SHQueryUserNotificationState
      ├─ apps/
      │  ├─ mod.rs               # commands
      │  ├─ enumerate.rs         # EnumWindows + Alt+Tab filter
      │  ├─ icon.rs              # HICON → PNG data URL, cached per exe path
      │  └─ activate.rs          # SetForegroundWindow fallback chain
      ├─ media/                  # Phase 2: mod (state + commands), watcher (thread + subscriptions),
      │                          #   snapshot (session → DTO), artwork (thumbnail → data URL), logic (pure, tested)
      ├─ activity/               # Phase 2.5: mod (state, watcher harness, commands), queue (pure, tested),
      │                          #   settings (switches), audio, brightness, power, screenshots (one thread each), drag
      ├─ processes/              # Phase 3
      └─ tray.rs settings.rs thumbnails/   # Phase 4
```

---

## IPC contract

### Phase 1

| Kind | Name | Payload / signature | Notes |
|---|---|---|---|
| event | `island:hover-enter` | `{}` | Rust → React: expand |
| event | `island:hover-leave` | `{}` | Rust → React: collapse |
| event | `island:visibility` | `{ hidden: boolean, reason: "fullscreen" \| null }` | Fullscreen app in foreground |
| command | `list_windows` | `() → WindowInfo[]` | `WindowInfo { hwnd: number, title, exeName: string \| null, icon: string \| null }` (icon = PNG data URL). Z-order, so most recently used first. Called on each expand. |
| command | `activate_window` | `(hwnd: number) → void` | Fallback chain; collapses the island on success. Errors: `{ kind: "windowGone" \| "foregroundDenied" \| …, message }` |
| command | `set_panel_rect` | `(width, bottom) → void`, **physical px** | Panel width, and its bottom edge from the window top. Physical because Windows text-size zoom changes the CSS↔physical ratio. |
| command | `request_collapse` | `() → void` | E.g. after clicking an app |
| command | `get_island_state` | `() → { expanded, hidden }` | Initial sync after a webview reload |

### Phase 2

| Kind | Name | Payload / signature | Notes |
|---|---|---|---|
| event | `media:state` | `{ sessions: MediaSession[] }`, most recently active first | Sent only when something React can't interpolate changed (CASESTUDY D12). Replaces the single-session shape reserved in Phase 0, to support the switcher. |
| command | `get_media_state` | `() → { sessions }` | Initial sync after a webview reload |
| command | `media_play_pause` | `(sessionId) → void` | Pause if playing, else play (not every app enables the toggle). |
| command | `media_next` / `media_prev` | `(sessionId) → void` | |
| command | `media_seek` | `(sessionId, positionMs) → void` | Relative to the timeline's start. |

`MediaSession { id, appName, title, artist, album, artwork: dataURL|null, status: "playing"|"paused"|"stopped"|"changing",
positionMs, durationMs|null, updatedAtMs, playbackRate, canPlay, canPause, canNext, canPrev, canSeek }`.
`id` is ours (a counter), not a Windows id. `positionMs` is the position at `updatedAtMs` (Unix ms, from GSMTC's
`LastUpdatedTime`); React interpolates `positionMs + (now − updatedAtMs) × playbackRate` while playing.
Command errors: `mediaSessionGone` (session ended), `mediaRejected` (the app returned `false`).

### Phase 2.5

| Kind | Name | Payload / signature | Notes |
|---|---|---|---|
| event | `activity:show` | `{ id, kind, payload, durationMs }` | Re-sent with the same `id` when a burst updates it in place. `durationMs` is the time left; Rust times it. |
| event | `activity:hide` | `{ id }` | Timed out, replaced, or the panel opened over it. Needed because Rust holds hovered activities. |
| command | `set_panel_rect` | `(width, bottom, mode: "panel" \| "activity")` | Gained `mode`; Rust ignores reports for the shape it isn't showing. |
| command | `get_island_state` | `() → { expanded, hidden, activity }` | Gained `activity` (the payload on screen, or `null`). |
| command | `activity_drag_screenshot` | `(id) → void` | Sync (main thread); call while the button is down. Resolves after the drop. |
| command | `activity_open_screenshot` | `(id) → void` | Opens in the default viewer and ends the activity. Error `activityGone` if `id` isn't showing. |

Payloads by `kind`: `volume { level, muted, device }`, `brightness { level }`,
`bluetooth { name, connected, battery|null }`, `power { event: "pluggedIn"|"unplugged"|"low", level }`,
`capsLock { on }`, `screenshot { fileName, thumbnail (PNG data URL, ≤256 px), width, height }`. The screenshot's path
stays in Rust and is looked up by `id`.

### Phase 2.6

| Kind | Name | Payload / signature | Notes |
|---|---|---|---|
| event | `activity:show` | `durationMs: number \| null` | `null` for a sticky notice (needs input): shown until Rust withdraws it with `activity:hide`. |
| event | `claude:sessions` | `{ sessions: ClaudeSession[] }`, most recently updated first | `ClaudeSession { sessionId, project, status: "idle" \| "working" \| "done" \| "needsInput", updatedAtMs }`. `sessionId` is ours (a counter); Claude's id stays in Rust. |
| command | `get_claude_sessions` | `() → { sessions }` | Initial sync after a webview reload. |
| command | `claude_focus_session` | `(sessionId) → void` | Sync (main thread). Acknowledges the session's activity, brings its terminal/editor forward, collapses. Errors: `claudeSessionGone`, `claudeWindowNotFound`, `windowGone`, `foregroundDenied`. |
| command | `claude_hooks_status` / `claude_hooks_install` / `claude_hooks_remove` | `() → { installed }` / `() → void` | For Phase 4b's settings window; the tray calls the same code. Error `claudeSettings` with a readable message. |

Activity kinds: `claudeDone { sessionId, project, failed }` (`failed`: `StopFailure`, an API error) and
`claudeNeedsInput { sessionId, project }`. Both are interactive (take clicks while hovered).

### Phase 2.7 (a–b)

| Kind | Name | Payload / signature | Notes |
|---|---|---|---|
| window | `command` | `command.html` → `src/command/` | Focusable, frameless, created on first use, hidden on Esc. The island stays unfocusable. |
| command | `agent_start` | `({ request: { prompt, cwd, model?, maxTurns?, maxBudgetUsd?, claudePath? } }) → { runId }` | Async. Folder normalised to its long path. Errors `agentInvalid`, `agentNoClaude`, `agentStart`. |
| command | `agent_send` | `(runId, text) → void` | Queued into the running turn, or resumes a finished run (`--resume`). |
| command | `agent_stop` | `(runId) → void` | Kills the run's Job Object (the whole tree). |
| command | `agent_runs` / `agent_transcript` | `() → { runs }` / `(runId) → AgentEntry[]` | Initial sync; the transcript keeps the last 400 entries. |
| command | `agent_decide` | `(requestId, decision: "allow" \| "deny", remember: "once" \| "run")` | Error `agentApprovalGone` if it was already answered or expired. |
| command | `agent_options` | `() → AgentOptions` | Installs (newest first), default install, recent folders, defaults, hotkey and `hotkeyError`. |
| command | `agent_pick_folder` | `() → string \| null` | Sync (main thread): the shell's folder picker, owned by the command window. |
| command | `command_window_show` / `command_window_hide` | `(runId?)` / `()` | Show focuses it and sends `command:focus-run`. |
| event | `agent:runs` | `{ runs: AgentRun[] }`, newest first | Replaces the reserved single-run `agent:run`. `AgentRun { runId, project, cwd, prompt, status, reason, step, turns, costUsd, model, pending: AgentApprovalActivity[], error, startedAtMs, updatedAtMs }`. |
| event | `agent:output` | `{ runId, entry: AgentEntry }` | Command window only. `AgentEntry = { atMs, kind: "user" \| "text" \| "tool" \| "toolResult" \| "approval" \| "decision" \| "result" \| "error", … }`. |
| event | `command:focus-run` | `runId` | Command window only. |

`status`: `starting | working | waiting | done | failed | stopped`; `reason`: `completed | maxTurns | maxBudget | error |
stopped`. Activity kinds: `agentApproval { requestId, runId, project, tool, summary, risk: "read" | "change" | "input",
rememberAs: string | null }` (sticky, interactive) and `agentFinished { runId, project, status, reason }` (5 s notice,
not shown after Stop). `rememberAs` ("git", "edits") replaces the planned `canRemember` flag so the island can say what
"for this run" covers.

### Phase 2.7c

| Kind | Name | Payload / signature | Notes |
|---|---|---|---|
| activity field | `agentApproval.desktop` | `boolean` | One of TopBar's desktop looks: the risk badge shows an eye. `tool` reads `desktop · screenshot`; `rememberAs` is `window lists`, `screenshots` or `reading UIs`. |
| MCP tool | `list_windows` | `{}` → text `{ windows: [{ hwnd, app, title, bounds, minimized, focused }] }` | Alt+Tab windows front to back, physical px, never TopBar's own. Minimized windows have empty bounds. |
| MCP tool | `focused_window` | `{}` → text `{ window: … \| null, note? }` | `null` + note when TopBar's own window is in front. |
| MCP tool | `screenshot` | `{ hwnd? }` → image block + text `{ source, width, height, origin, scale, format }` | No hwnd: the primary monitor. ≤1568 px long edge, ≤500 KB (PNG, else JPEG q80/q60). |
| MCP tool | `ui_tree` | `{ hwnd, maxDepth? (12, ≤25), maxNodes? (400, ≤1000) }` → text outline | `button "Save" #id @x,y wxh flags`, ≤60 KB, says when it was cut. Password fields: no value. |
| errors | (all four) | `isError` text `code: advice` | `windowGone`, `topbarWindow`, `notOnThisDesktop`, `minimized`, `notResponding`, `elevated`, `screenLocked`, `failed`. |

### Phase 2.7d

| Kind | Name | Payload / signature | Notes |
|---|---|---|---|
| activity | `agentActing` | `{ runId, project, step, phase: "waiting" \| "acting" }` | Sticky, not interactive, shown before approvals; one key (`acting`), so the phase updates in place. Not for `launch`. |
| activity field | `agentApproval.detail` | `string \| null` | The full text `type_text` / `set_value` would enter, for the command window's card. `desktop` + risk `input` = an action (pointer glyph). |
| MCP tool | `ui_tree` (changed) | each line starts with `[n]` | The ref the actions take as `element`; valid until the next `ui_tree` of that window (≤10 min), per run. |
| MCP tool | `focus_window` | `{ hwnd }` | `apps::bring_to_front` on the main thread. |
| MCP tool | `invoke` | `{ hwnd, element, action?: invoke \| toggle \| select \| expand \| collapse \| scrollIntoView \| focus }` | UIA patterns; no input, window may stay behind. |
| MCP tool | `set_value` | `{ hwnd, element, value }` | Value pattern, ≤1000 chars; reply says whether the field holds it. |
| MCP tool | `click` | `{ hwnd, element }` or `{ hwnd, x, y, from?: screen \| screenshot }`, `button?: left \| right \| double` | `SendInput`; brings the window forward; the point must be on it. |
| MCP tool | `type_text` | `{ hwnd, text }` | ≤500 chars, real keys from the window's layout (Unicode packets only for what it can't type), paced. |
| MCP tool | `press_keys` | `{ hwnd, keys: string[] }` | ≤5 chords; Win-key chords, Alt+Tab, Alt/Ctrl+Esc, Ctrl+Shift+Esc refused. |
| MCP tool | `launch` | `{ url }` or `{ app }` | https/http/`ms-settings:` links; Start-menu apps by name. |
| reply | (all seven) | text "Invoked button "Save" in Notepad · notes.txt. In front now: …" | Errors `isError` `code: advice`: `windowGone`, `elementGone`, `staleView`, `topbarWindow`, `notOnThisDesktop`, `notResponding`, `elevated`, `screenLocked`, `notForeground`, `covered`, `disabled`, `unsupported`, `passwordField`, `refused`, `notFound`, `aborted`, `failed`. |
| MCP notification | `notifications/cancelled` | `{ requestId }` | Withdraws that call's approval or aborts its running action. |

### Reserved for later phases (names fixed now so they slot in)

| Phase | Kind | Name |
|---|---|---|
| 3 | command | `list_heavy_processes(limit) → ProcessGroup[]` (`{ name, pids, memoryBytes, icon, protected }`) |
| 3 | command | `end_process_group(name, pids) → EndResult` (per pid: `ok \| accessDenied \| protected \| notFound`) |
| 4a | event | `monitor:changed` |
| 4b | event | `settings:changed` |
| 4b | command | `get_settings`, `set_settings` |
| 5 | command | timer: `timer_start(ms)`, `timer_cancel`; event `timer:tick` |
| spike | command | `show_thumbnail(hwnd, rect)`, `hide_thumbnail` |

---

## Phase 1: core island + running apps

- [x] Scaffold Tauri 2 + React-TS + Vite with pnpm; add framer-motion
- [x] Window: transparent, frameless, shadowless, always-on-top, skip taskbar, unfocusable,
      created hidden, 680×440 logical, top-center of primary monitor
- [x] Apply `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW` and start click-through
- [x] `ipc.rs` / `ipc.ts` / `error.rs` / `types.ts`
- [x] Hover poller thread + state machine (5 unit tests in `hover/logic.rs`)
- [x] `set_panel_rect`, `request_collapse`, `get_island_state`
- [x] React `<Island>`: clock pill ⇄ panel spring animation; transparent background
- [x] `list_windows`: Alt+Tab filter, title, exe name (UWP resolved through ApplicationFrameHost), icon (cached)
- [x] `activate_window`: fallback chain, `IsWindow` revalidation, collapse afterwards
- [x] Fullscreen watcher → hide the window, emit `island:visibility`
- [x] Manual test pass (checklist below): all 11 passed, 2026-10-28
- [x] Debug transition logging: kept at `debug` level (off by default; enable with `RUST_LOG`)

**Manual test checklist** (rerun before each release)
Hot zone = a 200×10 px strip at the very top center (move the cursor to the top edge).
1. Pill with the clock is visible at top center. It's not in the taskbar or Alt+Tab.
2. Click-through while collapsed: click a browser tab or title-bar spot directly under the pill.
3. Brushing quickly across the top edge doesn't open; resting ~150 ms does.
4. Focus: type in Notepad, open the island, move around it, and keep typing. Text still goes to Notepad.
5. Leaving the panel collapses it after ~¼ s. While it's open, the transparent area beside the panel is click-through.
6. The list matches Alt+Tab: no tool windows, nothing from other virtual desktops, and UWP apps show with icons.
7. Click an app tile → that app comes to the front and the island collapses. Repeat with a **minimized** app and an **elevated** app (e.g. an admin terminal).
8. Close a window, then click its tile from a list opened before closing → "That window has closed." and the list refreshes.
9. Fullscreen: YouTube fullscreen, F11 in a browser, a game, and PowerPoint slideshow. The island disappears; exiting restores it. A *maximized* window must **not** hide it.
10. Display scaling 125/150 % and text size 100/120/150 %: the panel fits, the hot zone and leave edges line up with what's drawn. (Scaling is read at startup; restart after changing it until Phase 4.)
11. Kill and relaunch the app several times: the pill always appears.

## Roadmap

Reprioritized 2026-10-28 after Phase 1 (see CASESTUDY D9). Phase numbers keep their original
meaning; new phases slot in between.

| Order | Phase | Scope | Why here |
|---|---|---|---|
| 1 | **1.5 Shell basics** | Tray icon, quit, launch at startup | ~1 hour. Testers need to quit and autostart before anything is shared. |
| 2 | **2 Media card** | Now playing + controls, media state in the collapsed pill | Makes the island feel alive; the planned next phase. |
| 3 | **2.5 Live Activities** | Island auto-expands for system events | The defining Dynamic Island behavior; reuses Phase 2's pill plumbing. |
| 4 | **2.6 Claude Code activity** | Island shows when a local Claude Code session finishes or needs a decision | Added 2026-10-30 (D17). Small once 2.5's activity queue exists; high daily value while working with Claude Code. |
| 5 | **2.7 Local automation agent** | Type an instruction; TopBar runs a headless Claude Code session, shows progress in the island, asks for approvals there, and gives Claude desktop tools through its own MCP server | Added 2026-10-30 (D21). Builds directly on 2.6's pipe, sessions and activities. Staged so each stage is useful on its own. |
| 6 | **4a Release readiness** | Multi-monitor + per-monitor DPI, code signing, v0.2 release | Docking/undocking breaks placement today; unsigned installers trigger SmartScreen. |
| 7 | **3 Memory-heavy apps** | As specified, behind a settings toggle | Power-user tool with the riskiest action (End Task); opt-in only. |
| 8 | **5 Utilities** | Timer/stopwatch/focus, drop shelf + clipboard | High daily value, low platform risk. |
| 9 | **Spikes** | Notifications mirror, calendar peek, live window previews | Uncertain feasibility or needs sign-in; prototype before committing. |

### Phase 1.5: shell basics (complete 2026-10-30)
- [x] Tray icon (`tauri` `tray-icon` feature, `src-tauri/src/tray.rs`). The menu (left or right click) shows a
      version label ("TopBar 0.1.0", in place of a separate About), a **Launch at startup** checkbox, and **Quit TopBar**
- [x] Launch at startup: per-user `HKCU\…\Run` entry named `TopBar`, no admin. Written by our own
      `autostart.rs` with a **quoted** path. `tauri-plugin-autostart` was dropped: its unquoted path
      didn't show in Task Manager on this machine (CASESTUDY C5). An old unquoted entry for the same exe
      is repaired on launch.
- [x] Single instance via `tauri-plugin-single-instance` 2.5: a second launch exits immediately (verified)
- [x] ~~Persist with `tauri-plugin-store`~~: not needed. The Run registry entry *is* the persisted
      state, and the checkbox re-reads it after every toggle (CASESTUDY D10)
- [x] Manual check (below): all steps passed on the installer that includes `autostart.rs` (2026-10-30)

**Manual check (Phase 1.5)**
1. The TopBar icon is in the system tray (it may be in the ^ overflow; drag it out to pin it).
2. Clicking it shows "TopBar 0.1.0" (greyed out), "Launch at startup" and "Quit TopBar".
3. **Quit TopBar** closes the island and the tray icon.
4. Tick **Launch at startup** from the *installed release build*. Reopen Task Manager → Startup apps:
   TopBar is listed and Enabled. Sign out and back in: the island appears.
   Untick it: TopBar disappears from Startup apps, and `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
   has no `TopBar` value.
4b. Disable TopBar *in Task Manager*, then open the tray menu: the checkbox shows unticked. Ticking it re-enables it.
5. Launching TopBar again while it's running doesn't create a second island.

### Phase 2: media card (complete 2026-10-30)
- [x] Rust `media/`: a `media-watcher` thread (MTA) owns the GSMTC manager and subscribes to `CurrentSessionChanged`,
      `SessionsChanged`, and per session `MediaPropertiesChanged`, `PlaybackInfoChanged`, `TimelinePropertiesChanged`.
      Handlers only forward a message; bursts are batched (50 ms) and `media:state` is emitted only on real changes
      (CASESTUDY D11, D12). A 5 s resync runs while something plays, in case an app goes quiet.
- [x] Album art: the thumbnail stream is passed through as a data URL (type sniffed from the bytes), not re-encoded;
      over 1.5 MB is skipped. Cached per (session, track), 16 entries; misses are retried on the next 3 events.
- [x] Controls: `media_play_pause`, `media_next`, `media_prev`, `media_seek`; buttons are disabled where the session doesn't allow them
- [x] Panel card: art, title, artist · app, controls, progress bar interpolated on `requestAnimationFrame` while the panel is open; click to seek
- [x] **Collapsed pill shows media state:** 20 px art + CSS-only equalizer beside the clock while playing
- [x] Multiple sessions: most recently active first (started playing, new track, new session, or Windows' current session),
      with app chips as a switcher. A manual pick lasts until a different session becomes most recent.
- [x] Verified from outside (2026-10-30): Spotify session detected at startup with art, title and artist; pressing play in
      Spotify produced one `media:state` and the pill switched to art + equalizer; no extra emits during steady playback
- [x] Album-art theme (2026-10-30, after sign-off): the island's background is a dark gradient in the art's dominant
      hue, and the equalizer and playing dot use a lighter accent of it. Colors are computed in the webview from the data
      URL and fade over 700 ms; plain black when there's no art (CASESTUDY D13)
- [x] Manual check (below): all 10 pass, 2026-10-30 (Spotify, YouTube in a browser, two sessions, nothing playing, text sizes)

**Manual check (Phase 2)** (run with `$env:RUST_LOG='info,topbar_lib=debug'` to see `media:state` lines)
1. **Nothing playing** (close Spotify, no media tabs): the pill is the plain clock; the panel has no media card.
2. **Spotify:** play a song. Within a second the pill shows its art, the clock and a moving equalizer. The panel card
   shows art, title, "Artist · Spotify", and the bar advances smoothly with the elapsed time.
3. **Pause/skip from the island:** pause → Spotify pauses, the art and equalizer leave the pill, the bar stops.
   Play resumes. Next/previous change the track; the art and title update (art may take a moment).
4. **Seek:** click the middle of the bar → Spotify jumps there, and the elapsed time follows.
5. **YouTube in Edge/Chrome:** play a video. It appears with its thumbnail and "Chrome"/"Edge". Play/pause works;
   next/previous are disabled unless the page provides them (e.g. a playlist).
6. **Two sessions:** with Spotify playing, start YouTube. YouTube moves to the top and chips appear. Click the Spotify
   chip → the card switches to Spotify. Then pause and play Spotify from Spotify itself: the card follows the newest session.
7. **Session ends:** close the browser tab, or quit Spotify → its card/chip disappears within a second.
8. **Focus:** typing in Notepad continues uninterrupted while you use the media buttons (no focus theft).
9. **Fit:** at text size 120 % and 150 %, the panel with the media card fits; the apps grid scrolls rather than overflowing.
10. **Windows Media Player** (the new app): play a local file → it shows as "Media Player" with its controls.

### Phase 2.5: Live Activities (complete 2026-10-30)
Sources push activities into one channel; the hover poller owns the queue as part of its state machine (`Showing` phase,
CASESTUDY D14). The island morphs to a 340 px compact activity (92 px tall for screenshots) and back.
Kick-off prompt: [prompts/phase-2.5-live-activities.md](prompts/phase-2.5-live-activities.md).
- [x] Queue: *feedback* (volume, brightness, Caps Lock) shows at once, replaces what's showing, and is dropped while the
      panel is open; *notices* (Bluetooth, power, screenshot) wait, go stale (10–60 s) and resume after interruption.
      Same key = update in place (one id). Never covers the open panel; hot-zone dwell still opens it (D14)
- [x] Volume/mute: `IAudioEndpointVolumeCallback` on the default output, re-subscribed by `IMMNotificationClient` (D15)
- [x] Brightness: WMI `WmiMonitorBrightnessEvent`, pulled semi-synchronously (D15)
- [x] Bluetooth audio connect/disconnect from endpoint state changes, deduplicated by container id; battery from the
      undocumented `{104EA319…} 2` property, re-read 1.5 s after connecting (D15)
- [x] Charger plugged/unplugged and low battery (crossing 20 % / 10 % on battery) via power-setting notifications on a
      message-only window
- [x] Caps Lock: `GetKeyState` polled in the 60 Hz poller (no hook)
- [x] Screenshot: overlapped `ReadDirectoryChangesW` on the Screenshots known folder, shell thumbnail; hovering holds it
      and makes it clickable; drag out with `SHDoDragDrop`, click to open (D16)
- [x] Switches in `%APPDATA%\dev.wpill.topbar\settings.json` (`activities.volume`, …, read at launch; UI in Phase 4b)
- [x] Charging look (D18): green glowing outline with a light orbiting it every 2.4 s, soft green halo, bolt and
      level in green; static ring under reduced motion. Checked by rendering the built CSS in headless Edge
- [x] Verified from outside (2026-10-30, `RUST_LOG=topbar_lib=debug`, SendInput + screen captures + `GWL_EXSTYLE`):
      Caps Lock toggled and restored → "Caps Lock · On" activity, island stayed click-through, foreground unchanged;
      2× volume up + 2× down (restored) → one activity updated in place; WMI brightness 100 → 90 → 100 → activity at 90;
      a file copied into Screenshots → thumbnail activity, clickable only while hovered, held past 6 s, gone ~1 s after
      leaving; hot-zone dwell during an activity → panel; a screenshot arriving while the panel was open showed after it
      collapsed; a synthetic drag from the thumbnail started a shell drag (drag image, "+ Copy") without changing the
      foreground window. 36 Rust tests pass
- [x] Manual check (below): all 12 pass, 2026-10-30 (including Bluetooth headphones, charger, screenshot drag-out and fullscreen)

**Manual check (Phase 2.5)** (run with `$env:RUST_LOG='info,topbar_lib=debug'`; `activity:show` lines appear in the log)
1. **Volume keys:** press volume up/down several times. One activity shows the bar moving and the level; it leaves 1.5 s
   after the last press. Mute shows "Muted" and an empty bar. Windows' own flyout also appears (known limitation).
2. **Brightness keys:** same, with the sun icon. Adaptive brightness changes may also show it (known limitation).
3. **Bluetooth headphones:** connect → "WUF-W35 · Connected" (with "· N%" if it reports battery, possibly appearing a
   moment later). Disconnect → "Disconnected". One activity each, not one per audio profile.
4. **Charger:** unplug → "On battery · N%"; plug in → "Charging · N%" with a green bolt, the outline glows green and a
   light orbits it smoothly (no stutter) until the activity leaves, when the ring fades out with it.
5. **Caps Lock:** on → "Caps Lock On" in the accent color; off → "Off".
6. **Screenshot:** Win+PrtScn → "Screenshot saved" with the thumbnail. Hover it: it stays up. Drag the thumbnail into an
   Explorer folder, a chat or an email → the file is copied there and the activity ends. Take another and click the
   thumbnail → it opens in Photos and the activity ends. Win+Shift+S (Snipping Tool auto-save on) also shows it.
7. **Panel open:** open the panel and press a volume key → nothing covers the panel. Take a screenshot with the panel
   open, then leave it → the screenshot activity appears after the panel closes.
8. **Hover during an activity:** while a volume activity shows, rest on the top edge → the full panel opens.
   Clicks under a volume/brightness activity reach the app beneath it.
9. **Focus:** typing in Notepad continues while activities come and go and while dragging a screenshot out.
10. **Fullscreen:** in a fullscreen video, volume keys show no island; a screenshot taken there appears after leaving
    fullscreen if within 30 s.
11. **Theme:** with Spotify playing (tinted island), activities use the art's colors; with nothing playing, black + green.
12. **Switches:** set `"capsLock": false` in `%APPDATA%\dev.wpill.topbar\settings.json`, restart → Caps Lock shows nothing.

### Phase 2.6: Claude Code activity (complete 2026-10-30)
The island tells you when a local Claude Code session finishes a task or needs a decision, through Claude Code
hooks. Start from [prompts/phase-2.6-claude-code.md](prompts/phase-2.6-claude-code.md) (research and a design
proposal before any code). Depends on Phase 2.5's activity queue.

```
Claude Code hook ──stdin JSON──▶ topbar.exe hook (forwarder, exits before Tauri starts)
                                   │ per-user named pipe
                                   ▼
                     TopBar: claude/ listener thread ──▶ activity queue (2.5) ──▶ island
                                                     └──▶ claude:sessions ──▶ panel section
```

- [x] `topbar.exe hook` forwarder (`claude/forward.rs`): checked in `main.rs` before Tauri; keeps only `session_id`,
      `cwd`, the event, `notification_type` and `source` (never prompt, tool or assistant text), adds its ancestor
      process chain, writes one message to the pipe. Stdin read capped at 8 MiB with a 1 s deadline; a busy pipe is
      retried for up to 1 s; always exits 0 and prints nothing
- [x] Pipe listener (`claude/pipe.rs`, a `Watcher`): `\\.\pipe\topbar-claude-<user SID>`, DACL for the current user
      only (`D:P(A;;GA;;;<SID>)`), `PIPE_REJECT_REMOTE_CLIENTS`, `FILE_FLAG_FIRST_PIPE_INSTANCE`, one instance, one
      message per connection, 16 KiB cap, 500 ms read timeout, overlapped connect/read waited with the stop event
- [x] Sessions (`claude/sessions.rs`): `idle | working | done | needsInput` keyed by `session_id`; events older than
      the last applied one are ignored (async hooks arrive out of order); dropped when their `claude.exe` exits
      (checked every 30 s, pid + creation time) or after 12 h without events
- [x] Activities: `claudeDone` on `Stop` (or `StopFailure`, "Stopped"), `claudeNeedsInput` on a permission prompt or
      question, *sticky* until the session's next event (D19). Both interactive: a click focuses the session
- [x] Claude look (motion-designer agent): Claude Code's spinner (`· ✢ * ✶ ✻ ✽` forwards then back, 120 ms a frame,
      Segoe UI Symbol, `--claude: #d77757`) left of the clock while a session works and on working rows; the island's
      edge glows orange while "done" shows and breathes slowly (3 s) while "needs input" shows. Reduced motion: a
      static `●` and a static edge. Checked in headless Edge renders on the plain and tinted island
- [x] Activity layout v2 (2026-10-30, after sign-off, from feedback): one 44 px row like the other compact
      activities. ✻ · project · "Done"/"Stopped" sliding out from behind the name (smaller, muted, 320 ms), and at the
      right end an orange check that draws in (360 ms, 120 ms after the text) or three blinking dots while it needs
      input (1.4 s cycle, aria-label "needs your input"). Needs-input → done switches in place. Reduced motion: no
      slide, check fully drawn, still dots
- [x] Panel section "Claude Code" between the media card and the apps: status mark, project, status, how long ago;
      click focuses (nearest ancestor owning an Alt+Tab window, then a title match on the folder name)
- [x] Tray "Connect Claude Code…" / "Disconnect Claude Code…": a native message box shows the exact entries and asks;
      backs up `settings.json`, merges (key order kept), quoted exe path, removes only our entries. Honours
      `CLAUDE_CONFIG_DIR`. Switch `activities.claude` in `settings.json`
- [x] Verified from outside (2026-10-30, `RUST_LOG=topbar_lib=debug`): sample hook JSON piped into `topbar.exe hook`
      (start → prompt → permission → tool → stop, and a 3 MB `Stop`) produced the expected activities, with no prompt
      or answer text in the log; ~85–150 ms per forwarder run (debug build), exit 0 with TopBar not running, with no
      stdin and with garbage; 12 forwarders at once all delivered; garbage, a 40 KB message and a silent client
      written straight to the pipe were dropped without stopping the listener; the pipe's ACL lists only the current
      user. Real `claude -p` runs on Claude Code 2.1.123 and 2.1.159 with the entries below (via `--settings`, the real
      `settings.json` untouched) sent SessionStart → prompt → Stop → SessionEnd and showed "hooktest · Done". The
      release exe (windows subsystem, so no console can flash) reads stdin through Git Bash and delivers in ~200 ms.
      Captures of the live island (`pnpm tauri dev`): spinner beside the clock while working, "wpill · Needs your
      input" with the orange edge, back to the spinner once the tool ran, then "wpill · Done"; the foreground window
      never changed. 67 Rust tests pass
- [x] Manual check (below): all 12 pass, 2026-10-30 (Windows Terminal with a PowerShell profile, VS Code, two
      sessions, permission prompts, Esc, fullscreen, TopBar not running, disconnect). Note: a WSL profile runs Linux
      Claude Code with its own settings, which never calls `topbar.exe` (known limitation)

**Manual hook entries.** Add these to `~/.claude/settings.json` under `"hooks"` (the tray item writes the same thing,
with the path of the TopBar that wrote it). One group per event, for `SessionStart`, `UserPromptSubmit`, `PostToolUse`,
`Notification`, `Stop` and `StopFailure`:

```json
{ "hooks": [ { "type": "command", "command": "'C:/Users/<you>/AppData/Local/TopBar/topbar.exe' hook",
               "shell": "bash", "async": true, "timeout": 5 } ] }
```

and for `SessionEnd` the same without `"async": true` (it runs at exit, so it shouldn't be left in the background).
Without Git Bash, use `"shell": "powershell"` and `"command": "& 'C:\\Users\\<you>\\…\\topbar.exe' hook"`. Keep the
shell form: Claude Code before 2.1.15x has no `args` exec form and would run a bare `topbar.exe`, starting TopBar.

**Manual check (Phase 2.6)** (run with `$env:RUST_LOG='info,topbar_lib=debug'`; connect from the tray first, then
restart any open Claude Code sessions)
1. **Task finishes while you're elsewhere:** give Claude a task, switch to another app. When it finishes the island
   shows "<project> · Done" with an orange edge for 5 s; the pill shows the spinner left of the clock while it works.
2. **Permission prompt:** ask for something that needs a permission (e.g. a Bash command not on the allow list), switch
   away. About 6 s later "<project> · Needs your input" appears with a slowly breathing orange edge and stays. Approve
   it in the terminal → the activity leaves when the tool finishes (and "Done" follows when the turn ends).
3. **Click to focus:** while "Needs your input" shows, hover it and click → the session's terminal comes to the front
   and the activity is gone. Clicks beside the island still reach the app under it.
4. **Two sessions in different folders:** both show in the panel with their own status; their activities take turns
   (needs-input first), never two at once.
5. **VS Code and Windows Terminal:** one session in each. Clicking each panel row brings the right window forward; with
   two VS Code windows open, the one whose title names the folder.
6. **Panel open when a session finishes:** keep the panel open → nothing covers it, the row turns "Done". A "needs input"
   arriving meanwhile shows after the panel closes.
7. **Esc:** interrupt a running task → the spinner stops about a minute later (idle notification), with no activity.
8. **Fullscreen:** a question asked while a fullscreen video plays shows after leaving fullscreen.
9. **TopBar not running:** quit TopBar; Claude Code runs normally with no hook errors.
10. **Close a terminal mid-task:** its session leaves the panel within ~30 s.
11. **Hooks removed:** tray → "Disconnect Claude Code…" → only TopBar's entries are gone (compare with the backup next
    to `settings.json`); the tray label switches back.
12. **Focus:** typing in another app continues while Claude activities come and go.

Only sessions that run on this machine and fire hooks are covered; claude.ai/code cloud sessions don't run here.

### Phase 2.7: local automation agent
**Goal:** press a hotkey, type an instruction ("build the release installer and copy it to Drafts", "rename these
screenshots by date", "open the PR page for this branch"), pick a folder, and walk away. TopBar runs a headless Claude
Code session for it, the island shows it working, asks you to approve risky steps right there (Allow / Deny), and tells
you when it's done. Claude gets desktop tools (windows, screenshots, UI Automation, and later clicking and typing)
from TopBar itself. Kick-off prompt: [prompts/phase-2.7-automation-agent.md](prompts/phase-2.7-automation-agent.md)
(research and a design proposal before any code). Decision: CASESTUDY D21.

```
hotkey ─▶ command window (focusable, separate) ── agent_start ─▶ Rust agent/ ── spawns ─▶ claude -p (stream-json)
                ▲ transcript (agent:output)               │  reads NDJSON events ◀──────────────┘   │
                │                                         ▼                                         │ stdio MCP
         island: spinner, agentApproval (Allow/Deny), agentFinished          topbar.exe mcp (bridge) ◀┘
                ▲                                         ▲                         │ per-user pipe
                └──────── activity queue (2.5/D19) ◀──────┴── TopBar MCP server ◀───┘
                                                              tools: approve, windows, screenshot, ui_tree, (click, type)
```

**Research at kick-off (2026-10-30, Claude Code 2.1.123, 2.1.159 and 2.1.312, plus small haiku runs):**
- `--permission-mode default` works in all three (2.1.312's help lists `manual`, an alias since 2.1.200). The flag
  overrides `defaultMode` from settings, but the user's allow rules, `PermissionRequest` hooks and mods still apply:
  steps already allowed in `~/.claude/settings.json` aren't asked about. From 2.1.302 `-p` starts in Manual anyway.
- `--permission-prompt-tool` and `--max-turns` work but aren't in `--help`. The prompt tool gets
  `{ tool_name, input, tool_use_id }` and replies with a text block holding `{"behavior":"allow","updatedInput":…}` or
  `{"behavior":"deny","message":…}` (not in the docs; checked by hand). A deny reaches Claude as an error tool result.
- Stream-json: `system/init` (session id, model, MCP status) opens every turn; `assistant` has `text` / `tool_use`;
  `user` has `tool_result`; `result` has `subtype` (`success`, `error_max_turns`, `error_max_budget_usd`,
  `error_during_execution`), `num_turns` (this turn), `total_cost_usd` (the session), `errors`, `permission_denials`.
  A user message written mid-turn is queued. A turn limit exits with code 1 and may let one more batch of tools run.
- `--max-budget-usd` caps Claude Code's own estimate (can overshoot); on a subscription it's a usage cap, not billing.
- Hooks fire in `-p` runs with the same session id, so the user's TopBar hooks see agent runs (2.6 now ignores them).
- Windows has no SIGTERM/SIGINT for these: Stop is TerminateJobObject; the transcript is on disk, so `--resume` works.
- Differences from this plan: `agent:run` → `agent:runs` (a list, like `claude:sessions`); `AgentRun` has no
  `sessionId` (Claude's id stays in Rust); the hotkey needs no message-only window (`RegisterHotKey(NULL)` posts to
  the thread); `AskUserQuestion` is disallowed (nobody can type an answer mid-run).

**Principles (non-negotiable)**
- **You approve, the island asks.** Every action Claude Code would ask about goes to `mcp__topbar__approve`, which
  shows an `agentApproval` activity and waits. Nothing runs with `bypassPermissions`; TopBar always passes
  `--permission-mode default` explicitly, whatever the user's settings say. Reading (screenshots, window lists, UI
  trees) is risk `read`; file and shell changes are `change`; clicking and typing into other apps are `input`, which
  can never be pre-approved for longer than one run.
- **Rule 2 still holds.** The island never takes keyboard input. Typing happens in the separate `command` window
  (a normal focusable Tauri window, hidden when not used). Approvals are clicks.
- **Rust owns runs.** Processes, timers, approvals and limits live in `src-tauri/src/agent/`; React renders events.
- **Always stoppable.** A visible agent state in the pill, Stop in the panel and the command window, and a global stop
  hotkey. Stopping kills the whole process tree (Job Object), including anything Claude started.
- **Leave a trail.** Every tool call, approval and result is appended to
  `%APPDATA%\dev.wpill.topbar\agent\<runId>.jsonl` (local only, pruned after 30 days).
- **Island shows status, not content** (as in 2.6): project, step name, the approval summary. Full output lives in
  the command window.

**Stage 2.7a: command window and headless runs** (useful alone: fire-and-forget tasks with island status)
- [x] Global hotkeys via `RegisterHotKey` on the `agent-hotkeys` thread (no window): **Win+Alt+A** opens the `command`
      window (prompt, recent folders, Browse…, model, turns, budget, Claude Code install), **Win+Alt+Escape** stops
      every run. A taken hotkey is logged and shown in the window. Esc hides it
- [x] `agent/` module: finds `claude.exe` installs (PATH, npm shim → real exe, `~/.local/bin`, VS Code; newest
      version unless `agent.claudePath`), spawns it suspended with `CREATE_NO_WINDOW` into a Job Object
      (`KILL_ON_JOB_CLOSE`), stream-json both ways, `--append-system-prompt` with TopBar's guidance (`run.rs`). Stdout
      is parsed on its own thread (`stream.rs`): init (session id), tool steps (→ "Running git status"), result (turns,
      cost, limits). Stdin closes once every message has its result; follow-ups resume with `--resume`
- [x] No double activities: 2.6's listener ignores hooks whose ancestor chain contains TopBar itself; agent runs are
      listed from `agent:runs` in the same panel section (agent badge, step, Stop), and `agentFinished` replaces
      `claudeDone` for them
- [x] Follow-ups (`agent_send`), Stop (panel, command window, hotkey), limits (`maxTurns` 25, `maxBudgetUsd` 1 by
      default; "Turn limit reached" / "Budget reached")
- [x] Audit trail `%APPDATA%\dev.wpill.topbar\agent\<runId>.jsonl` (start, tool, approval, asked, decision, followUp,
      stop, result; tool input cut to 4 KB), pruned after 30 days. Run ids are `20261030-142233-7f3a`
- [x] Settings in `settings.json` → `agent`: `hotkey`, `stopHotkey`, `claudePath`, `model`, `maxTurns`,
      `maxBudgetUsd`, `strictMcpConfig` (true), `userSettings` (true), `approvalTimeoutSecs` (600)
- [x] **Claude UI refresh** (motion-designer agent): status 11 px at 48 % white on the project's baseline; Claude
      content in **Cascadia Mono** (`--font-claude`, fallback Cascadia Code → Consolas → Segoe UI Variable Text),
      picked from rendered candidates (Cascadia Mono, Sitka Text, IBM Plex Sans); the spinner stays Segoe UI Symbol.
      The command window is sized by Windows' Text size factor

**Stage 2.7b: approvals in the island**
- [x] `topbar.exe mcp` (`bridge.rs`): run from `main.rs` before Tauri; relays newline JSON-RPC ↔ the per-user pipe
      `\\.\pipe\topbar-agent-<SID>` (duplex, message mode, user-only DACL). Its first message names the run and its
      128-bit token (from the generated `mcp.json`'s `env`, deleted when the process exits); anything else is refused.
      4 MiB cap per message; oversized calls are answered with an error by the bridge
- [x] `approve` → `agentApproval` (sticky, keyed per request): project · tool · risk, the summary built in Rust (the
      command, the path relative to the folder, the host), Deny / Allow, and "Allow {git / edits / …} for this run"
      where offered. Clicking the summary opens the command window on the run. No answer in 10 minutes = deny
- [x] Remember "for this run": per program for shell commands, one group for edits, per host for fetches; never for
      `input` or a shell command with an operator (`&`, `|`, `;`, backtick, `$(`, redirects, newlines)
- [x] Several runs waiting: approvals take turns (D19 rules) and park under the panel and fullscreen; the panel lists
      pending ones under their run with Deny / Allow
- [x] Interactive (non-agent) sessions stay notify-only, as in 2.6
- [x] Verified from outside (2026-10-30, debug build, haiku, `topbar.exe agent-test …` in debug builds only): deny
      (no file, Claude adapted, "Done"), allow, "allow for this run" (second `touch` answered as `remembered`), compound
      command not rememberable, turn limit (`failed / maxTurns`), follow-up resuming a stopped run, Stop with
      `ping -t localhost` under three shells (all six processes gone), TopBar killed mid-run and quit mid-run (no
      `claude.exe`, bridge or `PING.EXE` left), hotkey → command window in the foreground, captures of the approval
      activity and the command window. 95 Rust tests pass
- [x] Manual check (below): all 12 pass, 2026-10-30

**Stage 2.7c: desktop tools, read-only** (complete 2026-10-30). Kick-off prompt:
[prompts/phase-2.7c-desktop-tools.md](prompts/phase-2.7c-desktop-tools.md). Decisions: CASESTUDY D25, D26.

*Research at kick-off (Claude Code 2.1.159's bundle, checked live on 2.1.314):* MCP image blocks (`image/png`,
`jpeg`, `gif`, `webp`) reach the model as images, billed as image tokens (~w×h/750). Claude Code resizes them to fit
2000×2000 and re-compresses anything over 512,000 bytes itself. A tool result over `MAX_MCP_OUTPUT_TOKENS` (25,000;
an image counts as 1,600) is saved to a file or cut. MCP tools return "MCPTool requires permission", so in `default`
mode they go to `--permission-prompt-tool`; a user allow rule skips that, hence TopBar's own grant check.
`readOnlyHint` makes Claude Code treat a tool as read-only and parallel-safe, so TopBar doesn't set it.
`--allowedTools` isn't used: it would approve looks before anything happens. Claude Code loads MCP tools lazily
(`ToolSearch` → `select:mcp__topbar__…`) before the first call.

- [x] `desktop/` feature folder (Win32 isolated): `windows.rs` (the Alt+Tab list from `apps/`, target checks),
      `capture.rs` (`PrintWindow(PW_RENDERFULLCONTENT)` for a window, covered or not; `BitBlt` of the primary monitor
      with TopBar's windows set to `WDA_EXCLUDEFROMCAPTURE` only for that moment), `encode.rs` (WIC Fant scaling,
      PNG, else JPEG), `uia.rs` (control view walk, one cache request per element, 2 s UIA timeouts, 3 s budget, on a
      per-look MTA thread with an 8 s limit). Pure and tested: `logic.rs`, `tree.rs`
- [x] Four MCP tools (`agent/tools.rs`): `list_windows`, `focused_window`, `screenshot`, `ui_tree` (IPC table above).
      Refused: closed windows, TopBar's own, other virtual desktops (cloaked), minimized (screenshot), hung, elevated,
      and everything while the lock screen or a UAC prompt is up. Password fields never carry a value
- [x] Approvals: risk `read`; summaries "List open windows", "Screenshot of Chrome · GitHub", "Screenshot of the whole
      screen", "Read the UI of Notepad · notes.txt"; step labels "Taking a screenshot" etc.; "for this run" per kind
      (window lists = `list_windows` + `focused_window`, screenshots, reading UIs). Allowing a prompt leaves a one-time
      grant for that input; a call without one (a user allow rule) is asked about by TopBar itself; the window's
      process is checked again at look time
- [x] Screen-edge glow (`window/glow.rs`): native layered window over the looked-at monitor, never captured, below
      the island; Claude's accent, 32 px band, `0.6·(1−t)²`, 220 ms in, 1.5 s hold, 400 ms out; no fades with
      animations off (motion-designer spec)
- [x] Eye glyph in the `read` badge for desktop looks (island approval and the command window's pending card;
      motion-designer); transcript shows `[image]` for images; audit `look` entries name the target and size, never
      the image or tree; guidance in `--append-system-prompt` (prefer shell/files, `ui_tree` before `screenshot`, say
      what you're looking for)
- [x] Verified from outside (2026-10-30, debug build, haiku on 2.1.314, `agent-test` and the new
      `agent-test look …`): "what windows are open" (approved in the island, 4 windows, 1¢); "what does Notepad say"
      (`ui_tree`, 102 elements in 0.75 s, text read exactly); whole-screen screenshot (1568×882 JPEG, 89 KB);
      Notepad covered by a fullscreen video captured cleanly (460×517 PNG, 12 KB); "Allow screenshots for this run"
      (second shot `remembered`); Deny on `ui_tree` and `list_windows` (Claude stopped, no `look` entries); a
      project allow rule for `screenshot` (Claude Code skipped its prompt, TopBar asked, `by: topbar`, denied); closed
      hwnd → `windowGone`; minimized → `minimized`; elevated Task Manager → `elevated` for both tools; the glow window
      covering 1920×1080 with affinity 17, hidden ~2.3 s after a look; a screen capture with the island showing (no
      pill in the image). Largest image 105 KB, far under the 4 MiB pipe cap. 115 Rust tests pass
- [x] Manual check (below): all 12 pass, 2026-10-30

**Manual check (Phase 2.7c, all pass 2026-10-30)** (`pnpm tauri dev` with `$env:RUST_LOG='info,topbar_lib=debug'`, model `haiku`, a scratch
folder)
1. **Window list:** "What windows are open?" asks "List open windows" with an eye in the badge; the answer lists your
   windows and never TopBar's command window.
2. **Glow:** while any look runs, the screen's edges glow softly in Claude's color, under the island, and fade about
   1.5 s after; it never steals focus or clicks, and isn't in your own screenshots (Win+Shift+S during a look).
3. **UI tree:** "What does the Notepad window say?" asks "Read the UI of Notepad · …" and answers with the text.
4. **Password:** a sign-in form with a typed password (e.g. a browser page) read with `ui_tree`: the field shows as
   `password` with no value.
5. **Screenshot of a window:** "Screenshot the Spotify window and describe it" works while Spotify is covered.
6. **Whole screen with the island open:** hover the island open (or have an activity up) during "screenshot the whole
   screen, what's at the top center?": Claude doesn't see the island, the command window or the glow.
7. **Deny:** deny each kind once; Claude adapts and the audit has no `look` entry.
8. **For this run:** "Allow screenshots for this run", then a second screenshot (other window or the screen) runs
   without asking; a `ui_tree` still asks.
9. **Gone / minimized / elevated:** close a window between list and call (`windowGone`); minimized window screenshot
   (`minimized`); an app run as administrator (`elevated`).
10. **Locked:** start a run that waits a few seconds then screenshots (e.g. after `sleep 10`), lock with Win+L: the
    look fails with `screenLocked`, nothing is captured.
11. **Audit:** the run's `.jsonl` has `look` lines with target, size and format, and no image data.
12. **Text size 100 / 120 / 150 %:** the eye sits in the badge without changing the approval's size.

**Stage 2.7d: desktop actions** (complete 2026-10-31). Kick-off prompt:
[prompts/phase-2.7d-desktop-actions.md](prompts/phase-2.7d-desktop-actions.md). Decision: CASESTUDY D27; challenge C11.

*Research at kick-off (Claude Code 2.1.123 / 2.1.159 / 2.1.314 bundles):* MCP tools still need permission in `default`
mode, so actions reach `--permission-prompt-tool`; a user allow rule skips it, and TopBar's one-time grant check then
asks itself (checked live with a project allow rule). `MCP_TOOL_TIMEOUT` defaults to 1e8 ms (a per-server `timeout`
in `mcp.json` overrides it); 2.1.314 adds a 30 min idle limit for stdio servers, so a 10 min approval wait is fine.
On abort or timeout the client sends `notifications/cancelled { requestId }`. UIA patterns don't bring windows
forward (`SetFocus` does); a Win32 `Invoke` that opens a modal dialog can block until UIA's timeout.
`GetLastInputInfo` counts injected input; low-level hooks' `*_INJECTED` flags tell yours from ours. No new
`windows` features beyond `Win32_System_SystemInformation` (`GetTickCount64`).

- [x] `desktop/`: `act.rs` (re-find an element by runtime id, at its old position then `FindAll`, must match type,
      name and automation id; Invoke/Toggle/SelectionItem/ExpandCollapse/ScrollItem/focus/LegacyIAccessible default
      action; Value), `input.rs` (`SendInput` clicks over the virtual desktop, real-key typing from the window's
      layout, chords; target in front and point on it before every event; keys always released), `guard.rs`
      (low-level hooks only while acting), `launch.rs`; pure and tested: `plan.rs`, `refs.rs`
- [x] Seven MCP tools (`agent/actions.rs`, IPC table above); `ui_tree` lines carry `[n]` refs; clicks can be given in
      the run's last screenshot's image px (`staleView` if the window moved)
- [x] Approvals: risk `input`, one per action, never remembered (no batches: step two depends on step one); summaries
      "Press "Save" in Notepad · notes.txt", "Set "Search" to "weather" in …", "Click at 812, 40 in …", "Type
      "hello" into …", "Press Ctrl+A, then Delete in …", "Bring … to the front", "Open https://github.com in your
      browser", "Open the Calculator app"; step labels; the grant check covers user allow rules; typed text in full
      in the command window's card, only `{chars, lines}` in the audit
- [x] Refused: TopBar's windows, elevated, other desktops, hung, locked screen / UAC, password fields (also checked
      while typing), secret-looking text, Win-key and shell chords, `file:` and other schemes
- [x] Your input first: waits until you've let go for 500 ms (≤10 s), then any non-injected key or mouse event aborts
      it before its next event; Stop, Win+Alt+Escape and a cancelled call abort too; one action at a time; the island
      stays collapsed and click-through meanwhile (`hover::set_hands_off`)
- [x] UI (motion-designer): pointer glyph in the `input` badge, `agentActing` activity ("Waiting for you to let go…" →
      "Move the mouse or press a key to stop"), the command window's `detail` block; acting glow (band 40 px, peak
      0.72, 2 px rim at 0.92, 600 ms hold, act wins over look, repaint without a crossfade)
- [x] Guidance in `--append-system-prompt`: shell and files first, then refs + `invoke`/`set_value`, then raw input,
      `launch` for links and apps; look before and after, one action at a time, say what's next; stop after a deny
      or an abort; never type secrets
- [x] Verified from outside (2026-10-31, debug build, haiku on 2.1.314, approvals clicked in the island, and the new
      debug `agent-test act <tool> <argsJson>`): `set_value` into a scratch Notepad (text read back exactly);
      `invoke` of Notepad's Settings and Back buttons with the window behind; `focus_window`; a raw `click` on the
      editor by ref; `press_keys` Ctrl+End and Ctrl+A, Delete; `type_text` read back exactly in Notepad and WordPad
      after C11 (82 and 80 characters, Shift-heavy); `launch https://github.com`; Deny on `press_keys` (no `act`
      entry, Claude stopped); a project allow rule for `focus_window` (TopBar asked itself); "for this run" never
      offered; closed window → `windowGone`; elevated Task Manager → `elevated`; focused Chrome password field →
      `passwordField` before any key; secret text, Win+R and `file:` refused; typing aborted by moving the mouse
      after 269 of 391 characters with no modifier left down. 133 Rust tests pass
- [x] Manual check (below): all 12 pass, 2026-10-31

**Manual check (Phase 2.7d, all pass 2026-10-31)** (`pnpm tauri dev` with `$env:RUST_LOG='info,topbar_lib=debug'`, model `haiku`, a scratch
folder, Notepad on a scratch file)
1. **Set a field:** "Set the Notepad text to 'hello' using TopBar's tools": `ui_tree`, then "Set "Text editor" to
   "hello" in Notepad · …" with a pointer in the `input` badge and no "for this run"; Notepad shows hello.
2. **Invoke:** "Press Notepad's Settings button" while Notepad is behind another window: it works without coming
   forward; the island shows the acting activity briefly and the screen edges glow with a rim.
3. **Raw input:** "Click the editor, press Ctrl+End and type ' done.'": three approvals, each waiting until you let go;
   the text arrives exactly, the island stays closed even if the cursor passes the top centre.
4. **Command window:** a `type_text` approval shows the full text in the pending card; the island shows it short.
5. **Deny:** deny an action; Claude stops and says what it wanted; the audit has no `act` entry.
6. **Abort:** a long `type_text` (≈300 characters); move the mouse while it types: it stops at once, Claude is told
   `aborted`, and Shift/Ctrl aren't left down. Repeat with Win+Alt+Escape (the run stops too).
7. **Hands off:** keep moving the mouse after Allow: the activity says "Waiting for you to let go…"; after 10 s the
   action is `aborted` without doing anything.
8. **Gone / elevated / locked:** close the window between look and act (`windowGone`); an app run as administrator
   (`elevated`); Win+L during the hands-off wait (`screenLocked`).
9. **Password and secrets:** a browser sign-in page with the password field focused: `type_text` is refused
   (`passwordField`); asking Claude to type a token-like string is refused.
10. **Launch:** "Open github.com" opens your browser; "Open file:///C:/Windows/System32/calc.exe" is refused; "Open
    the Calculator app" starts Calculator.
11. **Audit:** the run's `.jsonl` has `act` lines with tool, target and result, and typed text only as `{chars, lines}`.
12. **Text size 100 / 120 / 150 %:** the pointer badge, the acting activity and the pending card's text block fit.

**Stage 2.7e: routines** (next). Kick-off prompt: [prompts/phase-2.7e-routines.md](prompts/phase-2.7e-routines.md)
- [ ] Saved instructions (prompt + folder + limits), run from the command window or on a schedule while TopBar runs

**Out of scope:** cloud or remote sessions, WSL sessions, voice input, other machines, running TopBar elevated,
approving prompts of interactive terminal sessions from the island (possible later through the `PermissionRequest`
hook; deliberately not mixed in here).

**Manual check (Phase 2.7a–b, all pass 2026-10-30)** (run the installed build or `pnpm tauri dev` with
`$env:RUST_LOG='info,topbar_lib=debug'`; use a test folder and cheap tasks, e.g. model `haiku` in Options)
1. **Hotkey:** Win+Alt+A from any app opens the command window with the cursor in the prompt; Esc hides it; the island
   never takes focus (typing in another app afterwards still goes there).
2. **A read-only task:** "List the 5 largest files in this folder" runs: the pill shows the spinner, the panel row shows
   the step, `agentFinished` "Done" appears, and the answer is in the command window.
3. **Approval, Deny:** a task needing a shell command shows the Allow / Deny activity with the command; Deny → Claude
   adapts and says so; the transcript shows "Denied by you".
4. **Approval, Allow and "for this run":** Allow runs it; "Allow git for this run" lets the next `git …` through without
   asking, but a different program (or `git a && rm b`) still asks.
5. **Stop:** a task running `ping -t localhost` (allow it) → Stop from the panel → the run reads "Stopped" and Task
   Manager shows no `PING.EXE` or extra `claude.exe`. Repeat with Win+Alt+Escape.
6. **Limits:** max turns 2 on a multi-step task ends with "Turn limit reached".
7. **Two runs at once:** both show in the panel; their approvals take turns; transcripts stay separate.
8. **Panel open / fullscreen:** an approval arriving with the panel open is listed under its run there (answerable);
   one arriving during a fullscreen video shows after leaving fullscreen.
9. **Follow-up:** after "Done", type a follow-up in the command window → the same run continues (resumed).
10. **TopBar quits mid-run:** tray → Quit while a run works → no orphaned `claude.exe`.
11. **Audit:** `%APPDATA%\dev.wpill.topbar\agent\<runId>.jsonl` lists the start, each tool call, approval and decision.
12. **Text size 100 / 120 / 150 %:** the approval activity and the command window fit; the island's click area matches.

### Phase 4a: release readiness
- [ ] Own visual identity before a public release: replace the Claude Code look-alike spinner, `✻` mark and orange
      with TopBar's own working indicator and an amber accent from the femiajanaku palette. Prompt:
      [prompts/spinner-redesign.md](prompts/spinner-redesign.md)
- [ ] Multi-monitor + per-monitor DPI: re-read geometry on `WM_DISPLAYCHANGE` / `WM_DPICHANGED` and
      reposition. Choose the island's monitor (primary by default)
- [ ] Code signing (certificate or Azure Trusted Signing) wired into `tauri build`
- [ ] Rerun the manual checklist; publish v0.2 installer

### Phase 3: memory-heavy apps (behind a settings toggle)
`sysinfo` refresh every ~2 s while expanded only. Group by process name, sum memory, sort.
End Task opens a confirm dialog, then `TerminateProcess` per pid. Protected list (csrss,
wininit, winlogon, services, lsass, smss, svchost, dwm, explorer, System, Registry, and
our own pid) is refused in Rust, not just hidden in the UI. Access-denied (elevated targets)
gets a clear message explaining the app can't close elevated processes unless run as admin.
Off by default.

### Phase 4b: settings
A settings window (a normal, focusable window, separate from the island) for: activities on/off,
Phase 3 toggle, hot-zone size, monitor choice, and launch at startup. Uses `tauri-plugin-store`.

### Phase 5: utilities
- [ ] Timer / stopwatch / focus mode: countdown lives in the pill; presets in the panel
- [ ] Drop shelf: drag files onto the island to park them, drag them out later
- [ ] Clipboard history (recent text/images), opt-in

### Spikes (prototype before committing)
- **Notifications mirror:** `UserNotificationListener` needs user consent and is unreliable for some apps.
  Decide go/no-go after a prototype.
- **Calendar peek:** next meeting in the pill + join link; needs Outlook/Google sign-in.
- **Live window previews:** `DwmRegisterThumbnail` drawn into our HWND at rects reported by React
  (the original Phase 4 item).

## Known limitations
- Geometry (monitor, scale) is read once at startup. Display changes need a restart until Phase 4a.
- Single instance is keyed on the bundle identifier, so a dev build and the installed release build
  can't run at the same time. Quit one from the tray first.
- Icons are cached per HICON/exe path. An app that changes its window icon at runtime may show a stale icon.
- No keyboard interaction in the island by design (`WS_EX_NOACTIVATE`).
- Fullscreen detection ignores fullscreen apps on secondary monitors (the island only lives on the primary one).
- Media: apps decide what GSMTC exposes. Browsers often disable next/previous, some apps never send a timeline (no
  progress bar), and live streams have no duration. App names come from the app id (`Spotify.exe` → "Spotify"),
  with a few known ids mapped; others are just capitalized.
- Activities: Windows' own volume/brightness flyouts still appear next to ours (hiding them needs shell-window hacks).
  Brightness covers internal panels only (no DDC/CI monitors) and also reacts to adaptive brightness. The Bluetooth
  battery level uses an undocumented device property and is missing for devices that don't report it. Screenshots come
  from the Screenshots folder only: PrtScn to the clipboard (no file) isn't an activity. Switches are edited by hand
  until Phase 4b.
- Claude Code: needs the hooks in `settings.json` (tray item); open sessions pick them up after a restart. Permission
  prompts are reported ~6 s late (Claude Code's own timing), and after Esc the session reads "working" for about a
  minute (`Stop` doesn't fire; the idle notification corrects it). Clicking focuses the window, not the Windows Terminal
  tab. The hook entry records the exe that clicked "Connect" (connect from the installed build, like autostart).
  The PowerShell form of the entry (machines without Git Bash) is untested. Sessions inside WSL aren't seen.
- Agent (2.7): only steps Claude Code would ask about reach the island; your own allow rules still auto-approve
  (`agent.userSettings: false` sends everything to the island but drops your hooks and plugins for those runs). Your
  own MCP servers aren't loaded in agent runs unless `agent.strictMcpConfig` is false. Budgets are Claude Code's
  estimate and can be overshot by one response; the turn limit can let one more batch of tools run. Claude can't ask
  questions mid-run (it asks in its final message; answer with a follow-up). The hotkeys are fixed until Phase 4b
  (edit `settings.json`). Clicking the panel's "agent" row opens the command window; there's no per-step "Details" yet.
- Desktop looks (2.7c): Claude Code keeps every image a tool returns in its own session transcript
  (`~/.claude/projects/<folder>/<session>.jsonl`), so screenshots persist there outside TopBar's control (TopBar's audit
  never stores them). Screen capture is the primary monitor only. Elevated windows are refused for both tools (UIPI;
  a whole-screen screenshot still shows them). Protected video comes out black. Big browser pages can hit the `ui_tree`
  time or node cap (it says so). "Allow screenshots for this run" covers every window and the screen, not just the
  one asked about. Excluding the island during a screen capture relies on `WDA_EXCLUDEFROMCAPTURE` (Windows 10 2004+).
- Desktop actions (2.7d): typing is paced for apps that mangle fast synthetic input (C11), so `type_text` takes
  about 10 characters a second and at most 500 (prefer `set_value`); characters the keyboard layout can't type go as
  Unicode packets and may still be mangled by such apps. AltGr characters are typed as packets. Input into elevated
  windows is refused (UIPI). `click` with `from: "screenshot"` needs a screenshot of that window (not of the whole
  screen). Some apps don't notice `set_value` (Win32 `WM_SETTEXT`, React inputs); the reply says when the value
  didn't stick. A UIA `invoke` that opens a modal dialog may be reported as "not confirmed". Only one action runs
  at a time across runs.

## Distribution
- `pnpm tauri build` produces `src-tauri/target/release/bundle/nsis/TopBar_0.1.0_x64-setup.exe`:
  **1.62 MiB download as of Phase 2.6** (1.47 MB at Phase 1; 4.13 MB installed then; see CASESTUDY §7 for RAM/CPU).
- Windows 11 already has the WebView2 runtime, so nothing else is downloaded. On machines without it,
  Tauri's default NSIS mode fetches a small WebView2 bootstrapper during install.
- A fresh release build takes ~11 min (fat LTO). The release profile trades build time for size.
- **Not code-signed yet:** SmartScreen will warn "Windows protected your PC" on other machines.
  Signing needs a code-signing certificate (or Azure Trusted Signing) before a public release.

## Open questions
- Final app name (affects bundle identifier `dev.wpill.topbar`, so decide before the first public release).
- Code signing: buy a certificate, or use Azure Trusted Signing?

## Decided
- Collapsed pill shows the clock (2026-10-28).
- Package manager: pnpm (2026-10-28).
