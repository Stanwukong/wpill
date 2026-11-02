Complete stage 2.8f of Cay ("Connect you can trust, and small things"), the last stage of Phase 2.8 (agent UX).

Read PLAN.md first: Current status, "Phase 2.8: agent UX" (2.8a–e are built; their items, manual checks and the IPC
table "Phase 2.8 (a–b)" show what exists), Known limitations and Open questions. Then CASESTUDY.md D36–D40 (how
terminal sessions' requests are held, previews, questions and plans, the step line, and D40: runs now use Claude
Code's own permission modes, Auto by default, never bypass). CLAUDE.md is loaded; follow its rules and Windows
gotchas, above all rule 1 (Rust owns state, no timers in React), rule 2 (the island is never activated), the hook
forwarder's rules (quiet, every wait bounded, prints only a decision you made) and "Testing Claude Code hooks without
touching `~/.claude/settings.json`".

Where 2.8a–e left things (2026-11-02): Connect writes nine hook entries (`hooks::EVENTS`: background ones, `SessionEnd`
waiting, `PreToolUse` in the background for the step line, `PermissionRequest` held with a 660 s timeout).
`hooks::installed` already counts an install as connected only when every entry matches what this version writes
(timeout and `async`), so an old install reads as not connected; the tray then offers "Connect" again, not "Update".
The author's own `~/.claude/settings.json` has a `statusLine` (`node "C:\Users\DELL 7300\.claude\hud\omc-hud.mjs"`).

## Work (each item ends with its doc updates; one commit for the stage, or one per item if they're large)

1. **Connect you can trust.** Connect already shows the change and keeps a dated backup (`claude/mod.rs` `run_dialog`,
   `hooks::preview`). Check, with tests on a temp folder (never the real settings): that the preview matches exactly
   what's written; that Disconnect removes only Cay's entries (and TopBar's old ones) and leaves everything else,
   key order included; and give the hook state three values (connected, out of date, not connected) so the tray reads
   "Update Claude Code connection…" for an out-of-date install instead of looking disconnected. Close the gaps; don't
   rebuild what works.
2. **Usage limits (5 h / 7 d) in the panel's Claude section**, from Claude Code's status line input, no network call.
   Decided with the author (chain theirs): Connect (with its preview) sets `statusLine.command` to `cay.exe
   statusline`, which reads stdin, sends only `rate_limits` (`five_hour` / `seven_day`: `used_percentage`,
   `resets_at`) to Cay over the hook pipe (bounded, quiet, never blocks), then runs the user's original command with
   the same stdin and prints its output unchanged (bounded wait; on any failure print nothing extra). The original
   `statusLine` object is saved in Cay's `settings.json` (`claude.statusLine`); Disconnect puts it back exactly. With
   no original, print nothing. Check the current docs (code.claude.com/docs/en/statusline: `rate_limits` appears only
   for Pro/Max/Ultra after the first response; each window may be absent; it's dropped after `resets_at`). Show the
   newest reading in the panel (percent used and when it resets), Rust-owned, with a stale marker after `resets_at`.
3. **An optional soft sound for "needs you"** (a session's needs-input, an approval, a question or a plan), off by
   default: `activities.sound` in Cay's `settings.json` until Phase 4b. Played by Rust (a short system sound or a small
   bundled WAV through `PlaySoundW` with `SND_ASYNC`), at most once per request, never while a fullscreen app runs.
4. **Spike: focus the Windows Terminal tab, not just the window** (known limitation). Find what's possible (UI
   Automation on Windows Terminal's tab list, matching the session's shell process or title; `wt.exe focus-tab` needs
   the index and acts on the most recent window). Report what's reliable; build it only if it is.

## Rules

- UI changes go through the `motion-designer` agent with a precise brief; review its renders and run `pnpm typecheck`.
- Never write the real `~/.claude/settings.json` in tests. Use temp folders, `CLAUDE_CONFIG_DIR`, hook JSON files written
  with the Write tool, and `claude -p … --settings <file>` with a recent Claude Code (2.1.314 under the VS Code
  extension).
- Don't edit Rust while the author drives `pnpm tauri dev` without saying so (it restarts Cay); check `Get-Process cay`
  before `agent-test`. No synthetic input while the author is at the machine.
- Commits: small, imperative, prefixed `phase2.8:`; push to `main`, no new branches. Ask before pushing.

## When the stage is done

cargo fmt / clippy / test and `pnpm typecheck` clean; PLAN.md (tick the items, IPC changes, Known limitations),
CASESTUDY.md (decisions from D41, challenges, timeline) and CLAUDE.md (layout, commands, gotchas) updated; a short manual
check list. When 2.8f is signed off, Phase 2.8 is complete: say so in PLAN.md's Current status, and the next step is
4a-5 (release v0.2, set aside for this phase). Update its kick-off prompt (`prompts/phase-4a-release-readiness.md`) for
what 2.8 changed: Connect must be rerun after updating, runs default to Auto, the README's permissions section, and a
full manual pass of 2.8a–f before the release.
