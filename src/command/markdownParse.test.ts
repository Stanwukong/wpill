// `pnpm test`: Node's own runner on the pure parser (Node strips the types; no test dependency).
import { test } from "node:test";
import assert from "node:assert/strict";
import { cells, inline, openable, parse } from "./markdownParse.ts";

test("tables with alignment, short rows padded, extra cells dropped", () => {
  const blocks = parse("Intro\n| Name | Size | Note |\n|:--|--:|:-:|\n| a | 1 |\n| b | 2 | x | extra |\nAfter");
  assert.deepEqual(blocks[0], { kind: "paragraph", text: "Intro" });
  assert.deepEqual(blocks[1], {
    kind: "table",
    align: ["left", "right", "center"],
    head: ["Name", "Size", "Note"],
    rows: [
      ["a", "1", ""],
      ["b", "2", "x"],
    ],
  });
  assert.deepEqual(blocks[2], { kind: "paragraph", text: "After" });
});

test("a table without outer pipes, and pipes without a delimiter row stay monospace", () => {
  assert.equal(parse("a | b\n--- | ---\n1 | 2")[0].kind, "table");
  assert.deepEqual(parse("| just | pipes |"), [{ kind: "pre", text: "| just | pipes |" }]);
});

test("cells keep escaped pipes and pipes inside code", () => {
  assert.deepEqual(cells("| `a|b` | c \\| d |"), ["`a|b`", "c | d"]);
});

test("fences carry their language", () => {
  assert.deepEqual(parse("```TS\nlet a = 1;\n```"), [{ kind: "code", text: "let a = 1;", lang: "ts" }]);
  assert.deepEqual(parse("~~~\nx\n~~~"), [{ kind: "code", text: "x", lang: "" }]);
});

test("links, autolinks and strikethrough", () => {
  assert.deepEqual(inline("see [the **docs**](https://tauri.app/v2 \"Tauri\") now"), [
    "see ",
    { kind: "link", href: "https://tauri.app/v2", children: ["the ", { kind: "strong", children: ["docs"] }] },
    " now",
  ]);
  assert.deepEqual(inline("at https://x.dev/a_b. Or <https://y.dev>"), [
    "at ",
    { kind: "link", href: "https://x.dev/a_b", children: ["https://x.dev/a_b"] },
    ". Or ",
    { kind: "link", href: "https://y.dev", children: ["https://y.dev"] },
  ]);
  assert.deepEqual(inline("~~old~~ and `https://not.a/link`"), [
    { kind: "del", children: ["old"] },
    " and ",
    { kind: "code", text: "https://not.a/link" },
  ]);
});

test("only https links open", () => {
  assert.ok(openable("https://github.com/x"));
  for (const bad of ["http://a.com", "javascript:alert(1)", "file:///C:/x", "./README.md", "https://", "https://@x"])
    assert.ok(!openable(bad), bad);
});

test("what 2.8c parsed still parses the same", () => {
  const blocks = parse("# Plan\n\n1. one\n   - nested\n2. two\n\n> quoted\n\n---\nText with *em* and __strong__.");
  assert.deepEqual(
    blocks.map((b) => b.kind),
    ["heading", "list", "quote", "rule", "paragraph"],
  );
  assert.deepEqual(inline("Text with *em* and __strong__."), [
    "Text with ",
    { kind: "em", children: ["em"] },
    " and ",
    { kind: "strong", children: ["strong"] },
    ".",
  ]);
});
