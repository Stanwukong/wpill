# Cay

**A Dynamic Island for Windows 11.** A small pill sits at the top centre of your screen. Hover the top edge and it
opens into a panel; when something happens (a song changes, the volume moves, a Claude Code session finishes or needs
you), it grows on its own for a moment and shrinks back. It never takes focus from the app you're in, and while
collapsed it doesn't block a single click.

<p>
  <img src="docs/images/pill-working.png" alt="The collapsed pill: an amber wave next to the clock while Claude works" height="48">
  &nbsp;
  <img src="docs/images/done.png" alt="A Done activity with an amber edge" height="56">
</p>
<p>
  <img src="docs/images/approval.png" alt="An approval card asking to allow a shell command, with Deny and Allow" height="136">
</p>

> **Status:** v0.2 is being prepared. Cay was called TopBar until November 2026. Built for Windows 11 only.

## What it does

- **Clock and running apps.** Hover the top edge: the panel lists your open windows with their icons; click one to
  switch to it.
- **Now playing.** Album art, title, progress and controls for whatever is playing (Spotify, browsers, anything that
  uses Windows' media controls). The island takes its colours from the album art.
- **Live activities.** Volume, brightness, Bluetooth headphones connecting (with battery), charger plugged in or
  low battery, Caps Lock, and new screenshots, which you can drag straight out of the island into a chat or folder.
- **Claude Code sessions** (optional). Cay shows when a session in your terminal or editor is working, when it's done
  and when it needs your input; click to jump to its window.
- **Claude agent** (optional). Press **Win+Alt+A**, type a task, pick a folder and walk away. Cay runs Claude Code in
  the background, shows progress in the island, and asks you to **Allow** or **Deny** each risky step right there.
  With your approval Claude can also look at windows and, one approved step at a time, click and type for you.
  **Win+Alt+Escape** stops every run. Tasks can be saved as routines that run on a schedule while Cay is running.

## Install

Download `Cay_<version>_x64-setup.exe` from [Releases](https://github.com/Stanwukong/wpill/releases) and run it. It installs for
your user only (no admin) into `%LOCALAPPDATA%\Cay`, and adds a tray icon with **Launch at startup** and **Quit Cay**.

v0.2 isn't code-signed yet (signing is planned from v0.3), so Windows SmartScreen will say "Windows protected your PC"
and list the publisher as unknown. Check your download first (below), then click **More info** → **Run anyway**.

**Check your download.** Each release lists the installer's SHA-256 checksum. In PowerShell:

```powershell
Get-FileHash .\Cay_0.2.0_x64-setup.exe -Algorithm SHA256
```

The hash must match the one on the release page exactly. If it doesn't, delete the file.

**Coming from TopBar?** Quit TopBar from its tray, then install Cay. On first start Cay copies TopBar's settings,
routines and agent history, and moves the startup entry if you had it on. If you'd connected Claude Code, click
**Connect Claude Code…** in Cay's tray once: it replaces TopBar's hooks. Then uninstall TopBar from Settings → Apps.

## What it needs

- Windows 11 (64-bit). The WebView2 runtime it uses is part of Windows 11.
- For the Claude features: [Claude Code](https://docs.claude.com/en/docs/claude-code) installed and signed in
  (tested with 2.1.123 to 2.1.314). Git Bash is used for the hooks when it's installed; PowerShell otherwise.
- Agent runs use your own Claude Code account and count against its usage and limits.

## What it touches, and privacy

Cay is local. **It makes no network requests of its own**: no telemetry, no update checks, no accounts. The only traffic
is Claude Code's own, to Anthropic, when *you* start an agent run (and a link you approve Claude opening in your
browser).

| Where | What | When |
|---|---|---|
| `%APPDATA%\io.github.stanwukong.cay\settings.json` | Activity switches and agent settings (edit by hand) | Read at start |
| `%APPDATA%\io.github.stanwukong.cay\routines.json` | Your saved routines | When you save one |
| `%APPDATA%\io.github.stanwukong.cay\agent\*.jsonl` | An audit trail of each agent run: what was asked, each tool call, your decisions. Typed text is reduced to its length; screenshots are never stored | During runs |
| `%APPDATA%\io.github.stanwukong.cay\agent\runs\` | Per-run config for Claude Code | Deleted when the run's process ends |
| `%LOCALAPPDATA%\io.github.stanwukong.cay\` | WebView2's cache | Always |
| `HKCU\…\CurrentVersion\Run`, value `Cay` | Launch at startup | Only when you tick it |
| `~/.claude/settings.json` | Seven hook entries that run `cay.exe hook` | **Only when you click Connect Claude Code…**, after a dialog that shows the exact entries. Backed up first; Disconnect removes only Cay's entries |

- **Hooks send almost nothing.** The hook forwarder passes five fields (the event, session id, folder, notification
  type and session source), a timestamp and its process chain to Cay, over a pipe only your user can open. Your prompts, Claude's answers and tool output
  never reach Cay.
- **Screenshots stay with Claude Code.** When you approve a screenshot in an agent run, Claude Code keeps the image in
  its own session transcript under `~/.claude/projects`, outside Cay's control. Cay never captures itself or a
  password field's contents.

## Agent runs never get more permission than you give

- Cay always starts Claude Code with `--permission-mode default` and routes every permission prompt to the island. It
  never uses `bypassPermissions` or `--dangerously-skip-permissions`, and never changes your Claude Code settings for
  a run. Your own allow rules in Claude Code still apply.
- Each look at the screen and each desktop action is approved on its own. Clicks and typing are never remembered "for
  this run". Any key you press or mouse move you make stops an action immediately, and the island stays out of the
  way while it acts.
- An unanswered approval is denied after 10 minutes. Routines can wait for you while you're away, but are never
  pre-approved.
- Everything a run starts lives in a Windows Job Object: Stop, Win+Alt+Escape or quitting Cay ends the whole process
  tree.

## Build from source

```powershell
.\scripts\check-env.ps1     # Rust (MSVC), VS Build Tools, Node, pnpm, WebView2
pnpm install
pnpm tauri dev              # run it
pnpm tauri build            # release build and NSIS installer
```

Stack: Tauri 2, Rust (`windows` crate), React + TypeScript + Vite, Framer Motion. [CLAUDE.md](CLAUDE.md) explains the
architecture and its Windows gotchas, [PLAN.md](PLAN.md) the roadmap, and [CASESTUDY.md](CASESTUDY.md) every decision
along the way.

## Not affiliated with Anthropic

Cay is an independent open-source project built on [Claude Code](https://docs.claude.com/en/docs/claude-code). It is not
made, endorsed or supported by Anthropic. Claude and Claude Code are trademarks of Anthropic, PBC.

## License

[MIT](LICENSE) © 2026 Oluwafemi S.E. Ajanaku
