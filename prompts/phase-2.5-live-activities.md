Start Phase 2.5 (Live Activities) of TopBar.

Read PLAN.md first: Current status, Roadmap, "Phase 2.5: Live Activities", the IPC contract (including
the reserved `activity:show` event) and Known limitations. Then read the CASESTUDY.md decision log
(especially D1, D2 and D7 for hover/geometry, and D11 and D12 for the Phase 2 watcher pattern). CLAUDE.md
is loaded; follow its rules, especially the Windows gotchas.

Scope: everything in PLAN.md's Phase 2.5 checklist.
- An activity system in Rust. Event sources push activities into a queue that owns priority and
  coalescing. A burst of volume key presses must update one activity in place, not queue ten of them.
  Rust emits `activity:show { kind, payload, durationMs }`, and the island expands to a compact
  activity size for a few seconds, then collapses.
- Sources: volume (and mute) changes, brightness changes, Bluetooth audio connect/disconnect (with
  battery level where Windows exposes it), charger plugged/unplugged and low battery, Caps Lock toggled,
  and screenshot taken (with a thumbnail you can drag out to drop the file somewhere).
- Per-activity on/off switches, stored now. Their UI comes in Phase 4b.

Constraints that matter here:
- Rust owns window and hover state (rule 1). An activity is a new state in the hover state machine
  (`hover/logic.rs`), not a React-side expand. Hovering the hot zone during an activity must still open
  the full panel. An activity must never interrupt an expanded panel; queue it or drop it.
- While an activity is showing and the cursor isn't on it, the island stays click-through. Decide
  whether hovering or clicking an activity does anything, and how that fits the panel-rect hit test (D2, D7).
- The island never activates (rule 2). Check that dragging a screenshot out of a `WS_EX_NOACTIVATE`,
  click-through-by-default window is feasible before committing to it. If it isn't, propose a fallback.
- Keep the activity queue generic. Phase 2.6 (Claude Code activity, see PLAN.md) will push activities
  into it from a named-pipe listener, not a system API, including one that should stay up until handled.
  Don't hard-code the source list into the queue or the hover state machine.
- Reuse the Phase 2 pattern for event sources: a dedicated thread owns the subscriptions, callbacks only
  forward a message, and registrations are RAII guards removed on drop and on `RunEvent::Exit`.
- Prefer polling inside the existing 60 Hz poller over a global low-level hook where it's cheap (D1).
  Caps Lock is the obvious candidate.
- The island's colors come from `--tint-from`/`--tint-to`/`--accent` (album-art theme, D13). Activities
  should look right with and without a media theme.
- No JS timers that need to run while idle (WebView2 throttling). Durations are timed in Rust.

Before coding:
1. Check the `windows` 0.62 crate in the cargo registry sources for each source's API and the feature
   names it needs. Likely candidates: Core Audio `IAudioEndpointVolume` + `IAudioEndpointVolumeCallback`
   and default-device changes through `IMMNotificationClient`; brightness through power-setting
   notifications or WMI; `RegisterPowerSettingNotification` / `WM_POWERBROADCAST` for power (these need
   a message-only window on its own thread, never the island's HWND); Bluetooth through
   `Windows.Devices.Enumeration` / `Windows.Devices.Bluetooth`; screenshots through a watch on the
   Screenshots folder. Say which ones are unreliable or need COM callbacks implemented in Rust
   (`#[implement]`).
2. Propose: the module layout on both sides, the threading model and how each subscription is kept
   alive and torn down, the hover state machine changes (new phases and transitions, with unit tests),
   the queue's priority and coalescing rules, the exact `activity:show` payloads per kind (and whether
   a new `activity:hide` event or command is needed), where the per-activity switches are stored, and
   the compact activity layout.
3. Wait for my OK.

When done: cargo fmt / clippy / test and pnpm typecheck clean. Verify what you can from outside the
app, for example toggling Caps Lock with SendInput (and toggling it back), screenshotting the island,
and checking click-through through the window's extended style. Don't change my volume or brightness
without restoring it. Give me a manual test list (volume keys, brightness keys, Bluetooth headphones,
charger, Caps Lock, Win+PrtScn and dragging the thumbnail out, an activity while the panel is open,
hovering during an activity, fullscreen apps). Update PLAN.md, CASESTUDY.md and CLAUDE.md, then
commit and push to main.
