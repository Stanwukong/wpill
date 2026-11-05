import { test } from "node:test";
import assert from "node:assert/strict";
import type { AgentEntry } from "../lib/types.ts";
import { turns } from "./transcriptTurns.ts";

let at = 0;
const e = (x: Record<string, unknown>) => ({ atMs: at++, ...x }) as AgentEntry;
const user = (text: string) => e({ kind: "user", text });
const text = (t: string) => e({ kind: "text", text: t });
const tool = (id: string, name: string) => e({ kind: "tool", id, name, label: `${name} · x` });
const result = (id: string, isError = false) => e({ kind: "toolResult", id, isError, text: "out" });

test("messages open turns; consecutive calls fold into one summarized group", () => {
  const t = turns([
    user("look"),
    text("Reading."),
    tool("1", "Read"),
    result("1"),
    tool("2", "Read"),
    tool("3", "Bash"),
    result("3"),
    result("2"),
    tool("4", "Read"),
    text("Done."),
    user("again"),
    tool("5", "Bash"),
    result("5", true),
  ]);
  assert.equal(t.length, 2);
  assert.equal(t[0].user?.text, "look");
  assert.deepEqual(
    t[0].items.map((i) => i.kind),
    ["text", "tools", "text"],
  );
  const g = t[0].items[1];
  assert.ok(g.kind === "tools");
  assert.equal(g.summary, "Read 3 files, ran a command");
  assert.equal(g.failed, false);
  assert.deepEqual(
    g.calls.map((c) => [c.tool?.id, c.result?.id ?? null]),
    [
      ["1", "1"],
      ["2", "2"],
      ["3", "3"],
      ["4", null],
    ],
  );
  const g2 = t[1].items[0];
  assert.ok(g2.kind === "tools" && g2.failed && g2.summary === "Ran a command");
});

test("approvals stay where they happened, a result after one still finds its call", () => {
  const t = turns([
    user("edit"),
    tool("1", "Edit"),
    e({ kind: "approval", requestId: 7, tool: "Edit", summary: "a.txt", risk: "write" }),
    e({ kind: "decision", requestId: 7, allow: true, by: "user" }),
    result("1"),
    e({ kind: "decision", requestId: 9, allow: false, by: "timeout" }),
    e({ kind: "result", status: "done", reason: "completed", turns: 1, costUsd: 0.01, durationMs: 1200 }),
  ]);
  const items = t[0].items;
  assert.deepEqual(
    items.map((i) => i.kind),
    ["tools", "approval", "decision", "result"],
  );
  assert.ok(items[0].kind === "tools" && items[0].calls[0].result?.id === "1" && items[0].summary === "Edited a file");
  assert.ok(items[1].kind === "approval" && items[1].decision?.allow === true);
  assert.ok(items[2].kind === "decision" && items[2].entry.requestId === 9, "a decision without its approval");
});

test("a transcript cut by the cap: no first message, a result without its call", () => {
  const t = turns([result("old"), text("…"), tool("a", "desktop · click"), tool("b", "mcp · thing"), tool("c", "Grep")]);
  assert.equal(t.length, 1);
  assert.equal(t[0].user, null);
  const [g, , g2] = t[0].items;
  assert.ok(g.kind === "tools" && g.calls[0].tool === null && g.summary === "An output");
  assert.ok(g2.kind === "tools" && g2.summary === "Used the desktop, used mcp · thing, searched");
});
