# CLAUDE.md

Guidance for Claude Code (and humans) working in this repo.

## Keep the docs current (required)

After every significant change (a new feature, an architecture or IPC change, a new
dependency, a phase milestone, a bug that took real effort to fix, or a decision that
was reversed), update these files in the same change:

| File | What to update |
|---|---|
| [PLAN.md](PLAN.md) | Tick off tasks, adjust scope, record new commands/events, update "Current status". |
| [CASESTUDY.md](CASESTUDY.md) | Add a decision-log entry (what, why, alternatives) and any notable challenge + resolution. |
| [CLAUDE.md](CLAUDE.md) | Update if build/run commands, folder layout, conventions or gotchas changed. |

A change is not finished until these are updated. Small refactors and typo fixes don't need entries.

## Project

**Cay** (formerly TopBar until Phase 4a-3, D32; repo `wpill`): a Windows 11 "Dynamic Island"-style bar pinned to
the top center of the screen. Collapsed it's a small pill; hovering a hot zone at the
top-middle expands it into a panel (running apps; later media, memory-heavy processes,
window previews).

- **Platform:** Windows 11 only. Win32/WinRT calls are fine; don't add cross-platform shims.
- **Stack:** Tauri 2 · React + TypeScript + Vite · Framer Motion · Rust (`windows` crate, `sysinfo`).
- **Package manager:** pnpm.

## Commands

```powershell
.\scripts\check-env.ps1     # verify toolchain (Rust/MSVC/Node/WebView2)
pnpm install
pnpm tauri dev              # run the app (Vite HMR for React; Rust rebuilds + relaunches on save)
$env:RUST_LOG='info,cay_lib=debug'; pnpm tauri dev   # + hover transitions, panel-rect, media:state and activity:show logs
pnpm tauri build            # release build + NSIS installer (~11 min cold: fat LTO; ~1.5 MB installer)
cd src-tauri; cargo fmt; cargo clippy --all-targets; cargo test
pnpm typecheck              # tsc --noEmit
# Debug builds only: drive agent runs from outside (forwarded to the running Cay; results in its log)
.\src-tauri\target\debug\cay.exe agent-test start <folder> "<prompt>" haiku 4 [auto|default|acceptEdits|plan]   # mode defaults to default (Ask) for tests
.\src-tauri\target\debug\cay.exe agent-test mode <runId> acceptEdits   # switch a run's permission mode (2.8e)
.\src-tauri\target\debug\cay.exe agent-test start <folder> "<prompt>" opencode/deepseek-v4.1-flash-free 6 auto opencode   # an ACP run on OpenCode (2.9b)
.\src-tauri\target\debug\cay.exe agent-test start <folder> "<prompt>" gpt-6-luna 4 default codex   # an ACP run on Codex (codex-acp; uses your Codex login)
.\src-tauri\target\debug\cay.exe agent-test decide <requestId> allow|deny [run]   # also: send, stop, stop-all, show, hide, quit
.\src-tauri\target\debug\cay.exe agent-test look screenshot '{"hwnd":123}' C:\tmp\shot.png   # a desktop look without Claude (also list_windows, focused_window, ui_tree)
.\src-tauri\target\debug\cay.exe agent-test act type_text '{"hwnd":123,"text":"hi"}'   # an action without Claude or approvals (no refs; real input: ask first)
.\src-tauri\target\debug\cay.exe agent-test session-decide <requestId> allow|deny   # a terminal session's held permission request (2.8a; id in the log)
.\src-tauri\target\debug\cay.exe agent-test routine list|save '<draftJson>'|run <id>|due <id>|delete <id>|pause on|off   # JSON args: from Git Bash (C12)
.\src-tauri\target\debug\cay.exe google-test state|refresh|connect|disconnect   # Phase 6: the Google watcher's state and counts in the log
.\src-tauri\target\debug\cay.exe google-test tool gmail_search '{"query":"newer_than:2d"}'   # 6c: one agent tool call, counts/ids in the log (gmail_draft makes a real draft)
# Google client (never commit it): set before `pnpm tauri dev` / `pnpm tauri build`, or use google.clientId in settings.json
$env:CAY_GOOGLE_CLIENT_ID='…apps.googleusercontent.com'; $env:CAY_GOOGLE_CLIENT_SECRET='…'
cd src-tauri; cargo test -- --ignored winhttp   # live: reaches Google over WinHTTP without an account
```

The first `cargo build` compiles ~370 crates (several minutes); later rebuilds of the
`cay` crate take 1–2 minutes in debug. `target\debug` grows by GBs over many rebuilds (it hit ~20 GB and filled
the drive once, and again in 6a and 6c, C14); `cargo clean --profile dev` when space runs low, or delete
`target\debug\incremental`, any `topbar*` files and every `*.rcgu.o` in `target\debug\deps` (crashed builds leave
thousands of them: 4.5 GB in 6c). A rustc `STATUS_ACCESS_VIOLATION` mid-build was the drive filling. With little space,
`$env:CARGO_INCREMENTAL=0` and `cargo test --lib` (skips the staticlib/cdylib) keep a build small.

## Layout

- `src-tauri/src/`: `lib.rs` (setup and command registration), `ipc.rs` (event names and DTOs), `error.rs`,
  `win32.rs` (shared HWND/class/pid helpers), `window/` (create, styles, show/hide, placement: pure `place`, `current`, `keep_at`),
  `hover/` (`mod.rs` = poller thread + commands, `logic.rs` = pure state machine + hit tests, unit-tested),
  `fullscreen.rs`, `apps/` (`enumerate.rs`, `icon.rs`, `activate.rs`), `tray.rs` (tray menu),
  `autostart.rs` (Run-key entry, quoted path, respects Task Manager's enable flag),
  `migrate.rs` (one-time TopBar → Cay moves: data folder copy, Run value; D32),
  `media/` (`mod.rs` = managed state + commands, `watcher.rs` = GSMTC thread + subscriptions,
  `snapshot.rs`, `artwork.rs`, `logic.rs` = pure helpers, unit-tested),
  `activity/` (`mod.rs` = managed `Activities`, `Watcher` harness, Caps Lock read, drag/open commands; `queue.rs` = pure
  priority/coalescing, unit-tested; `sound.rs` = the optional "needs you" sound (2.8f); `settings.rs` = switches in `%APPDATA%\io.github.stanwukong.cay\settings.json`; one thread
  per source: `audio.rs` (volume + Bluetooth), `brightness.rs`, `power.rs`, `screenshots.rs`; `drag.rs`).
  The activity queue is driven by `hover::logic::Machine` (the `Showing` phase), not by the activity module.
  Sources send `activity::Signal` (`Offer(Activity)` or `Retract(key)`); other features start their threads through
  the same `activity::Watcher` harness (`Activities::sender`/`add_watcher`) so shutdown stops them too.
  `sessions/` (terminal sessions of coding agents via their hooks; `claude/` until 2.9a, D47): `forward.rs` =
  `cay.exe hook` (run from `main.rs` before Tauri), `pipe.rs` = per-user pipe + server loop, `process.rs` = ancestor
  chain/liveness, `protocol.rs` + `table.rs` (the session table) = pure and unit-tested, `focus.rs` = which window to
  bring forward, `mod.rs` = managed `Sessions`, the `session-listener` thread, commands and the tray's Connect/Disconnect
  dialog. Claude Code's own parts are in `sessions/claude/`: `hooks.rs` (settings.json merge, pure and unit-tested) and
  `statusline.rs` = `cay.exe statusline` (2.8f: usage limits to the hook pipe, then the user's own status line; the
  `claude` block of Cay's `settings.json`). The pipe names (`cay-claude-…`) and the hook marker kept their names, so
  installed entries keep working. Codex's parts (2.9a) are in `sessions/codex/`: `hooks.rs` (pure: `hooks.json`
  merge, the command, version parsing, `MIN_VERSION`) and `mod.rs` (8.3 path, version check, tray Connect/Finish/
  Update/Disconnect Codex, `codex.hooksSeen` in Cay's `settings.json`). `files.rs` = reading, backing up and writing
  those settings files and Cay's own blocks. Sessions carry an `AgentKind`; the forwarder takes `--agent codex`.
  `agent/patch.rs` (pure) reads Codex's `apply_patch` format for cards, summaries and step lines. OpenCode's parts are
  in `sessions/opencode/`: `plugin.rs` (pure: the `cay.js` template, its marker and states), `tools.rs` (pure:
  OpenCode's tool and argument names → Claude Code's, applied by the forwarder for `--agent opencode`) and `mod.rs`
  (tray Connect/Update/Disconnect OpenCode). `version.rs` = which version of an agent CLI is on PATH. It ignores hooks from sessions Cay itself started (`is_self`).
  Permission requests (2.8a): `ask.rs` (pure: the held request, the reply, the hook output) and `asks.rs` (managed
  `Asks`, the `session-asks` pipe server, one thread per held request, `sessionApproval` cards). Questions and plans
  (2.8c): `agent/interact.rs` (pure: questions, answers, `updatedInput`, plan titles), shared by sessions and runs. They're never allowed bare:
  only `Answers` / `ApprovePlan` allow them, built from the input as Claude Code sent it.
  `duplex.rs` = the duplex, multi-instance pipe shared by the agent pipe (`agent/pipe.rs` names it) and `sessions/asks.rs`.
  `agent/` (Phase 2.7, headless Claude Code runs; other agents through ACP since 2.9b): `mod.rs` = managed `Agents`, pipe server + connection threads,
  approval flow, commands; `run.rs` = command line + spawn (suspended, `CREATE_NO_WINDOW`, Job Object via `job.rs`);
  `bridge.rs` = `cay.exe mcp` (run from `main.rs`, relays stdio JSON-RPC ↔ `pipe.rs`); `acp.rs` (pure: the Agent Client
  Protocol client, ACP messages → `StreamEvent`s, permission answers, the Ask-mode watchdog) and `backend.rs` (finding
  OpenCode, each mode's per-run rules, `ACP_GUIDANCE`, the MCP `Offer`) for runs on other agents (2.9b, D48); `hotkey.rs`, `folder.rs`
  (picker + long paths), `installs.rs` (find `claude.exe`, recent folders), `audit.rs` (jsonl trail, run ids),
  `settings.rs` (`agent` block in settings.json). Pure and unit-tested: `stream.rs` (stream-json), `lifecycle.rs`
  (run state), `approvals.rs`, `describe.rs` (risk, summaries, remember keys), `mcp.rs`, `mode.rs` (2.8e: the cycle,
  `set_permission_mode` lines, when desktop tools follow Claude Code), `preview.rs` (2.8b: the card's
  and the command window's diff or wrapped command; `read_existing` is its one file read).
  Routines (2.7e): `routines.rs` (`routines.json` store, validation, the Windows time zone) and `schedule.rs` (pure:
  due times, DST, missed/overlap verdicts); the `agent-routines` thread and `start_run` (shared with `agent_start`) are
  in `mod.rs`.
  `google/` (Phase 6, D42): `mod.rs` = managed `Google`, the `google-watcher` thread (through `activity::Watcher`),
  sign-in on a `google-auth` thread, tray Connect/Disconnect, commands, the cache; `auth.rs` (PKCE, loopback listener,
  token exchange, refresh, revoke), `http.rs` (WinHTTP client), `vault.rs` (Credential Manager), `calendar.rs`,
  `tasks.rs`, `network.rs` (connectivity hint), `settings.rs` (`google` block), `gmail.rs` (6c: search, read, draft);
  pure and unit-tested with synthetic fixtures (`google/fixtures/`): `logic.rs` (Google's JSON → events and tasks,
  phases, boundaries, dates, backoff), `tools.rs` (6c: the agent tools' arguments, ranges, events and tasks for
  Claude), `mail.rs` (6c: messages → text, HTML → text, RFC 2822 drafts). `run_tool` in `mod.rs` runs one agent call;
  `agent/google_tools.rs` is its MCP side (definitions, approval lines, untrusted replies, audit).
  `win32::local_offset_min` is Windows' time-zone offset (shared with routines).
  `window/command.rs` creates the focusable `command` window (`command.html` → `src/command/`).
  `desktop/` (Phase 2.7c looks, 2.7d actions): `mod.rs` = `run(Look)` on a per-look MTA thread and `act(…)` on a
  per-action one (one action at a time), `windows.rs` (Alt+Tab list from `apps/`, target checks: gone, ours, cloaked,
  hung, elevated, locked), `capture.rs` (PrintWindow / BitBlt + capture exclusion), `encode.rs` (WIC), `uia.rs`,
  `act.rs` (find an element again from a ref, UIA patterns), `input.rs` (`SendInput`), `guard.rs` (low-level hooks:
  your input aborts), `launch.rs` (links, Start-menu apps); pure and tested: `logic.rs`, `tree.rs` (outline with
  `[n]` refs), `plan.rs` (actions, chords, text/secret checks, coordinates, URLs, `ActError`), `refs.rs` (per-run refs
  and screenshot views). `agent/tools.rs` = the four look tools, `agent/actions.rs` = the seven action tools
  (definitions, arguments, ref resolution, replies, redacted audit; pure); `agent/approvals.rs` also holds the
  one-time grants for looks and actions. `window/glow.rs` = the native screen-edge glow window (own thread,
  `look_started`/`look_ended`, stronger with a rim for `act_started`/`act_ended`). `hover::set_hands_off` keeps the
  island collapsed and click-through while an action runs.
- `lib.rs` uses `.build()?.run(|app, event| …)` so `RunEvent::Exit` can stop the media and activity watchers (unregisters their handlers).
- Plugins (registered in `lib.rs`): `tauri-plugin-single-instance` (must stay first). Don't use
  `tauri-plugin-autostart`: it writes the exe path unquoted, which breaks for paths with spaces.
- `prompts/`: kick-off prompts for starting a phase in a fresh session (one per phase, linked from PLAN.md;
  currently 2.5, 2.6, 2.7, 2.7c, 2.7d, 2.7e, 4a, 2.8 (stages b–f), 2.8f, 6 (Google) and 2.9 (other agents), plus `spinner-redesign.md` for
  stage 4a-1).
  When a phase is signed off, write the next one so a new session can pick up without this conversation.
- `src/`: `lib/ipc.ts` + `lib/types.ts`, `island/` (Island, Clock, hooks, `motion.ts`), `features/apps/`,
  `features/media/` (`useMedia` lives in `Island` so state survives the panel remounting; `MediaCard`, `PillMedia`, `useProgress`,
  `artTheme` = album-art colors → `--tint-from`/`--tint-to`/`--accent`, set on `.stage`; those are `@property`-registered in
  `global.css` so they can transition. Use `var(--accent)` for highlights, not a fixed color),
  `features/calendar/` (Phase 6b: `useCalendar` lives in `Island`; `CalendarMark` = Cay's own line calendar (D43: not
  Google's icon, and never a look-alike of it); `CalendarSoon` activity, `CalendarSection` in the panel),
  `features/agent/PreviewBlock.tsx` renders an `ApprovalPreview` exactly as Rust built it (island and command window),
  `features/activities/` (`ActivityView` per kind, `ScreenshotActivity` with the drag/click gesture, `SessionActivity`,
  inline SVG `icons`), `features/sessions/` (`useSessions` lives in `Island`, `WorkingGlyphs.tsx` =
  `WorkingIndicator` (the amber wave while something works) and `AgentMark` (the wave at rest), `StepLabel` (the
  live step line and `n/m`, 2.8d), the panel's `AgentsSection` (heading "Agents"), `AgentTag.tsx` = the lowercase agent name after a session's project (2.9a; shown when the agent isn't Claude or the list mixes agents, never in the pill; `mixedAgents`); `--agent` (amber, with `--agent-rgb`/`-deep`/`-ink`) is a fixed token like `--charge`, mirrored by
  `COLOR` in `window/glow.rs`; never reintroduce Claude Code's spinner, `✻` or orange (D30); Claude content uses
  `--font-agent`, Cascadia Mono). The equaliser uses `--media-accent`: the art's accent, or soft white with no art, `features/agent/` (`useAgentRuns` lives in `Island`, panel rows `AgentRunItems` in AgentsSection's
  `agent` slot; the `agentApproval` view is `features/activities/AgentApproval.tsx`), `command/` (the command window's
  own page: new task form, run list, transcript, follow-ups; `SessionAskView` + `useSessionAsks` for terminal sessions'
  held requests (2.8b); routines: `RoutineList`, `RoutineEditor`, `TaskFields`
  shared with `NewTask`, `useRoutines`, `routineLabels`).
  The island has three shapes (`data-mode`: `pill` | `activity` | `panel`), all driven by Rust events.

## Architecture rules

1. **Rust owns window and hover state.** The cursor poller in `src-tauri/src/hover/` decides
   expanded vs collapsed vs showing an activity, times activities, and toggles click-through
   (`window::set_click_through`). React only animates in response to events and never runs timers to hide things. Never drive expand/collapse from DOM `mouseenter`/`mouseleave`:
   while collapsed the webview gets no mouse events at all.
2. **One window, never activated.** The island window has `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW`.
   It must never take focus from the user's app. Anything that needs keyboard input goes in a *separate*, normal
   window: the `command` window (Phase 2.7), settings (Phase 4b).
3. **IPC names live in one place.** Rust: `src-tauri/src/ipc.rs` (event name consts + payload
   structs). TS: `src/lib/ipc.ts` (typed `invoke`/`listen` wrappers). Components never call
   `invoke("...")` with a string literal.
4. **Win32 is isolated.** All `unsafe` Win32/WinRT code lives in feature modules under
   `src-tauri/src/` (`window/`, `hover/`, `apps/`, `fullscreen.rs`, `media/`, `activity/`, `desktop/`, later
   `processes/`, `thumbnails/`). Each `unsafe` block gets a one-line `// SAFETY:` comment.
   Commands are thin wrappers that return `Result<T, AppError>`.
5. **Payloads are camelCase.** Rust DTOs use `#[serde(rename_all = "camelCase")]`; TS types in
   `src/lib/types.ts` mirror them by hand. Update both sides together.
6. **HWNDs cross IPC as numbers** (`i64` in Rust, `number` in TS). Always revalidate with
   `IsWindow` before acting on one; windows die between list and click.
7. **Feature folders scale by phase.** New phases add a folder on both sides
   (`src/features/<name>/`, `src-tauri/src/<name>/`) plus their entries in `ipc.rs` and `ipc.ts`.
   They don't touch other features' internals.

## Windows gotchas (read before touching Win32 code)

- **`windows` crate version vs Tauri's.** `WebviewWindow::hwnd()` returns the `HWND` type from
  *Tauri's* `windows` version (0.62 as of Tauri 2.12). Ours is pinned to the same minor in
  `Cargo.toml`. After a Tauri upgrade, check `cargo tree -i windows` and bump ours to match.
- **SetForegroundWindow** is restricted for background processes. Use the fallback chain
  in `apps/activate.rs` (restore if minimized → `SetForegroundWindow` → empty synthetic mouse
  `SendInput` and retry → `AttachThreadInput` with the current foreground thread). The empty
  mouse input avoids the side effects of the classic Alt-key trick, such as menu bars activating.
  `activate_window` is a sync command, so it runs on the main thread, which has a message queue.
- **Never use Tauri's `show()` on the island.** It uses `SW_SHOW`, which activates the window. Use
  `window::show_no_activate` (`SW_SHOWNOACTIVATE` + re-assert `HWND_TOPMOST`).
- **Startup visibility race.** On a cold start Tauri can apply the builder's `visible(false)`
  *after* our show in `setup()`. The poller therefore re-shows the window every 250 ms
  whenever it should be visible and isn't.
- **Display changes are polled, not cached (D31).** The poller re-reads the primary monitor every 250 ms
  (`window::current`) and re-places the island with `window::keep_at` (async, never activating); it also undoes Tao's
  own `WM_DPICHANGED` resize. Don't keep a copy of `Geometry` anywhere else. A changed geometry clears React's rect and
  emits `monitor:changed`. Test scale changes with `SystemParametersInfo(SPI_SETLOGICALDPIOVERRIDE = 0x9F, step,
  …, SPIF_UPDATEINIFILE)` (relative to the recommended scale; read and restore the step with `0x9E`), resolution with
  `ChangeDisplaySettings` without `CDS_UPDATEREGISTRY` (temporary; reset with `ChangeDisplaySettings(NULL, 0)`): ask
  first. Measuring helpers must call `SetProcessDpiAwarenessContext(-4)` first, or other windows' rects come back scaled
  to 96 DPI (C13).
- **Never call Tauri's `set_ignore_cursor_events` on the island.** It toggles `WS_EX_LAYERED` along
  with `WS_EX_TRANSPARENT`, and DWM drops the surface of a WebView2 window for ~150 ms whenever
  `WS_EX_LAYERED` changes, so the island visibly blinks out. Use `window::set_click_through(hwnd, bool)`.
  It keeps the window permanently layered and flips only `WS_EX_TRANSPARENT`, with no `SWP_FRAMECHANGED`.
  The poller re-asserts it every 250 ms in case Tao rewrites the style.
- **Autostart registers the exe that toggled it.** Turning on "Launch at startup" from a dev build
  writes `target\debug\cay.exe` into the Run key. Toggle it from the installed build.
- **Run-key paths must be quoted** (`"C:\Users\DELL 7300\…\cay.exe"`). Unquoted paths with spaces
  don't show in Task Manager's Startup apps and are a hijack risk. `HKCU\…\Explorer\StartupApproved\Run`
  holds Task Manager's own switch (odd first byte = disabled): read it in `is_enabled`, and clear it when the user toggles from the tray.
- **Single instance blocks dev while the installed app runs** (same identifier). Quit it from the tray first. An
  installed *TopBar* (the old identifier) is a different app to Windows: quit it too, or two islands run.
- **The TopBar → Cay rename (D32) is migrated once, in `migrate.rs`**, first thing in `setup`. Keep `OLD_IDENTIFIER`,
  `autostart::OLD_VALUE_NAME` and `hooks::OLD_MARKER` until v1; the data copy only runs while Cay's folder doesn't
  exist (test it by renaming that folder aside, not by deleting TopBar's).
- **WebView2 background throttling can't be disabled on Windows** (Tauri's
  `background_throttling` is unsupported there). Don't build on JS timers running while idle.
- **Alt+Tab filter:** visible, non-empty title, root owner + last active popup equals
  itself, not `WS_EX_TOOLWINDOW` (unless `WS_EX_APPWINDOW`), not DWM-cloaked
  (`DWMWA_CLOAKED`), not our own window.
- **Transparent windows still eat clicks** over their transparent pixels while not ignoring
  cursor events. That's why we only accept cursor events while the pointer is inside the
  reported panel rect.
- **DPI and Windows "Text size".** Win32 cursor/monitor APIs use physical px. CSS px in the webview
  are physical ÷ `devicePixelRatio`, and WebView2 folds Windows' *Text size* accessibility
  setting into that ratio. The monitor scale factor Rust sees does **not** include it (on the
  dev machine: scale 1.0, text size 120%, so dpr 1.2). Therefore React → Rust geometry is always sent in
  **physical px** (CSS × `devicePixelRatio`), and CSS must fit the viewport rather than assume
  640 CSS px are available. Never mix physical and CSS px in one struct.
- **Icons:** `WM_GETICON` through `SendMessageTimeoutW` (hung apps), then `GetClassLongPtrW`,
  then the exe icon via `SHGetFileInfoW`. Always `DestroyIcon` what you own.
- **WinRT async/collections need their own crates.** `windows` re-exports only `windows-core`, so
  `IAsyncOperation` comes from `windows-future` and `IVectorView` from `windows-collections`. Keep their versions
  in step with what `windows` pulls in (`cargo tree -i windows-future`). Blocking `.join()` is fine on our own MTA
  threads (`media-watcher`); in async commands, `.await` the operation instead.
- **GSMTC events fire on thread-pool threads.** Handlers must only send a message to the `media-watcher` thread;
  never touch shared state or call back into WinRT from them. Hold every registration in a `Subscription` guard
  so it's removed on drop. GSMTC objects are agile (`Send + Sync`), so commands can use a cloned session directly.
- **Media apps fire events in bursts and in odd orders.** Spotify often sends a track's title before its thumbnail,
  so artwork misses are not cached and are retried. Timeline events are emitted to React only when they disagree
  with its interpolation (`logic::worth_emitting`); keep `positionAt` in `useProgress.ts` in step with `expected_position`.
- **`#[implement]` needs `windows-core` as a direct dependency** (the macro expands to `windows_core::` paths). Keep it
  on the same minor as `windows`. `CreateFileW` and `CreateEventW` are gated behind the `Win32_Security` feature.
- **Core Audio callbacks** (`IMMNotificationClient`, `IAudioEndpointVolumeCallback`) run on system threads and must not
  call back into the device API: forward a message to `audio-watcher`, like GSMTC handlers.
- **Power notifications need a window.** `power.rs` owns a message-only window (`HWND_MESSAGE`) on its own thread. Never
  register anything against the island's HWND.
- **Screenshot drag-out** must be a *sync* command (main thread: OLE STA, owns the island) started while the mouse button
  is still down. Don't call `disable_drag_drop_handler()` on the island: WebView2's own drop target then accepts files
  dropped on it (C6). Tauri's handler intercepts them harmlessly.
- **Activities and hit tests:** only interactive activities (the screenshot) take clicks, and only while hovered.
  `set_panel_rect` reports carry a `mode`; reports for the shape Rust isn't showing are ignored.
- **Testing activities from outside without side effects:** toggle Caps Lock with SendInput and toggle it back; volume
  up N then down N; brightness via WMI `WmiSetBrightness` and restore; copy an image into the Screenshots folder (then
  delete it). Read `GWL_EXSTYLE` (`WS_EX_TRANSPARENT`) to check click-through.
- **Codex hooks (2.9a, D47).** Codex 0.166 runs a hook's command through `powershell.exe -NoProfile -Command`
  (cmd in older versions): a quoted path runs nothing there and shows "Hook failed" on every session start, so the
  entry is the exe's unquoted 8.3 path (`& '…'` only without 8.3 names). Codex skips new or changed hooks until you
  trust them with `/hooks`; `--dangerously-bypass-hook-trust` is for scratch tests only (Claude Code's auto mode
  refuses it unless you allow the exact command). `codex exec` never fires `PermissionRequest` (it reports
  `bypassPermissions`): test permission cards in the TUI. Test with a scratch `CODEX_HOME` (its own `codex login`, so
  test runs never share a refresh token with your real login); never write the real `~\.codex\hooks.json` in tests.
  `SessionEnd` and `Interrupt` time out after 3 s at most.
- **OpenCode (2.9a, D47).** Its `permission.ask` plugin hook is declared but never called (1.19.5): Cay's plugin answers
  `permission.asked` through OpenCode's own client instead, and kills its waiting forwarder on `permission.replied`.
  `opencode run` rejects every request at once, so test cards in the TUI or `opencode serve`. Connect never edits the
  user's `opencode.json` (OpenCode allows edits and shell by default). Test with `OPENCODE_CONFIG_DIR` pointing at a
  scratch folder; OpenCode creates its own defaults in `~\.config\opencode` on first run, Cay doesn't.
- **ACP runs (2.9b, D48).** An ACP agent gets its mode's rules for one process through its environment
  (`OPENCODE_CONFIG_CONTENT`), never by writing its config; a mode change applies from the next message. OpenCode marks a
  step `in_progress` before it asks, so the watchdog judges completed steps. A free model can stall a run with no output
  (Stop ends it); test ACP runs with `agent-test start … opencode` in a scratch folder and decide with `agent-test
  decide` (you may click the card first). Its `opencode.exe` is the real exe behind npm's shim
  (`node_modules\opencode-ai\bin`). Codex runs through `@agentclientprotocol/codex-acp` (not Zed's deprecated
  package), started as `node.exe <dist\index.js>`; its "reject once" cancels the whole turn. An ACP mode the agent refuses
  ends the run; the first message waits for the mode. An ACP run silent for 10 minutes is stopped (`ACP_SILENCE`).
  A `pnpm tauri dev` started in the background and then stopped can leave Vite on port 1420 and `cay.exe` running:
  free the port and quit Cay before starting another, and never send `agent-test quit` with no Cay running (it starts
  one).
- **The hook forwarder must stay quiet and fast.** `cay.exe hook` runs before Tauri, never panics
  (release is `panic = "abort"`, a non-zero exit shows as a hook error in Claude Code) and bounds every wait. The only
  thing it ever prints is a `PermissionRequest` decision you made in the island (2.8a); with no decision it prints
  nothing, so Claude Code asks as usual. Keep Cay's hold (`asks::hold_for`: 55 s for permissions, 600 s for questions
  and plans) < the forwarder's wait (+5 s) < the hook entry's timeout (660 s), and never answer a request nobody
  clicked: timeouts, errors and Cay's own runs reply `pass`. Questions and plans are never allowed bare: only
  `Answers` / `ApprovePlan` allow them, built from the original input. Keep
  the hook entry in *shell form* with an explicit `shell`: Claude Code before 2.1.15x ignores the `args` exec form
  and would run a bare `cay.exe`, launching Cay. "Connect Claude Code…" records the exe that clicked it (as with
  autostart), so connect from the installed build.
- **Testing Claude Code hooks without touching `~/.claude/settings.json`:** pipe hook JSON *files* into
  `target\debug\cay.exe hook` (shells mangle backslashes in inline JSON, C7), or run
  `claude -p "…" --settings <file with the hook entries>`. Never write the real settings in tests; `hooks.rs` tests
  use a temp folder, and `CLAUDE_CONFIG_DIR` points the tray item at another folder. Write hook JSON files with the
  Write tool, not a Git Bash heredoc (it collapsed the doubled backslashes and the forwarder silently ignored the invalid
  JSON). For `PermissionRequest` in `-p` mode use a recent Claude Code (2.1.314 under the VS Code extension runs it;
  2.1.123 on PATH doesn't), a scratch folder and a command that really asks (`touch`, not `echo`).
- **`cay.exe statusline` (2.8f) is a status line, so it must print exactly the user's output.** It never prints
  anything of its own, sends only `rate_limits` to the pipe (the input holds paths, costs and session names), runs the
  user's command through the same shell Claude Code would (Git Bash, else PowerShell) with `CREATE_NO_WINDOW`, and
  bounds everything (stdin 1 s, connect 1 s, their command 10 s). Connect keeps their `statusLine` in Cay's
  `settings.json` (`claude`); never put Cay's own command there, and test with `APPDATA` pointed at a scratch folder.
- **The `PreToolUse` hook (2.8d) is background-only and summarized in the forwarder.** It must never print or block:
  a `PreToolUse` output can allow or deny a tool. Only the step line and to-do counts go on the pipe, never the input.
- **Editing Rust while `pnpm tauri dev` runs restarts Cay**, and a `cay.exe agent-test …` sent during the rebuild finds
  no instance and starts a full Cay of its own; the dev build's relaunch then hands off to it and exits. Check
  `Get-Process cay` before driving tests, and stop a stray instance.
- **Agent runs use Claude Code's permission modes, never bypass (D40).** Runs start in the mode the user picked (Auto
  by default; Ask, Accept edits, Plan) and always pass `--permission-prompt-tool mcp__cay__approve`, so whatever
  Claude Code asks about reaches the island. Never `bypassPermissions`, `dontAsk` or `--dangerously-skip-permissions`
  (`AgentMode` can't hold them), never change a terminal session's mode, and never write the user's
  `~/.claude/settings.json` for a run (per-run files live under `%APPDATA%\io.github.stanwukong.cay\
  agent\runs\<runId>\`, deleted when the process exits). The prompt tool's reply is a text block holding
  `{"behavior":"allow","updatedInput":…}` / `{"behavior":"deny","message":…}`.
- **Agent processes:** `claude.exe` must start `CREATE_SUSPENDED`, be assigned to its Job Object, then resumed
  (`job.rs`), or children it starts at once escape Stop. The `claude` on PATH may be an npm shim (no `.exe`): spawn the
  real exe (`installs.rs`). Pass folders through `folder::long_path`: with an 8.3 cwd Claude Code asks before reading
  its own files.
- **Overlapped reads that complete at once** still need `GetOverlappedResult` for the byte count (C8). Both pipes do.
- **A newly created command window misses events sent while it loads.** `window::command` keeps the focus it was
  opened for, and the page takes it once on load (`command_take_focus`). Do the same for anything else it must select.
- **Building a webview window** from a sync command or the main thread deadlocks on Windows: `window::command::show`
  is called from the hotkey thread and async commands. The hotkeys use `RegisterHotKey(NULL)` on their own thread
  (`WM_HOTKEY` goes to the thread queue); receiving one is what lets the command window take the foreground.
- **Desktop looks never capture Cay.** Every window of our own process is refused by pid (island, command window,
  glow). For a screen capture, `capture.rs` sets `WDA_EXCLUDEFROMCAPTURE` on our visible windows, `DwmFlush`es twice,
  captures, and restores `WDA_NONE`; don't make it permanent on the island (it would vanish from the user's own
  screenshots). The glow is always excluded, so it can't be screenshotted: check it through its window
  (`IsWindowVisible`, `GetWindowDisplayAffinity` = 17). The island hides during fullscreen, so test exclusion with it
  visible. Never return a password field's value (`tree.rs` drops it even if a walk collected one).
- **Images for Claude** stay ≤1568 px long edge and ≤500 KB (Claude Code re-compresses over 512,000 bytes and cuts
  results over `MAX_MCP_OUTPUT_TOKENS`), and replies must stay well under the agent pipe's 4 MiB cap (an oversized reply
  is dropped and Claude Code waits). Claude Code stores tool images in its session transcripts under `~/.claude/projects`.
- **The Win32 `VARIANT` has no `Drop`:** `VariantClear` what UIA's `GetCachedPropertyValue` returns.
- **Desktop actions never act on a guess.** An element is found again by its runtime id (at its old position, then
  `FindAll`) and must still match type, name and automation id; a raw click point must be on the target
  (`WindowFromPoint` root) and the target in front before every `SendInput` batch. Your input aborts: the low-level
  hooks in `guard.rs` count events without `LLKHF_INJECTED`/`LLMHF_INJECTED` (`GetLastInputInfo` counts ours too);
  their callback only stores atomics (Windows silently drops slow LL hooks). An action first waits until you've let
  go for 500 ms (the Allow click itself is your input). Always release keys that went down. Input actions are never
  remembered "for this run"; typed text is redacted in the audit (`actions::redact`). Win-key chords, Alt+Tab and
  Ctrl(+Shift)+Esc are refused; `launch` takes https/http/ms-settings links and Start-menu apps only.
- **Synthetic typing is paced on purpose (C11).** Windows 11 Notepad turns a burst of `KEYEVENTF_UNICODE` events into
  copies of the last character, and drops keys or loses Shift when real key events arrive together. `input.rs` types
  real keys from the target's layout (`VkKeyScanExW`, Shift and Caps Lock handled), one event per `SendInput`, 25 ms
  apart and 60 ms after a modifier goes down. Don't batch it back up; check typed text by reading it back (UIA or a
  screenshot), not by `SendInput`'s count, and compare with WordPad when an app looks wrong.
- **Testing agent runs:** use `cay.exe agent-test …` (debug builds only), a scratch folder, `haiku` and low
  max turns. Don't send synthetic keystrokes while the user is at the machine; they land in whatever has focus.
  UI Automation actions (`invoke`, `set_value`) send no input and work with the window behind; ask before testing
  `focus_window`, `click`, `type_text`, `press_keys` or `launch`.
- **Routines run like a new task.** A routine's run goes through `start_run`; its only differences are `--name`, its
  own mode (Auto for new routines, Ask for ones saved before 2.8e), the desktop tools disallowed unless the routine
  allows them, and patient approvals (timeout paused while you're away, 12 h cap) for scheduled runs. Don't add allow
  lists to routines. Schedules are
  checked by the `agent-routines` thread (at least once a minute; never a JS timer). `routines.json` is Cay's file:
  test with `agent-test routine …`, and test due times with `routine due <id>` or a time two minutes ahead, never by
  changing the clock.
- **Google (Phase 6) has secrets and network code; keep them narrow.** The refresh token lives only in Credential
  Manager (`Cay/google`); the access token only in the watcher's memory; never log a token, an event title or a
  description (the debug command logs counts and times). The OAuth client comes from `CAY_GOOGLE_CLIENT_ID` /
  `CAY_GOOGLE_CLIENT_SECRET` at build time (`option_env!`, `build.rs` reruns on change) or `google.clientId`; never commit
  one. HTTP goes through `google/http.rs` (WinHTTP, Windows' TLS and proxy): don't add reqwest/ureq/rustls. Sign-in is
  the system browser and a one-shot `127.0.0.1` listener that checks `state`; never a webview (Google refuses them).
  Links stay in Rust (https only, meeting hosts checked in `logic.rs`) and open by id. Tests use the fixtures, never a
  live account; ask before anything that signs in, registers with Google or sends a request as the author. Calendar
  sync tokens can't be combined with `timeMin`/`timeMax`/`orderBy`: keep the windowed query. Task `due` is a date
  (midnight UTC), compared with the local date.
- **Google tools for agent runs (6c, D46).** The access token stays the watcher's: a tool call borrows it per call
  through `Wake::Token` (never stored elsewhere). Gmail is opt-in (`google.gmail`), never follows Auto (Cay asks), and a
  draft is never remembered. Mail is someone else's words: replies say so (`<email-data>`), and `read_mail` makes the
  run's desktop actions ask in Cay even in Auto; keep both. Never add a send call or `gmail.send`; `gmail.compose` is
  the only write scope. The audit and the debug command log counts and ids, never mail, titles or draft text (redact
  with `agent::redact`). Gmail ids go into URL paths only after `mail::check_id`; draft headers only after
  `check_draft` (no CR/LF). Test with the fixtures; a live call or draft needs the author's go-ahead.
- **Windows PowerShell 5.1 can't list GSMTC sessions** (`GetSessions()` comes back empty). Check media state with the
  `cay_lib=debug` log instead.

## Conventions

- Rust: `cargo fmt`, clippy clean, `thiserror` for `AppError`, `log` macros (no `println!`).
- TS: strict mode, function components + hooks, no default exports except `App`.
- Animation: Framer Motion `layout` + spring transitions. Keep timing constants in
  `src/island/motion.ts`. Ambient loops (e.g. the charging ring) are CSS keyframes on `@property` values,
  paused unless their state is showing, with a `prefers-reduced-motion` fallback.
- **UI changes go through the `motion-designer` agent** ([.claude/agents/motion-designer.md](.claude/agents/motion-designer.md)):
  any change to components, CSS, icons or motion in `src/`. It holds the visual and motion rules (minimal, native,
  token-based colors, Rust-owned state) and how to verify visuals (headless Edge renders or captures of the real island).
  Keep that file current when the motion system changes.
- Commits: small, imperative subject line; reference the phase (e.g. `phase1: alt-tab filter`).
- **Branches: do not create new branches unless explicitly asked to. All work is committed and
  pushed to `main`.**
