Start Phase 3 of Cay: memory-heavy apps. Research and a design proposal come first; no code until the author has
agreed to the design.

Read PLAN.md first: Current status, the Roadmap table, "Phase 3: memory-heavy apps (behind a settings toggle)", the
IPC section (above all "Reserved for later phases": `list_heavy_processes` and `end_process_group` were named in Phase
0), Known limitations and Open questions. Then CASESTUDY.md: D1–D3 (Rust owns hover state, one window, never
activated), D4 (IPC names fixed up front), D14–D15 (activities and one watcher thread per source), D18 (UI work goes
through the motion-designer agent), D30 (Cay's own look), D44 (one scroller for the panel's lists) and D27 (how Cay
already treats risky actions: one approval each, never on a guess, your input comes first). CLAUDE.md is loaded; follow
its rules and Windows gotchas, above all rule 1 (Rust owns state), rule 2 (the island never takes focus: a confirm needs
a native dialog or the command window), rule 3 (IPC names in one place), rule 4 (Win32 isolated, `// SAFETY:` per
`unsafe`), rule 6 (revalidate before acting) and rule 7 (feature folders).

Where things stand (2026-11-04): Phase 2.9 (other agents and open models: Codex and OpenCode sessions in the island,
runs on OpenCode and Codex through ACP, the agent and model picker) is complete; Goose with Ollama waits for the
author's other machine. Release 4a-5 (v0.2) stays deferred to the end of all phases. The settings window is Phase 4b,
so a Phase 3 switch lives in `settings.json` until then (like `activities.*` and `agent.*`).

Goal: a panel section, off by default, listing the apps using the most memory, grouped by app, with an End Task that
asks first, refuses protected processes in Rust, and explains elevated ones. It must cost nothing while the panel is
closed.

## Research first (report findings, with sources, before proposing)

- Grouping: by exe name, by the app a window belongs to (`apps/` already maps windows to exes and icons), or by
  process tree (browsers and Electron apps run many children). Which matches what a user calls "an app" for Chrome,
  Edge, VS Code, Teams, WhatsApp (UWP) and `svchost`? What does Task Manager's "Apps" grouping do?
- Memory: which number (working set, private bytes, commit) Task Manager's "Memory" column shows, and what `sysinfo`
  reports on Windows for each; whether `sysinfo` is already a dependency and its refresh cost for ~300 processes; or
  `NtQuerySystemInformation` / `K32GetProcessMemoryInfo` directly.
- Refresh: ~2 s while the panel is open and nothing while it's closed (the poller already knows); how long one refresh
  takes on the dev PC (8 GB RAM), measured, not guessed.
- Ending a group: `TerminateProcess` per pid vs `WM_CLOSE` to its windows first (unsaved work), the order for a tree,
  pids reused between list and click (creation time, as `claude/process.rs` checks), and what happens to UWP apps.
  The protected list in PLAN.md (csrss, wininit, winlogon, services, lsass, smss, svchost, dwm, explorer, System,
  Registry, Cay's own pid) and whether anything is missing (Defender's `MsMpEng`, `audiodg`, `fontdrvhost`, …).
- Elevated targets: what `OpenProcess(PROCESS_TERMINATE)` returns, and the message the user sees.
- The confirm: the island can't take keyboard focus (rule 2), so a native message box (as Connect uses) or the command
  window; which reads better for "End WhatsApp (3 processes, 1.2 GB)?".
- Whether agent runs should get a read-only `list_heavy_processes` tool in Cay's MCP server (and never an end-process
  tool), or nothing.

## Then propose (and wait for the author's OK)

1. The `processes/` feature folder (Rust and `src/features/processes/`), the IPC (keep or revise the reserved names and
   payloads, in `ipc.rs` / `ipc.ts` / `types.ts`), and the switch in `settings.json`.
2. How the list refreshes only while the panel is open, and its cost.
3. End Task: the confirm, the order (close windows, then terminate), revalidation, the protected list enforced in Rust,
   the elevated message, and what's audited or logged.
4. The panel section's look (through `motion-designer`): where it sits among the panel's sections, rows, icons, the
   memory figure, the End button, the empty and error states; minimal and native, no new colours.
5. Tests: pure grouping, sorting, protection and revalidation logic unit-tested; manual checks with a throwaway
   process (e.g. a second Notepad), never by ending the user's real apps without asking.

## Rules

- Stages if the work is big enough (e.g. 3a list, 3b End Task), each ending with its own commit, doc updates and a
  manual check list. Don't start a stage before the previous one is signed off.
- Never end a process in tests except ones the test started itself; ask before anything that closes the author's apps.
- UI changes go through the `motion-designer` agent with a precise brief; review its renders and run `pnpm typecheck`.
- New dependencies are a decision: record each in CASESTUDY with why and the size cost.
- Don't edit Rust while the author drives `pnpm tauri dev` without saying so (it restarts Cay). Watch the disk:
  `target\debug` has filled the drive several times (C14; see CLAUDE.md's Commands); about 12 GB was free on
  2026-11-04.
- Commits: small, imperative, prefixed `phase3:`; push to `main`, no new branches. Ask before pushing.

## When each stage is done

cargo fmt / clippy / test and `pnpm typecheck` clean; PLAN.md (tick items, IPC changes, Known limitations, Current
status), CASESTUDY.md (decisions from D49, challenges, timeline) and CLAUDE.md (layout, commands, gotchas) updated in
the same change. When Phase 3 is signed off, write the next phase's kick-off prompt (5, utilities, per the roadmap) so
a new session can pick up without this conversation.
