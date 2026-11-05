# Cay daily routine log

## 2026-11-04 | Phase 3: memory-heavy apps (research and design only)
- Status: BLOCKED on design approval. The kick-off prompt (prompts/phase-3-memory-heavy.md) requires research and the
  author's OK before any code, and Phase 3 had no task list, so this run did research and drafted the design and tasks
  in PLAN.md ("Phase 3 design: DRAFT – needs Femi's OK"). Nothing was executed. Branch `self/phase-3-memory-heavy`, draft PR https://github.com/Stanwukong/wpill/pull/2.
- Tasks: none run (3a.1–3b.2 drafted).
- Desktop build: the running Cay is `pnpm tauri dev` (target\debug\cay.exe built 11:36, after e42e752, the last
  code commit on main; 172a158 is docs only), so it matches main. No installed release exists; nothing rebuilt.
- Measured: NtQuerySystemInformation over 339 processes = 3.5 ms; Get-Process + exe paths ≈ 3 s.
- Next run: if Femi has approved (or edited) the draft, remove the DRAFT mark's blocking status per their notes and start
  task 3a.1 on this branch. If not, don't redo the research; wait.
- For Femi: pick the three **(choose)** items (grouping, confirm, agent tool) in PLAN.md's Phase 3 design. Suggested
  rule clarification: CLAUDE.md says "push to main, no new branches" while the routine says "branch, never push to
  main"; the routine followed its own prompt.

## 2026-11-04 (continued) | Phase 3: memory-heavy apps, stage 3a
- Status: PARTIAL. Femi approved the design (all recommendations). Stage 3a is built and its checks pass; the 3a manual
  check is Femi's. Draft PR https://github.com/Stanwukong/wpill/pull/2.
- Tasks: ✅ 3a.1 · ✅ 3a.2 · ✅ 3a.3 · ✅ 3a.4 (motion-designer) in one commit (see git log) · ⏳ 3a manual check
  (Femi) · 3b.1–3b.2 not started (they wait for 3a's sign-off).
- Checks: cargo fmt --check, clippy --lib --tests -D warnings, cargo test --lib (282 passed, 1 ignored), pnpm typecheck,
  pnpm build: all clean.
- How: worked directly with the shell and the motion-designer agent rather than driving the VS Code extension, because
  Femi was at the machine (no focus stealing). Rust was edited in a git worktree (`..\wpill-phase3`), because the live
  `pnpm tauri dev` in the main checkout restarts Cay on Rust edits and this run lives inside that Cay.
- Next run: if Femi signed off 3a, start 3b.1 (`processes/end.rs`) on this branch in the worktree. If 3a's check found
  problems, fix those first.
- For Femi: run the 3a manual check in PLAN.md (check out the branch, set `"processes": { "enabled": true }`). The
  worktree `..\wpill-phase3` can be removed with `git worktree remove ..\wpill-phase3` once you're done with it.

## 2026-11-05 | Phase 2.10: agent window UI (research and design only)
- Status: BLOCKED on design approval. Phase 3 is merged (PR #2); 2.10 is next and its design was "to be agreed", so
  this run researched and drafted it in PLAN.md ("Phase 2.10 design: DRAFT – needs Femi's OK"). No code was run or
  written. Branch `self/phase-2.10-agent-window`, draft PR https://github.com/Stanwukong/wpill/pull/3.
- Tasks: none run (2.10a.1–2.10d proposed).
- Desktop build: the running Cay is `pnpm tauri dev`; `target\debug\cay.exe` was built 2026-11-05 12:06, after
  b894f09 (the last code commit on main; later commits are docs only), so it matches main. No installed release; nothing rebuilt.
- Checks on main: `cargo test --lib` 290 passed (2 ignored), `pnpm typecheck` clean.
- Next run: if Femi has answered the six (choose) items, drop the DRAFT mark, put the proposed tasks in place of the
  stage bullets and start 2.10a.1 (Rust, in a worktree beside the live dev Cay). If not, wait; don't redo the research.
- For Femi: pick choices 1–6 in PLAN.md's Phase 2.10 design. Choice 4 adds `--add-dir` to runs, which widens what a
  run can read; it needs your explicit OK, since it sits next to the protected permission rules.

## 2026-11-05 (continued) | Phase 2.10: agent window UI, stage 2.10a
- Status: PARTIAL. Femi approved the design ("proceed": every recommendation, including `--add-dir` for the run's
  attachments folder, D52). Stage 2.10a is built and its checks pass; the 2.10a manual check is Femi's. Draft PR
  https://github.com/Stanwukong/wpill/pull/3.
- Tasks: ✅ 2.10a.1 0a6b91a · ✅ 2.10a.2 23c920a · ✅ 2.10a.3 e622e23 · ✅ 2.10a.4 e6623dd · ✅ 2.10a.5 797dd6e+e6623dd ·
  ⏳ 2.10a manual check (Femi) · 2.10b–d not started (they wait for 2.10a's sign-off).
- Checks: cargo fmt --check, clippy --lib --tests -D warnings, cargo test --lib (296 passed, 2 ignored), pnpm typecheck,
  pnpm test (13, new: `node --test`), pnpm build: all clean. Claude Code 2.1.123's `--include-partial-messages` output
  checked by hand against the parser (text_delta picked up, thinking deltas ignored).
- How: shell + the motion-designer agent for every UI change (Femi was at the machine), not the VS Code extension. Rust
  in the worktree `..\wpill-phase210`, building into the main checkout's `target` (`CARGO_TARGET_DIR`) to reuse its
  compiled deps; the main checkout stays on `main`, so the desktop dev Cay still matches main.
- Next run: if Femi signed off 2.10a, start 2.10b.1 (attachments, Rust) in `..\wpill-phase210`. If the check found
  problems, fix those first.
- For Femi: run the 2.10a manual check in PLAN.md (stop the dev Cay on main, then `pnpm tauri dev` in
  `..\wpill-phase210`, or check out the branch). Gotcha seen twice today: Python heredocs in Git Bash turn `\a` and
  `\n` inside Rust/TS string literals into control characters; write such code with the Write/Edit tools.

## 2026-11-05 (afternoon) | Phase 2.10: agent window UI, stage 2.10b
- Status: PARTIAL (2.10a and 2.10b built; their manual checks are Femi's; 2.10c–d not started). Draft PR
  https://github.com/Stanwukong/wpill/pull/3.
- Found: 2.10b.1 committed locally (3795ff3, unpushed) and 2.10b.2 half-done, uncommitted, in `..\wpill-phase210`: the
  author's VS Code session had hit its 5-hour limit at 13:18. Femi chose "Routine finishes 2.10b" (asked in Cay).
- Tasks: ✅ 2.10b.1 3795ff3 · ✅ 2.10b.2 0da7c8d (wired ACP start/follow-up, `acceptsImages`) · ✅ 2.10b.3 fc7a749 ·
  ✅ 2.10b.4 09dc6e8 (motion-designer; file picker command added) · ⏳ 2.10b manual check (Femi).
- Checks: cargo fmt --check, clippy --lib --tests -D warnings, cargo test --lib (317 passed, 2 ignored), pnpm
  typecheck, pnpm test (16), pnpm build: all clean. Headless Edge renders of the chips/overlay (C:	mpttach-render).
  Not run live: the live dev Cay hosts this run, so the worktree build wasn't launched; the clipboard was never
  written (it's Femi's).
- Desktop build: the dev Cay from the main checkout (on main, 53fc680) was rebuilt 14:11 today; main's last code
  commit is b894f09, so it matches main. Nothing rebuilt.
- How: shell + motion-designer, not the VS Code extension (Femi was at the machine and their VS Code session holds
  this branch's history).
- Next run: if Femi signed off 2.10a/2.10b, start 2.10c.1 (maximize button, F11, double-click) in `..\wpill-phase210`.
  If the checks found problems, fix those first.
- For Femi: run the 2.10a and 2.10b manual checks in PLAN.md. Live unknowns: whether WebView2 reports "Files" in
  `clipboardData.types` for a Win+Shift+S paste, and whether OpenCode/Codex read a `resource_link` outside the
  project folder without asking.

## 2026-11-05 (evening) | Phase 2.10: agent window UI, stage 2.10c
- Status: PARTIAL (2.10a–b merged in PR #3; 2.10c built, its manual check is Femi's; 2.10d not started, it waits for
  the 2.10c check as earlier stages did). PR: see below.
- Found: PR #3 merged at 15:36 with no review comments; nothing uncommitted. The branch was fast-forwarded to main
  (101b1cc) in `..\wpill-phase210` and reused, as "resume on its existing branch".
- Tasks: ✅ 2.10c.1 e3b5ab0 (button, F11, Esc restores first, `command:maximized`; the title-bar double-click is
  Tauri's default `allow-internal-toggle-maximize`, no capability change) · ✅ 2.10c.2 4be4996 (`agent.window`, pure
  `fit` with 4 tests; `agent::settings` made `pub mod`) · ✅ 2.10c.3 a873d4e (CSS only, motion-designer; pixel-identical
  at 720 and 540 px in headless Edge, C:\tmp\cmdwide) · ⏳ 2.10c manual check (Femi).
- Checks: cargo fmt --check, clippy --lib --tests -D warnings, cargo test --lib (322 passed, 2 ignored), pnpm
  typecheck, pnpm test (16), pnpm build: all clean. Not run live: the dev Cay on main hosts this run, so the branch build
  wasn't launched (single instance; editing it would end the run).
- Desktop build: the dev Cay's `target\debug\cay.exe` was rebuilt 15:47, after main's merge commit 101b1cc (15:36),
  so it matches main. Nothing rebuilt.
- How: shell + motion-designer (no VS Code extension driving; Femi may be at the machine).
- Next run: if Femi signed off 2.10c, start 2.10d.1 (composer: grows, Enter/Shift+Enter, agent/model/mode, per-run
  draft) on this branch in `..\wpill-phase210`. If the check found problems, fix those first.
- For Femi: run the 2.10c manual check in PLAN.md. Live unknown: whether Tao's maximize of the undecorated window stays
  inside the work area here (the plan's open question for choice 5).
