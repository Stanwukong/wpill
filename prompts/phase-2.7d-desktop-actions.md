Start stage 2.7d (desktop actions) of TopBar's Phase 2.7.

Read PLAN.md first: Current status, "Phase 2.7: local automation agent" (principles, what 2.7a–c built, stage 2.7d and
the stages after it), the Phase 2.7 (a–b) and 2.7c IPC tables and Known limitations. Then read the CASESTUDY.md
decision log, especially D21 (desktop tools in TopBar's own MCP server), D23 (risk classes, "allow for this run"),
D25 (desktop looks: target checks, the one-time grant, TopBar asking itself when Claude Code didn't) and D26 (the
glow). CLAUDE.md is loaded; follow its rules and Windows gotchas, above all rule 2 (the island never takes focus or
keyboard input), rule 6 (revalidate every HWND), "Agent runs never weaken permissions" and "Desktop looks never
capture TopBar". Phase 2.7a–c are signed off (2.7c on 2026-10-30, all 12 manual checks pass).

Goal: Claude, in a run TopBar started, can *act* on the desktop: bring a window forward, press a button or set a
field through UI Automation, and, only when that's not possible, click, type and press keys with synthetic input, or
launch an app or URL. Every action is approved from the island, one at a time, and never remembered for the run. I
can always see that Claude is acting (not just looking), and anything I do with the mouse or keyboard stops it at
once.

## What 2.7a–c give you (read the code before designing)

- `src-tauri/src/desktop/`: `windows.rs` (`list`, `focused`, `check(hwnd, expect_pid)`: gone, TopBar's own, cloaked,
  hung, elevated, screen locked), `capture.rs`, `encode.rs`, `uia.rs` (control-view walk with a cache request, UIA
  timeouts, a time budget; per-look MTA thread from `mod.rs`), and the pure `logic.rs` / `tree.rs`. Coordinates are
  physical screen px; a screenshot reply carries `origin` and `scale` (screen px per image px).
- `agent/tools.rs`: the four look tools (definitions, argument parsing, replies, audit fields). `agent/mcp.rs`
  dispatches `tools/call` for them as `Action::Look`. `agent/mod.rs`: `approve`, `look`, `ask` (one place that turns a
  request into an `agentApproval`), `run_look` (thread + glow + reply + audit), and the debug `agent-test look`.
- `agent/approvals.rs`: `Purpose::Prompt | Look`, one-time grants for an approved prompt's exact input, remembered
  keys per run. `add_grant` only accepts `describe::is_desktop` tools today.
- `agent/describe.rs`: `INPUT_TOOLS` already names `focus_window`, `invoke`, `set_value`, `click`, `type_text`,
  `press_keys`, `launch` as risk `input`; `remember_key` returns `None` for them and `decide` never remembers `input`.
- `apps/activate.rs`: the foreground fallback chain (`bring_to_front`); it must run on the main thread (sync command
  or `run_on_main_thread`).
- `window/glow.rs`: the glow (`look_started` / `look_ended`), excluded from capture, below the island.
- The UI tree has no element references yet: lines carry control type, name, value, `#automationId` and bounds only.

## Scope for this session: stage 2.7d only

Build the input tools in PLAN.md, their approvals, summaries and step labels, the abort-on-your-input rule and the
agent guidance. Don't change how 2.7a–c's approvals work for other tools, and don't start 2.7e (routines).

## Before coding

1. **Check the installed Claude Code** (`claude --version`; 2.1.123 on PATH, 2.1.159 via npm, 2.1.31x in VS Code).
   Confirm that our input tools still reach `--permission-prompt-tool` in `default` mode, and how a user allow rule for
   them would behave (TopBar's own check must cover it, as D25 does for looks). Check how Claude Code handles a tool
   that takes several seconds and one that's cancelled mid-call (`notifications/cancelled`), and what a long
   `MCP_TOOL_TIMEOUT` means for an approval that waits.
2. **Check the Win32 pieces:**
   - UI Automation patterns first: `Invoke`, `Value` (`SetValue`), `Toggle`, `SelectionItem`, `ExpandCollapse`,
     `Scroll`; what each app class supports (Win32, WinUI/UWP, Chrome/Electron, WPF); doing them on an MTA thread with
     timeouts; which ones bring the window forward on their own.
   - How Claude names an element: element references in `ui_tree` output (runtime ids, a path of indices, or
     automation id + name + control type), how long they stay valid, and re-finding an element when the tree has
     changed. Never act on a stale guess.
   - `SendInput` as the fallback: it goes to whatever has focus, so the target must be foreground and checked again
     right before each event; absolute coordinates over the virtual desktop (0–65535, multi-monitor, DPI); Unicode
     text with `KEYEVENTF_UNICODE`; key chords; UIPI (input into elevated windows is silently dropped).
   - Telling your input from ours: `LLKHF_INJECTED` / `LLMHF_INJECTED` in a low-level hook (on its own thread, with a
     message loop, timing limits), or `GetLastInputInfo` (counts injected input too; probably not enough). The cost of
     a low-level hook that's only installed while an action runs.
   - `focus_window` through `apps::bring_to_front` on the main thread; what happens with minimized, cloaked and
     elevated targets and with the foreground lock.
   - `launch`: `ShellExecuteExW` for apps and URLs; allowed schemes (`https`, `http`, maybe `ms-settings:`); never
     `file:` executables or scripts by path, which go through the shell tool and its approvals instead.
   - List any new `windows` crate features.
3. **Propose, then wait for my OK:**
   - the module layout (e.g. `desktop/act.rs` + `desktop/input.rs` + a pure `desktop/plan.rs`, or a new folder), with
     Win32 isolated and the pure parts unit-tested (element references, coordinate mapping, chord parsing, text
     limits, summaries);
   - each tool's input and output schema: how an element is named, what a reply says (done, plus a fresh look at what
     changed?), errors (`windowGone`, `elementGone`, `elevated`, `notForeground`, `aborted`, `passwordField`,
     `topbarWindow`, `notResponding`, `screenLocked`);
   - approvals: risk `input`, one per action, never remembered, summaries that say exactly what will happen ("Click
     Save in Notepad · notes.txt", "Type 42 characters into Chrome · GitHub", "Press Ctrl+S in Notepad", "Open
     https://github.com in your browser"), whether a short listed batch can be approved at once, and what the island
     and the command window show (keep the eye for looks; something distinct for actions);
   - the abort rule: what counts as "you moved the mouse or typed", what's aborted (the current action and anything
     queued), what Claude is told, and how this works with the stop hotkey Win+Alt+Escape;
   - privacy and safety: no input into TopBar's own windows or password fields, no typing of anything that looks like
     a secret, audit entries that record what was done without the typed text (or with it cut and marked), and no
     action while the screen is locked or a UAC prompt is up;
   - the glow while acting: the same, stronger, or a different color than looking (brief for the motion-designer
     agent);
   - the `--append-system-prompt` additions (prefer the shell and files, then UI Automation patterns, then raw input;
     look before acting and after; one action at a time; say what you're about to do);
   - any UI changes, as a brief for the `motion-designer` agent.

## Rules for this stage

- Nothing acts without an approval for that exact action; nothing input-related is remembered for the run.
- Never focus, click or type into the island, the command window or the glow. The island stays unfocusable.
- Never type into a password field; never print or type secrets.
- Any physical input from me while an action runs aborts it, and Win+Alt+Escape stops the whole run.
- Every UI change goes through the `motion-designer` agent; delegate it, then review what it returns.
- **Testing:** synthetic input lands in whatever has focus, so don't send any while I'm at the machine unless I've said
  I'm ready. Prefer UI Automation actions on a scratch target (Notepad on a scratch file, a scratch folder in
  Explorer), `haiku`, low max turns, small prompts, and `topbar.exe agent-test …` in debug builds. Ask me before each
  test that sends real mouse or keyboard input, and tell me which window it will touch. Unit-test the pure parts
  (element references, coordinate mapping, chord parsing, summaries, risk and remember rules, MCP replies).
- The C: drive is nearly full: watch `target\debug` and use `cargo clean --profile dev` (or clear stale
  `deps/topbar*` and `incremental`) before it fills.

## When done

cargo fmt / clippy / test and pnpm typecheck clean. Verify from outside with small haiku runs, asking me before any
real input: focus a window, invoke a button through UI Automation, set a text field, a raw click and typed text into a
scratch Notepad, a key chord, launch a URL, Deny on each, "for this run" not offered for any, an action on a window
closed between look and act, an elevated window, a password field, and an action aborted by moving the mouse. Write
the manual check list for 2.7d in PLAN.md, update PLAN.md, CASESTUDY.md (decision entries and challenges) and
CLAUDE.md (layout, gotchas), and commit and push to main. Once I've signed off 2.7d, write the kick-off prompt for
2.7e (routines).
