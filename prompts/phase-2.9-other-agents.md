Start Phase 2.9 of Cay: other agents and open models. Research and a design proposal come first; no code until the
author has agreed to the design.

Read PLAN.md first: Current status, the Roadmap, "Phase 2.9: other agents and open models" (its principles, stages
2.9a–c and open questions), the IPC section (naming and payload conventions), Known limitations and Open questions.
Then CASESTUDY.md: D17 (the Claude Code activity through hooks), D21–D23 (agent runs: headless Claude Code in a Job
Object, the token-checked pipe and approvals), D25 and D27 (desktop looks and actions in Cay's MCP server), D30 (Cay's
own look: no vendor spinners, logos or colours), D34 (the competitor survey that proposed this phase), D36 (terminal
sessions' permission requests, never decided without a click), D40 (Claude Code's permission modes, never bypass) and
D46 (Google tools in `cay.exe mcp`, mail as untrusted input). CLAUDE.md is loaded; follow its rules and Windows
gotchas, above all rule 1 (Rust owns state), rule 3 (IPC names in one place), rule 7 (feature folders), the hook
forwarder's rules (quiet, fast, fails open, never answers a request nobody clicked) and the agent-run rules (suspended
start + Job Object, per-run files only, never write the user's own settings for a run).

Where things stand (2026-11-03): Phase 6 (Google Calendar, Tasks and Gmail, in the island and as agent tools) is
complete. Release 4a-5 (v0.2) stays deferred to the end of all phases (the author's call), so 2.9 comes next even
though PLAN.md once said "not scheduled until 4a-5 ships". Cay today talks only to Claude Code: terminal sessions
through hooks (`claude/`) and runs it starts itself (`agent/`, stream-json, `--permission-prompt-tool
mcp__cay__approve`).

Goal: sessions of other coding agents (Codex CLI first, OpenCode next) shown and approved in the island like Claude
Code's, and agent runs through a second backend (ACP; Goose with a local Ollama model first), without weakening any
permission and without touching the native Claude Code path.

## Research first (report findings, with sources, before proposing)

- Codex CLI today: its hooks (`~/.codex/hooks.json` or `config.toml`?), which events exist (SessionStart/End,
  PreToolUse, PermissionRequest since which version, Stop), their input and the output that allows or denies, timeouts,
  and how it behaves when a hook fails or is slow. Fetch the current docs and changelog; don't guess. Also `codex exec
  --json` and its approval modes.
- OpenCode: `opencode serve` (`GET /event` SSE, the permission endpoints), plugins vs hooks, whether attaching to a
  running TUI's server is possible, and auth on that server.
- Agent Client Protocol: the current spec (initialize, `session/new` and its MCP-servers field, `session/prompt`,
  `session/update`, `session/request_permission`, `session/cancel`, client fs/terminal capabilities), its version and
  stability, and which agents implement it natively or through an adapter (Goose, OpenCode, Codex, Gemini CLI, Claude
  Code's own adapter).
- Goose: running it under ACP with an Ollama model, its permission modes, and the exact setting that makes every
  write and shell command ask. The same for each other candidate backend; a backend without one isn't added.
- Which local models handle tool calls well enough on a typical PC (and the RAM they need), so the README can say
  honestly what to expect.
- How the rivals named in PLAN.md (Nook, Vibe Island, Open Island, Agent Notch, …) mark which agent a session belongs
  to, to keep Cay's own look (D30) without copying theirs.

## Then propose (and wait for the author's OK)

1. 2.9a: the `agent` field in `claude::protocol` / `sessions` (or a rename of `claude/` to something agent-neutral, and
   at what cost), `cay.exe hook --agent codex`, Connect/Disconnect per agent in the tray (preview, dated backup, only
   Cay's entries removed), and how a Codex permission request maps to the island's card and back.
2. The per-agent mark in the pill, activity and panel (no vendor logos or colours; through `motion-designer`), and how
   to name the "Claude" UI (`ClaudeSection`, `ClaudeActivity`, `--font-claude`) once it isn't only Claude.
3. 2.9b: the `agent::backend` trait over what `run.rs` / `stream.rs` / `lifecycle.rs` do today, with `ClaudeCode`
   unchanged; the pure `agent::acp` module; how Cay's MCP server (desktop and Google tools) reaches an ACP agent; how
   2.8e's modes map per backend (or Ask only); desktop tools off for non-Claude backends until a scripted
   look-and-act test passes.
4. 2.9c: the `agent.backends` block in settings.json, discovery like `installs.rs`, the command window's agent and model
   picker (per task and per routine; the default stays Claude Code), and what the runs list and audit record.
5. Answers to PLAN.md's open questions (Gemini CLI, one ACP backend vs native paths, UI naming), and the order of
   stages.

## Rules

- Stage by stage (2.9a sessions, 2.9b ACP runs, 2.9c settings and docs), each ending with its own commit, doc updates
  and a manual check list. Don't start a stage before the previous one is signed off.
- Every backend gets a test that fails if the agent could auto-approve a write or a shell command.
- Never write the user's real `~/.codex`, `~/.config/opencode` or `~/.claude` settings in tests: use temp folders and
  the agents' config-dir overrides, as `hooks.rs` does with `CLAUDE_CONFIG_DIR`. Ask before Connect touches the real
  ones.
- UI changes go through the `motion-designer` agent with a precise brief; review its renders and run `pnpm typecheck`.
- New dependencies are a decision: record each in CASESTUDY with why and the size cost.
- Don't edit Rust while the author drives `pnpm tauri dev` without saying so (it restarts Cay). Watch the disk:
  `target\debug` has filled the drive several times (C14; see CLAUDE.md's Commands).
- Commits: small, imperative, prefixed `phase2.9:`; push to `main`, no new branches. Ask before pushing.

## When each stage is done

cargo fmt / clippy / test and `pnpm typecheck` clean; PLAN.md (tick items, IPC changes, Known limitations, Current
status), CASESTUDY.md (decisions from D47, challenges, timeline) and CLAUDE.md (layout, commands, gotchas) updated in
the same change. When Phase 2.9 is signed off, write the next phase's kick-off prompt (3 memory-heavy apps, per the
roadmap) so a new session can pick up without this conversation.
