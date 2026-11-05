import { test } from "node:test";
import assert from "node:assert/strict";
import { draftStore } from "./composerDrafts.ts";

test("a draft is kept per run", () => {
  const s = draftStore();
  s.set("a", "fix the tests");
  s.set("b", "and then\nthe docs");
  assert.equal(s.get("a"), "fix the tests");
  assert.equal(s.get("b"), "and then\nthe docs");
  assert.equal(s.get("c"), undefined);
});

test("text is kept as typed, spaces and all", () => {
  const s = draftStore();
  s.set("a", "  two words ");
  assert.equal(s.get("a"), "  two words ");
});

test("empty or blank text deletes the draft", () => {
  const s = draftStore();
  s.set("a", "x");
  s.set("a", "");
  assert.equal(s.get("a"), undefined);
  s.set("a", "x");
  s.set("a", " \n\t ");
  assert.equal(s.get("a"), undefined);
  assert.equal(s.size, 0);
});

test("clear forgets one run only", () => {
  const s = draftStore();
  s.set("a", "x");
  s.set("b", "y");
  s.clear("a");
  s.clear("missing");
  assert.equal(s.get("a"), undefined);
  assert.equal(s.get("b"), "y");
});

test("past the cap the draft written longest ago goes", () => {
  const s = draftStore(3);
  s.set("a", "1");
  s.set("b", "2");
  s.set("c", "3");
  s.set("a", "1 again"); // a is now the newest
  s.set("d", "4");
  assert.equal(s.size, 3);
  assert.equal(s.get("b"), undefined);
  assert.equal(s.get("a"), "1 again");
  assert.equal(s.get("c"), "3");
  assert.equal(s.get("d"), "4");
});

test("the default cap is 50", () => {
  const s = draftStore();
  for (let i = 0; i < 60; i++) s.set(`run${i}`, `text ${i}`);
  assert.equal(s.size, 50);
  assert.equal(s.get("run9"), undefined);
  assert.equal(s.get("run10"), "text 10");
  assert.equal(s.get("run59"), "text 59");
});
