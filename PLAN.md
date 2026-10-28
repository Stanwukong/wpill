# PLAN.md

> Keep this file current: see "Keep the docs current" in [CLAUDE.md](CLAUDE.md).

## Current status

**Phase 1 is complete (2026-10-28): built, smoke-tested, and all 11 manual tests passed.**
**Phase 1.5 (tray, launch at startup, single instance) is built; it needs the short manual tray check.
Next: Phase 2 (media card).** See [Roadmap](#roadmap).
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
│  │  ├─ media/                  # Phase 2
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
      ├─ media/                  # Phase 2
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

### Reserved for later phases (names fixed now so they slot in)

| Phase | Kind | Name |
|---|---|---|
| 2 | event | `media:state` → `{ title, artist, album, artwork: dataURL\|null, status, positionMs, durationMs, canPlay/Pause/Next/Prev }` |
| 2 | command | `media_play_pause`, `media_next`, `media_prev`, `media_seek(positionMs)` |
| 3 | command | `list_heavy_processes(limit) → ProcessGroup[]` (`{ name, pids, memoryBytes, icon, protected }`) |
| 3 | command | `end_process_group(name, pids) → EndResult` (per pid: `ok \| accessDenied \| protected \| notFound`) |
| 2.5 | event | `activity:show` → `{ kind: "volume" \| "brightness" \| "bluetooth" \| "power" \| "capsLock" \| "screenshot", payload, durationMs }` |
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
| 4 | **4a Release readiness** | Multi-monitor + per-monitor DPI, code signing, v0.2 release | Docking/undocking breaks placement today; unsigned installers trigger SmartScreen. |
| 5 | **3 Memory-heavy apps** | As specified, behind a settings toggle | Power-user tool with the riskiest action (End Task); opt-in only. |
| 6 | **5 Utilities** | Timer/stopwatch/focus, drop shelf + clipboard | High daily value, low platform risk. |
| 7 | **Spikes** | Notifications mirror, calendar peek, live window previews | Uncertain feasibility or needs sign-in; prototype before committing. |

### Phase 1.5: shell basics (built 2026-10-28; awaiting the manual tray check)
- [x] Tray icon (`tauri` `tray-icon` feature, `src-tauri/src/tray.rs`). The menu (left or right click) shows a
      version label ("TopBar 0.1.0", in place of a separate About), a **Launch at startup** checkbox, and **Quit TopBar**
- [x] Launch at startup via `tauri-plugin-autostart` 2.7 (per-user `HKCU\…\Run` entry named `TopBar`, no admin)
- [x] Single instance via `tauri-plugin-single-instance` 2.5: a second launch exits immediately (verified)
- [x] ~~Persist with `tauri-plugin-store`~~: not needed. The Run registry entry *is* the persisted
      state, and the checkbox re-reads it after every toggle (CASESTUDY D10)
- [ ] Manual check (below)

**Manual check (Phase 1.5)**
1. The TopBar icon is in the system tray (it may be in the ^ overflow; drag it out to pin it).
2. Clicking it shows "TopBar 0.1.0" (greyed out), "Launch at startup" and "Quit TopBar".
3. **Quit TopBar** closes the island and the tray icon.
4. Tick **Launch at startup** from the *installed release build*, sign out and back in: the island appears.
   Untick it: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` no longer has a `TopBar` value.
5. Launching TopBar again while it's running doesn't create a second island.

### Phase 2: media card
- [ ] Rust `media/`: GSMTC `GlobalSystemMediaTransportControlsSessionManager` (`windows` features
      `Media_Control`, `Storage_Streams`). Subscribe to `CurrentSessionChanged`, `SessionsChanged`,
      `MediaPropertiesChanged`, `PlaybackInfoChanged`, `TimelinePropertiesChanged`, and emit `media:state`.
- [ ] Album art: read the thumbnail stream, re-encode to a PNG/JPEG data URL, cache per track
- [ ] Controls: `media_play_pause`, `media_next`, `media_prev`, `media_seek` (only where the session allows it)
- [ ] Panel card: art, title, artist, controls, progress bar interpolated client-side between timeline updates
- [ ] **Collapsed pill shows media state:** small art + animated equalizer beside the clock while playing
- [ ] Multiple sessions (e.g. Spotify + a browser tab): show the most recently active one, with a switcher
- [ ] Test with Spotify, Edge/Chrome (YouTube), the Windows Media Player app, and nothing playing

### Phase 2.5: Live Activities
A small activity system: an event source in Rust emits `activity:show { kind, payload, durationMs }`.
The island expands to a compact "activity" size for a few seconds, without stealing hover state, then collapses.
- [ ] Activity queue + priority (don't interrupt a hovered/expanded island; coalesce rapid repeats)
- [ ] Volume and brightness changes (Core Audio endpoint volume callbacks; WMI or monitor brightness)
- [ ] Bluetooth audio connected/disconnected, with battery level where exposed
- [ ] Charger plugged/unplugged, low battery (`GetSystemPowerStatus` / power notifications)
- [ ] Caps Lock toggled
- [ ] Screenshot taken: thumbnail you can drag out (watch the Screenshots folder / clipboard)
- [ ] Per-activity on/off switches (stored now, UI in Phase 4b)

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

## Known limitations (Phase 1)
- Geometry (monitor, scale) is read once at startup. Display changes need a restart until Phase 4a.
- Single instance is keyed on the bundle identifier, so a dev build and the installed release build
  can't run at the same time. Quit one from the tray first.
- Icons are cached per HICON/exe path. An app that changes its window icon at runtime may show a stale icon.
- No keyboard interaction in the island by design (`WS_EX_NOACTIVATE`).
- Fullscreen detection ignores fullscreen apps on secondary monitors (the island only lives on the primary one).

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
