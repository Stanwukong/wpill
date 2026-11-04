# Cay daily routine log

## 2026-11-04 | Phase 3: memory-heavy apps (research and design only)
- Status: BLOCKED on design approval. The kick-off prompt (prompts/phase-3-memory-heavy.md) requires research and the
  author's OK before any code, and Phase 3 had no task list, so this run did research and drafted the design and tasks
  in PLAN.md ("Phase 3 design: DRAFT – needs Femi's OK"). Nothing was executed. Branch `self/phase-3-memory-heavy`.
- Tasks: none run (3a.1–3b.2 drafted).
- Desktop build: the running Cay is `pnpm tauri dev` (target\debug\cay.exe built 11:36, after e42e752, the last
  code commit on main; 172a158 is docs only), so it matches main. No installed release exists; nothing rebuilt.
- Measured: NtQuerySystemInformation over 339 processes = 3.5 ms; Get-Process + exe paths ≈ 3 s.
- Next run: if Femi has approved (or edited) the draft, remove the DRAFT mark's blocking status per their notes and start
  task 3a.1 on this branch. If not, don't redo the research; wait.
- For Femi: pick the three **(choose)** items (grouping, confirm, agent tool) in PLAN.md's Phase 3 design. Suggested
  rule clarification: CLAUDE.md says "push to main, no new branches" while the routine says "branch, never push to
  main"; the routine followed its own prompt.
