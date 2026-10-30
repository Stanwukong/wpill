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

**TopBar** (working name, repo `wpill`): a Windows 11 "Dynamic Island"-style bar pinned to
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
$env:RUST_LOG='info,topbar_lib=debug'; pnpm tauri dev   # + hover transitions, panel-rect, media:state and activity:show logs
pnpm tauri build            # release build + NSIS installer (~11 min cold: fat LTO; ~1.5 MB installer)
cd src-tauri; cargo fmt; cargo clippy --all-targets; cargo test
pnpm typecheck              # tsc --noEmit
# Debug builds only: drive agent runs from outside (forwarded to the running TopBar; results in its log)
.\src-tauri\target\debug\topbar.exe agent-test start <folder> "<prompt>" haiku 4
.\src-tauri\target\debug\topbar.exe agent-test decide <requestId> allow|deny [run]   # also: send, stop, stop-all, show, hide, quit
```

The first `cargo build` compiles ~370 crates (several minutes); later rebuilds of the
`topbar` crate take 1–2 minutes in debug. `target\debug` grows by GBs over many rebuilds (it hit ~20 GB and filled
the drive once); `cargo clean --profile dev` when space runs low.

## Layout

- `src-tauri/src/`: `lib.rs` (setup and command registration), `ipc.rs` (event names and DTOs), `error.rs`,
  `win32.rs` (shared HWND/class/pid helpers), `window/` (create, styles, show/hide),
  `hover/` (`mod.rs` = poller thread + commands, `logic.rs` = pure state machine + hit tests, unit-tested),
  `fullscreen.rs`, `apps/` (`enumerate.rs`, `icon.rs`, `activate.rs`), `tray.rs` (tray menu),
  `autostart.rs` (Run-key entry, quoted path, respects Task Manager's enable flag),
  `media/` (`mod.rs` = managed state + commands, `watcher.rs` = GSMTC thread + subscriptions,
  `snapshot.rs`, `artwork.rs`, `logic.rs` = pure helpers, unit-tested),
  `activity/` (`mod.rs` = managed `Activities`, `Watcher` harness, Caps Lock read, drag/open commands; `queue.rs` = pure
  priority/coalescing, unit-tested; `settings.rs` = switches in `%APPDATA%\dev.wpill.topbar\settings.json`; one thread
  per source: `audio.rs` (volume + Bluetooth), `brightness.rs`, `power.rs`, `screenshots.rs`; `drag.rs`).
  The activity queue is driven by `hover::logic::Machine` (the `Showing` phase), not by the activity module.
  Sources send `activity::Signal` (`Offer(Activity)` or `Retract(key)`); other features start their threads through
  the same `activity::Watcher` harness (`Activities::sender`/`add_watcher`) so shutdown stops them too.
  `claude/` (Claude Code via hooks): `forward.rs` = `topbar.exe hook` (run from `main.rs` before Tauri), `pipe.rs` =
  per-user pipe + server loop, `process.rs` = ancestor chain/liveness, `protocol.rs` + `sessions.rs` + `hooks.rs`
  (settings.json merge) = pure and unit-tested, `focus.rs` = which window to bring forward, `mod.rs` = listener,
  commands and the tray's Connect/Disconnect dialog. It ignores hooks from sessions TopBar itself started (`is_self`).
  `agent/` (Phase 2.7, headless Claude Code runs): `mod.rs` = managed `Agents`, pipe server + connection threads,
  approval flow, commands; `run.rs` = command line + spawn (suspended, `CREATE_NO_WINDOW`, Job Object via `job.rs`);
  `bridge.rs` = `topbar.exe mcp` (run from `main.rs`, relays stdio JSON-RPC ↔ `pipe.rs`); `hotkey.rs`, `folder.rs`
  (picker + long paths), `installs.rs` (find `claude.exe`, recent folders), `audit.rs` (jsonl trail, run ids),
  `settings.rs` (`agent` block in settings.json). Pure and unit-tested: `stream.rs` (stream-json), `lifecycle.rs`
  (run state), `approvals.rs`, `describe.rs` (risk, summaries, remember keys), `mcp.rs`.
  `window/command.rs` creates the focusable `command` window (`command.html` → `src/command/`).
- `lib.rs` uses `.build()?.run(|app, event| …)` so `RunEvent::Exit` can stop the media and activity watchers (unregisters their handlers).
- Plugins (registered in `lib.rs`): `tauri-plugin-single-instance` (must stay first). Don't use
  `tauri-plugin-autostart`: it writes the exe path unquoted, which breaks for paths with spaces.
- `prompts/`: kick-off prompts for starting a phase in a fresh session (one per phase, linked from PLAN.md;
  currently 2.5, 2.6 and 2.7).
  When a phase is signed off, write the next one so a new session can pick up without this conversation.
- `src/`: `lib/ipc.ts` + `lib/types.ts`, `island/` (Island, Clock, hooks, `motion.ts`), `features/apps/`,
  `features/media/` (`useMedia` lives in `Island` so state survives the panel remounting; `MediaCard`, `PillMedia`, `useProgress`,
  `artTheme` = album-art colors → `--tint-from`/`--tint-to`/`--accent`, set on `.stage`; those are `@property`-registered in
  `global.css` so they can transition. Use `var(--accent)` for highlights, not a fixed color),
  `features/activities/` (`ActivityView` per kind, `ScreenshotActivity` with the drag/click gesture, `ClaudeActivity`,
  inline SVG `icons`), `features/claude/` (`useClaudeSessions` lives in `Island`, `ClaudeSpinner`/`ClaudeMark`, the
  panel's `ClaudeSection`; `--claude` is a fixed token like `--charge`; Claude content uses `--font-claude`,
  Cascadia Mono), `features/agent/` (`useAgentRuns` lives in `Island`, panel rows `AgentRunItems` in ClaudeSection's
  `agent` slot; the `agentApproval` view is `features/activities/AgentApproval.tsx`), `command/` (the command window's
  own page: new task form, run list, transcript, follow-ups).
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
   `src-tauri/src/` (`window/`, `hover/`, `apps/`, `fullscreen.rs`, `media/`, `activity/`, later
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
- **Never call Tauri's `set_ignore_cursor_events` on the island.** It toggles `WS_EX_LAYERED` along
  with `WS_EX_TRANSPARENT`, and DWM drops the surface of a WebView2 window for ~150 ms whenever
  `WS_EX_LAYERED` changes, so the island visibly blinks out. Use `window::set_click_through(hwnd, bool)`.
  It keeps the window permanently layered and flips only `WS_EX_TRANSPARENT`, with no `SWP_FRAMECHANGED`.
  The poller re-asserts it every 250 ms in case Tao rewrites the style.
- **Autostart registers the exe that toggled it.** Turning on "Launch at startup" from a dev build
  writes `target\debug\topbar.exe` into the Run key. Toggle it from the installed build.
- **Run-key paths must be quoted** (`"C:\Users\DELL 7300\…\topbar.exe"`). Unquoted paths with spaces
  don't show in Task Manager's Startup apps and are a hijack risk. `HKCU\…\Explorer\StartupApproved\Run`
  holds Task Manager's own switch (odd first byte = disabled): read it in `is_enabled`, and clear it when the user toggles from the tray.
- **Single instance blocks dev while the installed app runs** (same identifier). Quit it from the tray first.
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
- **The hook forwarder must stay quiet and fast.** `topbar.exe hook` runs before Tauri, never prints, never panics
  (release is `panic = "abort"`, a non-zero exit shows as a hook error in Claude Code) and bounds every wait. Keep
  the hook entry in *shell form* with an explicit `shell`: Claude Code before 2.1.15x ignores the `args` exec form
  and would run a bare `topbar.exe`, launching TopBar. "Connect Claude Code…" records the exe that clicked it (as with
  autostart), so connect from the installed build.
- **Testing Claude Code hooks without touching `~/.claude/settings.json`:** pipe hook JSON *files* into
  `target\debug\topbar.exe hook` (shells mangle backslashes in inline JSON, C7), or run
  `claude -p "…" --settings <file with the hook entries>`. Never write the real settings in tests; `hooks.rs` tests
  use a temp folder, and `CLAUDE_CONFIG_DIR` points the tray item at another folder.
- **Agent runs never weaken permissions.** TopBar always passes `--permission-mode default` and
  `--permission-prompt-tool mcp__topbar__approve`; never `bypassPermissions` or `--dangerously-skip-permissions`, and
  never write the user's `~/.claude/settings.json` for a run (per-run files live under `%APPDATA%\dev.wpill.topbar\
  agent\runs\<runId>\`, deleted when the process exits). The prompt tool's reply is a text block holding
  `{"behavior":"allow","updatedInput":…}` / `{"behavior":"deny","message":…}`.
- **Agent processes:** `claude.exe` must start `CREATE_SUSPENDED`, be assigned to its Job Object, then resumed
  (`job.rs`), or children it starts at once escape Stop. The `claude` on PATH may be an npm shim (no `.exe`): spawn the
  real exe (`installs.rs`). Pass folders through `folder::long_path`: with an 8.3 cwd Claude Code asks before reading
  its own files.
- **Overlapped reads that complete at once** still need `GetOverlappedResult` for the byte count (C8). Both pipes do.
- **Building a webview window** from a sync command or the main thread deadlocks on Windows: `window::command::show`
  is called from the hotkey thread and async commands. The hotkeys use `RegisterHotKey(NULL)` on their own thread
  (`WM_HOTKEY` goes to the thread queue); receiving one is what lets the command window take the foreground.
- **Testing agent runs:** use `topbar.exe agent-test …` (debug builds only), a scratch folder, `haiku` and low
  max turns. Don't send synthetic keystrokes while the user is at the machine; they land in whatever has focus.
- **Windows PowerShell 5.1 can't list GSMTC sessions** (`GetSessions()` comes back empty). Check media state with the
  `topbar_lib=debug` log instead.

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
