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

### Still open
- **Foreground-lock rules** in practice: the fallback chain is implemented but not yet exercised
  against elevated or minimized targets (manual test 7).
- **Testing approach that worked:** with no UI test harness, driving the real cursor with
  `SetCursorPos` and reading the island's `GWL_EXSTYLE` (click-through bit = expanded state) from
  PowerShell gave reliable end-to-end checks of the Rust state machine without touching the app.

## 6. Timeline

| Date | Milestone |
|---|---|
| 2026-10-28 | Phase 0: environment audit, Tauri 2 API research, architecture + IPC contract drafted |
| 2026-10-28 | Phase 1 built: island window, hover state machine, pill ⇄ panel animation, running apps with icons, activation, fullscreen hiding. Fixed text-size DPI mismatch and the startup visibility race. |
| 2026-10-28 | Fixed collapse flash (own click-through toggling, permanently layered window); size-optimized release profile. |
| 2026-10-28 | Phase 1 signed off: all 11 manual tests pass. Roadmap reprioritized (D9). |
| 2026-10-28 | Phase 1.5 built: tray menu, launch at startup, single instance (verified: second launch exits). |
| 2026-10-28 | Replaced `tauri-plugin-autostart` with `autostart.rs` (quoted Run path; C5). |

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
