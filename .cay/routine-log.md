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
