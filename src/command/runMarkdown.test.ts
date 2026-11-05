import { test } from "node:test";
import assert from "node:assert/strict";
import type { AgentEntry } from "../lib/types.ts";
import { formatDuration, resultParts, toMarkdown } from "./runMarkdown.ts";

const e = (x: Record<string, unknown>) => ({ atMs: 0, ...x }) as AgentEntry;

test("durations", () => {
  assert.equal(formatDuration(860), "0.9 s");
  assert.equal(formatDuration(42_400), "42 s");
  assert.equal(formatDuration(185_000), "3 min 5 s");
  assert.equal(formatDuration(120_000), "2 min");
  assert.equal(formatDuration(3_720_000), "1 h 2 min");
  assert.equal(formatDuration(-5), "0.0 s");
});

test("the summary skips what isn't known", () => {
  const r = e({ kind: "result", status: "done", reason: "completed", turns: 1, costUsd: null, durationMs: null });
  assert.deepEqual(resultParts(r as Extract<AgentEntry, { kind: "result" }>), { outcome: "Done", details: ["1 turn"] });
});

test("a run as Markdown", () => {
  const md = toMarkdown(
    [
      e({ kind: "user", text: "Fix it " }),
      e({ kind: "text", text: "Looking at **a.txt**." }),
      e({ kind: "tool", id: "1", name: "Bash", label: "Bash · echo `hi`" }),
      e({ kind: "toolResult", id: "1", isError: true, text: "boom\nmore" }),
      e({ kind: "approval", requestId: 3, tool: "Edit", summary: "a.txt", risk: "write" }),
      e({ kind: "decision", requestId: 3, allow: true, by: "user" }),
      e({ kind: "result", status: "done", reason: "completed", turns: 2, costUsd: 0.0123, durationMs: 4200 }),
    ],
    "wpill",
  );
  assert.equal(
    md,
    [
      "# wpill",
      "## You\n\nFix it",
      "## Claude",
      "Looking at **a.txt**.",
      "- `` Bash · echo `hi` ``\n  - Error: boom",
      "> Asked: Edit a.txt. Allowed by you.",
      "---\n\n**Done** · 2 turns · 4.2 s · $0.0123",
    ].join("\n\n") + "\n",
  );
});
