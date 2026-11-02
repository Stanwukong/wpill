Start Phase 6 of Cay: Google Calendar and Gmail (proposed in D35). Research and a design proposal come first; no code
until the author has agreed to the design.

Read PLAN.md first: Current status, the Roadmap, "Phase 6: Google Calendar and Gmail" (its plan, stages 6a–6c and
open questions), the IPC section (naming and payload conventions), Known limitations and Open questions. Then
CASESTUDY.md: D2 (one window sized to the largest panel), D14/D19 (the activity queue, sticky notices), D21–D25 (agent
runs, approvals, Cay's MCP server and its desktop tools), D30 (Cay's own look), D35 (why Cay signs in to Google itself)
and D40 (runs use Claude Code's permission modes, Auto by default; Cay's own tools follow the mode in Auto). CLAUDE.md
is loaded; follow its rules and Windows gotchas, above all rule 1 (Rust owns state; no JS timers), rule 2 (the island
is never activated: sign-in happens in the system browser, any typing in a separate window), rule 4 (Win32 isolated,
`// SAFETY:` per `unsafe`) and rule 7 (a feature folder on both sides).

Where things stand (2026-11-02): Phase 2.8 (agent UX) is complete. Release readiness 4a-5 (v0.2) is deferred to the end
of all phases (the author's call), so phases follow the roadmap without a release in between. Cay has no network code
and no secrets today; this phase adds both, so treat them with care.

Goal: upcoming events and today's tasks in the island (the Google Calendar icon in the pill while something is
pending, a "starting soon" activity with Join, a Calendar section in the panel), and later Gmail and Calendar tools for
agent runs, with data going only from Google to this PC.

## Research first (report findings, with sources, before proposing)

- Google's installed-app OAuth today: PKCE, loopback redirect on `127.0.0.1`, the system browser, refresh tokens, token
  revocation; what a desktop client's "client secret" means (it isn't secret) and how other open-source desktop apps ship
  theirs. Fetch the current docs (developers.google.com/identity/protocols/oauth2/native-app); don't guess.
- Verification: what Calendar (`calendar.events.readonly`, `calendar.calendarlist.readonly`) and Tasks
  (`tasks.readonly`) scopes need (sensitive: privacy policy, homepage, demo video), how long it takes, the 100-user cap
  and 7-day refresh tokens while unverified or in Testing; whether Gmail's restricted scopes need the CASA assessment
  for a local-only app with no server (D35 says confirm with Google before 6c).
- Calendar API incremental sync (sync tokens, `410 Gone`), events in the next 24 h across selected calendars, all-day
  and declined events, Meet / conference links; Tasks API (due today and overdue; tasks have dates, not times).
- The iCal "secret address" fallback (read-only, no tasks, no sign-in) as a way to ship 6a–b before verification.
- Rust crates: an HTTP client that's small and maintained (and its TLS: Windows' own schannel vs rustls), JSON, and
  storing tokens in Windows Credential Manager (`CredWriteW` / `CredReadW`, DPAPI) with the `windows` crate already in
  use. Mind binary size (the installer is ~1.6 MB) and the release profile.
- Google's API brand guidance for showing the Calendar icon unaltered in the pill.

## Then propose (and wait for the author's OK)

1. Which path ships first: Cay's own OAuth client (register it, go through verification) or the iCal fallback for 6a–b,
   with sign-in later. Recommend one, with the costs.
2. The `google/` module design (`auth.rs`, `calendar.rs`, `tasks.rs`, pure `logic.rs`), the `google-watcher` thread
   through the `activity::Watcher` harness (refresh every 5 min and on wake, network change and unlock; back off
   offline), and the IPC names (`google:state`, `calendar:upcoming`, commands) per the conventions.
3. Where Connect / Disconnect Google lives before Phase 4b (the tray, like Claude Code's Connect, with a preview of
   what's granted), and what's stored where (tokens in Credential Manager only, never `settings.json`; a small cache).
4. The pill's priority when the calendar icon, media art, the agent wave and the step line all want it (open question).
5. For 6c: the MCP tools (`calendar_list_events`, `tasks_list`, `gmail_search`, `gmail_read`, `gmail_draft`), how they're
   approved under D40's modes (reads in Auto?), never `gmail.send`, mail treated as untrusted input (prompt injection),
   and the `agent.claudeConnectors` switch for users who already connected claude.ai's Gmail and Calendar connectors.

## Rules

- Stage by stage (6a watcher, 6b island, 6c agent tools), each ending with its own commit, doc updates and a manual
  check list. Don't start a stage before the previous one is signed off.
- UI changes go through the `motion-designer` agent with a precise brief; review its renders and run `pnpm typecheck`.
- Never commit a client secret, token or the author's calendar data; tests use recorded fixtures, not live accounts.
  Ask before any step that signs in, registers anything with Google, or sends a request from the author's account.
- New dependencies are a decision: record each in CASESTUDY with why and the size cost.
- Don't edit Rust while the author drives `pnpm tauri dev` without saying so (it restarts Cay).
- Commits: small, imperative, prefixed `phase6:`; push to `main`, no new branches. Ask before pushing.

## When each stage is done

cargo fmt / clippy / test and `pnpm typecheck` clean; PLAN.md (tick items, IPC changes, Known limitations, Current
status), CASESTUDY.md (decisions from D42, challenges, timeline) and CLAUDE.md (layout, commands, gotchas) updated in the
same change. When Phase 6 is signed off, write the next phase's kick-off prompt (2.9 other agents and open models, per
the roadmap) so a new session can pick up without this conversation.
