Start stage 2.7c (read-only desktop tools) of TopBar's Phase 2.7.

Read PLAN.md first: Current status, "Phase 2.7: local automation agent" (principles, the research notes, what 2.7a–b
built, stage 2.7c and the stages after it), the Phase 2.7 (a–b) IPC table and Known limitations. Then read the
CASESTUDY.md decision log, especially D21 (why desktop tools live in TopBar's own MCP server), D22 (runs as
`claude -p` in a Job Object) and D23 (the token-checked pipe, risk classes, "allow for this run"). CLAUDE.md is loaded;
follow its rules and Windows gotchas, above all rule 2 (the island never takes keyboard input) and "Agent runs never
weaken permissions". Phase 2.7a–b are signed off (2026-10-30, all 12 manual checks pass).

Goal: Claude, in a run TopBar started, can *look at* the desktop: list windows, see which one is focused, take a
screenshot of a window or the screen, and read a window's UI Automation tree. It can't click, type or focus anything
yet (that's 2.7d). Every look is approved from the island, so I always know when Claude is looking at my screen.

## What 2.7a–b give you (read the code before designing)

- `src-tauri/src/agent/mcp.rs`: TopBar's MCP server logic (pure, tested). `tools/list` returns only `approve` today;
  `tools/call` dispatches by name. `bridge.rs` (`topbar.exe mcp`) only relays; `pipe.rs` and `mod.rs` hold the
  connection threads and answer calls. Messages are capped at 4 MiB per message, which limits screenshot size.
- `agent/describe.rs`: `risk()` (`READ_TOOLS`, `INPUT_TOOLS` with the 2.7d names already listed, everything else is
  `change`), the summaries and step labels, `remember_key()`. TopBar's new read tools are `mcp__topbar__*` names that
  `risk()` doesn't know yet, so today they'd count as `change`.
- `agent/run.rs`: the command line (`--strict-mcp-config`, `--permission-prompt-tool mcp__topbar__approve`,
  `--disallowedTools AskUserQuestion`, `--append-system-prompt` with TopBar's guidance).
- `src-tauri/src/apps/`: `enumerate.rs` (Alt+Tab filter, titles, exe paths, UWP via ApplicationFrameHost),
  `icon.rs`, `mod.rs` (`top_level_windows`). Reuse these; don't copy them.
- `window/`: the island's HWND, and the command window's. Neither may ever appear in a screenshot or tree.

## Scope for this session: stage 2.7c only

Build the four read-only tools in PLAN.md (`list_windows`, `focused_window`, `screenshot`, `ui_tree`), their approvals,
summaries and step labels, and the agent guidance that tells Claude when to use them. Don't add `focus_window`, `click`,
`type_text` or any other input tool. Don't change how 2.7a–b's approvals work for other tools.

## Before coding

1. **Check the current docs and the installed Claude Code** (`claude --version`; 2.1.123 on PATH, 2.1.159 via npm,
   2.1.312 in VS Code). Pin down how MCP *image* content (`{"type":"image","data":…,"mimeType":…}`) in a tool result
   reaches the model in `-p` runs, the size Claude Code accepts or resizes images to, whether it's charged as image
   tokens, and how a tool result that's too large is handled. Check whether a tool that our own server lists still
   goes through `--permission-prompt-tool` in `default` mode (it should; confirm), and whether `--allowedTools` could
   pre-approve one per run instead (we likely won't use it; say why).
2. **Check the Win32 pieces:** capturing a window that may be covered or partly off-screen (`PrintWindow` with
   `PW_RENDERFULLCONTENT` vs `BitBlt` from the screen vs Windows.Graphics.Capture, and which ones show a yellow
   border or need a newer build; this machine is 23H2, 22631). Excluding our own windows from a screen capture
   (`SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` on the island and command window, or masking them; check it
   doesn't affect DWM or the click-through gotchas). Downscaling and PNG encoding without a big new dependency (the
   app is ~1.6 MB; `image` crate features, or WIC through `windows`). Physical px vs DPI in the coordinates we return
   (CLAUDE.md's DPI gotcha). UI Automation from Rust (`IUIAutomation`, `TreeWalker` or `FindAll` with a cache request,
   `CoInitializeEx` on its own MTA thread, timeouts for hung apps, `IsPassword`). List any new `windows` crate
   features (e.g. `Win32_UI_Accessibility`, `Win32_Graphics_Imaging`, `Win32_Storage_Xps` for `PrintWindow`).
3. **Propose, then wait for my OK:**
   - the module layout (e.g. `src-tauri/src/agent/desktop/` or a new `desktop/` feature folder with Win32 isolated,
     pure parts unit-tested: tree trimming, size caps, summaries);
   - each tool's input and output schema: how a window is named (HWND number from `list_windows`, revalidated with
     `IsWindow`, rule 6), the screenshot's max size and format, the tree's depth / node / byte caps and which properties
     it carries (name, control type, value, bounding box, enabled, focusable); password fields with no value; errors
     for gone, elevated (UIPI) and hung windows;
   - privacy: never our own windows; what happens with DRM or protected content (black), secure desktop (UAC, lock
     screen), other virtual desktops and cloaked windows;
   - approvals: risk `read`, one approval per tool *kind* per run ("Allow screenshots for this run"), the summaries
     (e.g. "Screenshot of Chrome · GitHub", "Read the UI of Notepad"), what the island and the command window show, and
     whether `list_windows` / `focused_window` are mild enough to ask about once together;
   - the audit entries (record that a screenshot was taken and of what, never the image) and what the command
     window's transcript shows for an image result;
   - the added `--append-system-prompt` guidance (prefer the shell and files; `ui_tree` before `screenshot`; say what
     you're looking for);
   - any UI changes, as a brief for the `motion-designer` agent (probably a risk badge or icon in the approval
     activity; keep it small).

## Rules for this stage

- No input of any kind: nothing in 2.7c may focus, click, type, move or resize a window.
- Never capture or describe the island or the command window. Never return a password field's value.
- Every UI change goes through the `motion-designer` agent; delegate it, then review what it returns.
- Tests must not spend much of my usage: `haiku`, low max turns, small prompts, and `topbar.exe agent-test …` in debug
  builds. Don't send synthetic keystrokes while I'm at the machine. Unit-test the pure parts (schemas, caps,
  trimming, risk mapping, summaries, MCP replies with image content).

## When done

cargo fmt / clippy / test and pnpm typecheck clean. Verify from outside with small haiku runs: "what windows are
open", "what does Notepad say" (ui_tree), "describe what's on screen" (screenshot), Deny on each, "for this run" on a
second call, a window closed between list and call, an elevated window, and a screenshot with the island open (it must
not appear). Check the image size stays under the pipe cap. Write the manual check list for 2.7c in PLAN.md, update
PLAN.md, CASESTUDY.md (decision entries and challenges) and CLAUDE.md (layout, gotchas), and commit and push to main.
Once I've signed off 2.7c and used it for a while, write the kick-off prompt for 2.7d.
