Redesign TopBar's "working" spinner and Claude mark so the app has its own visual identity, in the colour palette of
my portfolio site (the `femiajanaku` repo, next to this one).

Read CLAUDE.md first (loaded), then `.claude/agents/motion-designer.md`, CASESTUDY.md D18, D19 and D24, and
PLAN.md "Current status". **All UI work goes through the `motion-designer` agent**: delegate it, then review what it
returns. Don't change Rust, IPC names or payloads; this is a visual change only.

## Why

TopBar is going public as an open-source project, for exposure. Today its Claude UI copies Claude Code's own look:
the spinner steps through Claude Code's glyphs (`· ✢ * ✶ ✻ ✽`, forwards then back, 120 ms a frame), the activities
and command window use the `✻` mark, and everything Claude-related is Claude Code's orange (`--claude: #d77757`),
including the island's edge glow. Anthropic's branding guidance for products built on Claude Code says the product
"should maintain its own branding and not appear to be Claude Code or any Anthropic product", and lists "Claude
Code-branded ASCII art or visual elements that mimic Claude Code" as not permitted. Referring to Claude by name is
fine ("Claude Agent", "Powered by Claude", a "Claude Code" heading for the list of Claude Code sessions); looking
like Claude Code is not.

## The palette (from `femiajanaku/src/app/globals.css` and `tailwind.config.ts`)

The site is warm amber on near-black. The island is always dark, so use the site's dark-mode values:

| Token (site) | HSL | Hex | Use in TopBar |
|---|---|---|---|
| `--accent` (dark) | `38 92% 55%` | `#f6a823` | The new agent/Claude accent: spinner, mark, edge glow, filled Allow button |
| `--accent` (light) | `38 92% 50%` | `#f59e0b` | A deeper amber, e.g. for gradients or the pressed state |
| `--background` (dark) | `0 0% 6%` | `#0f0f0f` | Ink on amber (text on the filled Allow button: white on amber fails contrast) |
| `--card` / `--muted` (dark) | `0 0% 10%` / `12%` | `#1a1a1a` / `#1f1f1f` | Command window surfaces, if they need adjusting |
| `--border` (dark) | `0 0% 20%` | `#333333` | Hairlines |
| `--muted-foreground` (dark) | `0 0% 60%` | `#999999` | Quiet text |
| `--foreground` (dark) | `0 0% 93%` | `#ededed` | Primary text |

Its motion vocabulary: `--ease-default: cubic-bezier(0.33, 1, 0.68, 1)`, `--ease-dramatic: cubic-bezier(0.65, 0, 0.35,
1)`, durations 200 / 400 / 800 ms, and two loops: `pulse-amber` (opacity 1 → 0.5 → 1 over 2 s, ease-in-out) and a slow
`rotate` (10 s linear). Borrow the feel; keep TopBar's own spring layout motion (`src/island/motion.ts`) as it is.

Keep the amber a **fixed token**, like `--charge` (green, charging) and today's `--claude`. It must not follow the
album-art `--accent`. Check it still reads on a tinted island, including warm, amber-ish album art, and that it
doesn't get confused with the low-battery alert colour or the charging green.

## What to design

1. **A new "working" indicator** to replace `ClaudeSpinner`. It shows left of the clock in the pill while any Claude
   session or agent run works (~10–12 px tall next to the clock), on working rows in the panel, and wherever the
   command window shows a live run. It must be original: no asterisk or starburst glyphs, no glyph-cycling animation
   like Claude Code's, nothing that reads as the Claude/Anthropic mark. Ideas worth rendering, not requirements: an
   amber dot or short arc orbiting inside a tiny pill (echoing the island's own shape); a pill that breathes with
   `pulse-amber`; a slowly rotating arc in the site's `rotate` rhythm.
2. **A new static mark** to replace `ClaudeMark` (`✻`): the icon of the Claude activities ("Done", "Needs your
   input", `agentFinished`), the approval card's header, the panel's agent rows and the command window title. It should
   pair with the indicator (e.g. the indicator at rest).
3. **The edge glow** (`data-claude="done"` steady, `"needsInput"` breathing, also used by approvals) moves from
   orange to the amber, with the breathing on the `pulse-amber` timing or close to it.
4. **Replace every remaining `--claude` orange** (dots, Allow button, status colours, command window accents) with
   the new token. Rename `--claude` to something neutral (e.g. `--agent`) so the name stops pointing at Claude Code's
   look; rename `ClaudeSpinner` / `ClaudeMark` to match. Keep the `--font-claude` typeface (Cascadia Mono) unchanged.
5. Strings stay as they are unless they imply TopBar *is* Claude Code. "Claude agent" (window title) and the "Claude
   Code" section heading are fine.

Where the old look lives today (`--claude`, `ClaudeSpinner`, `ClaudeMark`, `✻`, `data-claude`):
`src/styles/global.css` (the token), `src/features/claude/ClaudeGlyphs.tsx` + `claude.css` (spinner, mark),
`ClaudeSection.tsx`, `src/features/activities/ClaudeActivity.tsx`, `AgentApproval.tsx`, `activities.css` (edge glow,
dots), `src/features/agent/AgentRunItems.tsx` + `agent.css`, `src/island/Island.tsx`, and `src/command/CommandApp.tsx` +
`command.css`. Search again before finishing; nothing from Claude Code's visual set may remain.

## Rules (from CLAUDE.md and the motion-designer agent)

- Ambient loops are CSS keyframes on transforms, opacity or `@property` values: no JS timers (WebView2 throttles
  them), and the indicator is *absent*, not paused, when nothing is working. Reduced motion gets a static mark.
- React renders what Rust's events say; nothing here changes when things show or hide.
- Keep every activity's measured size and hit area as they are now (the island reports its own size).
- Check contrast: text on the amber button needs dark ink (`#0f0f0f`), and amber text on the black island must stay
  readable at 11 px.

## Steps

1. **Candidates first.** Have the motion-designer agent render two or three indicator + mark pairs side by side in
   headless Edge (as it did for the fonts): pill with the clock, a "Done" and a "Needs your input" activity with the
   edge glow, an approval card, two panel rows, the command window header. Show them on the plain island and on a
   tinted one (include a warm, amber-ish tint), at dpr 1.2 and one row at 1.5. Animated candidates also need a
   short frame strip or GIF so the motion can be judged. **Stop and let me pick** before anything is applied.
2. Apply the chosen design everywhere listed above, through the motion-designer agent. `pnpm typecheck` clean.
3. Verify in the real app (`pnpm tauri dev`): capture the pill while an agent run works, a "Done" and an approval
   activity, and the command window. Use `target\debug\topbar.exe agent-test …` with `haiku` and a scratch folder
   (CLAUDE.md, "Testing agent runs"). Don't send synthetic keystrokes or take screen captures while I'm using the
   machine; if the foreground isn't TopBar, stop and ask.
4. Docs, in the same change: a CASESTUDY decision entry (why the look changed, the palette's source, the candidates
   and why the chosen one won); PLAN.md (mark the redesign done under Phase 4a and update the 2.6 and 2.7 descriptions
   that mention the spinner, orange or `✻`); CLAUDE.md (the renamed token and components in "Layout");
   `.claude/agents/motion-designer.md` (the new token, indicator and mark, so future UI work uses them).
5. Commit (`polish: own working indicator and amber accent`) and push to `main`.
