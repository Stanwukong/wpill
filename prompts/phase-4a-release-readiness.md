Start Phase 4a (release readiness) of TopBar.

Read PLAN.md first: Current status, the Roadmap table, "Phase 4a: release readiness", Known limitations, Distribution
and Open questions. Then CASESTUDY.md: D2 (one window sized to the largest panel), D5 (the hot zone), D7 (geometry in
physical px), D8 (self-healing visibility), D10 and C5 (autostart registers the exe that toggled it), D26 (the glow
window per monitor) and §7 (results and metrics). CLAUDE.md is loaded; follow its rules and Windows gotchas, above
all rule 1 (Rust owns window state), the DPI / Text size gotcha and "Never use Tauri's `show()` on the island".

Phase 2.7 (local automation agent, stages 2.7a–e) is complete; 2.7e (routines) was signed off on 2026-10-31.

Goal: TopBar can be published as an open-source v0.2 that a stranger can install and trust: its own look (not Claude
Code's), correct on any monitor setup and DPI, a final name, a signed installer, and a repo that explains itself.

## Work in this phase (one stage at a time; each ends with its own commit and doc updates)

**4a-1 Own visual identity.** Follow [spinner-redesign.md](spinner-redesign.md) as written: candidates rendered by
the `motion-designer` agent, I pick, then it's applied. Nothing from Claude Code's visual set may remain.

**4a-2 Multi-monitor and per-monitor DPI.** Today geometry (monitor, scale) is read once at startup (Known
limitations), so docking, undocking, changing the primary monitor, resolution or scale breaks placement until a
restart. Read the code first (`window/mod.rs`, `hover/` hit tests, `fullscreen.rs`, `window/glow.rs`,
`window/command.rs`, `desktop/capture.rs` for the screen capture), then propose:
- how Rust learns of changes (`WM_DISPLAYCHANGE`, `WM_DPICHANGED`, `WM_SETTINGCHANGE` for the taskbar and work
  area; a message-only window on its own thread like `power.rs`, never the island's HWND) and re-reads geometry;
- what moves with it: the island's position and size (still top centre, D2's largest panel), the hot zone, the
  panel-rect hit tests, the fullscreen check, the glow window; and the reserved `monitor:changed` event (PLAN.md IPC)
  if React needs to know (Text size is folded into `devicePixelRatio`, D7: keep React → Rust geometry in physical px);
- which monitor the island lives on: primary by default; whether to offer "the monitor with the cursor" or a fixed
  choice now or leave the choice to Phase 4b settings (`settings.json` until then);
- how to test without a second physical monitor (changing scale and resolution in Settings, `ChangeDisplaySettingsEx`
  is too invasive: ask me before touching display settings), and what I need to check by hand with a real dock.

**4a-3 Name, identity and repo.** The final app name is an open question (PLAN.md): it sets the bundle identifier
(`dev.wpill.topbar`), the `%APPDATA%` folder, the single-instance key, the Run-key name, the agent pipe names and the
installer. Propose two or three names with checks (GitHub, winget, a trademark search you can do from the web),
then wait for my choice. Then: rename everywhere, with a one-time migration of `%APPDATA%\dev.wpill.topbar`
(settings, routines, agent audit) and the autostart entry; a README (what it is, a short GIF or screenshots,
install, what it needs, what it touches: hooks in `~/.claude/settings.json` only when you click Connect, local-only
audit files, no network calls of its own, the agent's approvals), a LICENSE (ask me which), a privacy note, and an
honest "built on Claude Code, not affiliated with Anthropic" line (see the branding notes in spinner-redesign.md).

**4a-4 Code signing.** Compare a code-signing certificate (OV/EV), Azure Trusted Signing (eligibility for an
individual in my country, cost) and leaving v0.2 unsigned with clear SmartScreen instructions. Recommend one, wait for
my OK, then wire it into `pnpm tauri build` (Tauri's `bundle.windows.signCommand` or `certificateThumbprint`) without
committing any secret. Check `topbar.exe` and the installer with `signtool verify /pa`.

**4a-5 Release v0.2.** Bump the version in `package.json`, `tauri.conf.json` and `Cargo.toml`; a release build (~11
min, fat LTO; check free space first, C: is nearly full: `cargo clean --profile dev` if needed); install it over the
dev setup; rerun the manual check lists that matter for a release (Phase 1, 1.5, 2.5 activities, 2.6, 2.7a–e smoke
items: pick a short combined list with me); measure size, RAM and CPU again for CASESTUDY §7; a GitHub release with
the installer and notes. Don't push tags or publish anything without asking.

## Rules for this phase

- Every UI change goes through the `motion-designer` agent; delegate it, then review what it returns.
- Propose before coding in 4a-2, 4a-3 and 4a-4, and wait for my OK.
- Don't change display settings, the registry outside TopBar's own Run key, or `~/.claude/settings.json` without
  asking. Don't send synthetic input while I'm using the machine.
- Agent runs never weaken permissions (CLAUDE.md); a rename must keep that true (approval tool name, pipe ACLs).
- Commits: small, imperative, prefixed `phase4a:`; push to `main`, no new branches.

## When each stage is done

cargo fmt / clippy / test and `pnpm typecheck` clean; PLAN.md (tick the item, IPC changes, Known limitations,
Distribution), CASESTUDY.md (decision entries, challenges, timeline) and CLAUDE.md (layout, commands, gotchas)
updated in the same change; a manual check list for anything only I can verify. When all of 4a is signed off, say
what's next on the roadmap (Phase 3 or 4b) and write its kick-off prompt.
