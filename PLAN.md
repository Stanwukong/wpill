# PLAN.md

> Keep this file current: see "Keep the docs current" in [CLAUDE.md](CLAUDE.md).

## Current status

**Phase 1 is complete (2026-10-28): built, smoke-tested, and all 11 manual tests passed.**
**Phase 1.5 is complete (2026-10-30): tray menu, launch at startup and single instance, with all manual checks passed.**
**Phase 2 is complete (2026-10-30): media card, pill art + equalizer and session switcher, with all 10 manual checks passed.**
After sign-off, the island also took its colors from the album art (D13).
**Phase 2.5 is complete (2026-10-30): Live Activities for volume, brightness, Bluetooth audio, power, Caps Lock and
screenshots, with all 12 manual checks passed.** The charging activity also got an orbiting green outline (D18).
**Next: Phase 2.6 (Claude Code activity).** See [Roadmap](#roadmap). Kick-off prompt for a new session:
[prompts/phase-2.6-claude-code.md](prompts/phase-2.6-claude-code.md).
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

### Reserved for later phases (names fixed now so they slot in)

| Phase | Kind | Name |
|---|---|---|
| 3 | command | `list_heavy_processes(limit) → ProcessGroup[]` (`{ name, pids, memoryBytes, icon, protected }`) |
| 3 | command | `end_process_group(name, pids) → EndResult` (per pid: `ok \| accessDenied \| protected \| notFound`) |
| 2.6 | activity kinds | `claudeDone`, `claudeNeedsInput` (payload: `{ sessionId, project }`) in `activity:show` |
| 2.6 | event | `claude:sessions` → `{ sessions: { sessionId, project, status: "working" \| "done" \| "needsInput", updatedAtMs }[] }` |
| 2.6 | command | `claude_focus_session(sessionId)`, `claude_hooks_status() → { installed }`, `claude_hooks_install()`, `claude_hooks_remove()` |
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
| 5 | **4a Release readiness** | Multi-monitor + per-monitor DPI, code signing, v0.2 release | Docking/undocking breaks placement today; unsigned installers trigger SmartScreen. |
| 6 | **3 Memory-heavy apps** | As specified, behind a settings toggle | Power-user tool with the riskiest action (End Task); opt-in only. |
| 7 | **5 Utilities** | Timer/stopwatch/focus, drop shelf + clipboard | High daily value, low platform risk. |
| 8 | **Spikes** | Notifications mirror, calendar peek, live window previews | Uncertain feasibility or needs sign-in; prototype before committing. |

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

### Phase 2.6: Claude Code activity
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

- [ ] `topbar.exe hook` forwarder: argument checked in `main.rs` before Tauri; stdin → named pipe; silent
      exit 0 when TopBar isn't running; never blocks or fails the hook
- [ ] Pipe listener thread (`src-tauri/src/claude/`): current-user-only pipe, size-capped defensive parsing,
      session state keyed by `session_id` (working / done / needs input) with expiry
- [ ] Activities: "<project>: done" on `Stop`; "<project>: needs your input" on `Notification`, at higher
      priority. Project = `cwd` folder name; never prompt or transcript text
- [ ] Claude look: Claude Code's orange spinner glyph (· ✢ ✳ ✶ ✻ ✽, frame-stepped in CSS) in the pill and panel while a
      session works; the island's edges glow Claude orange (`--claude`, charging-ring technique, D18) when a session
      is done (for the activity's duration) or needs input (until its next event). Built through the motion-designer agent
- [ ] Panel section: open sessions with status dots; click focuses the session's terminal or VS Code window
      (best effort: ancestor process chain, then a title match on the folder name)
- [ ] Tray "Connect Claude Code…": shows the exact `~/.claude/settings.json` entries, asks first, backs up,
      merges without clobbering existing hooks, quoted exe path; can remove them again
- [ ] Manual hook JSON documented here for people who prefer to edit settings themselves
- [ ] Manual test list (in the prompt): finish while in another app, permission prompt, two sessions,
      VS Code + Windows Terminal, click-to-focus, TopBar not running, hooks removed

Hooks in scope (confirm the exact fields against the current Claude Code docs first): `Stop`, `Notification`,
`UserPromptSubmit` (working), `SessionStart` / `SessionEnd`. Only sessions that run on this machine and fire
hooks are covered; claude.ai/code sessions don't run here.

### Phase 4a: release readiness
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

## Distribution
- `pnpm tauri build` produces `src-tauri/target/release/bundle/nsis/TopBar_0.1.0_x64-setup.exe`:
  **1.47 MB download, 4.13 MB installed** (see CASESTUDY §7 for RAM/CPU).
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
