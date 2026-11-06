import { test } from "node:test";
import assert from "node:assert/strict";
import { nextIndex } from "./listKeys.ts";

test("arrows step one row and stop at the ends", () => {
  assert.equal(nextIndex("ArrowDown", 0, 3), 1);
  assert.equal(nextIndex("ArrowUp", 2, 3), 1);
  assert.equal(nextIndex("ArrowDown", 2, 3), null);
  assert.equal(nextIndex("ArrowUp", 0, 3), null);
});

test("Home and End jump to the first and last row", () => {
  assert.equal(nextIndex("Home", 2, 3), 0);
  assert.equal(nextIndex("End", 0, 3), 2);
  assert.equal(nextIndex("Home", 0, 3), null);
  assert.equal(nextIndex("End", 2, 3), null);
});

test("from the filter box only Down enters the list", () => {
  assert.equal(nextIndex("ArrowDown", -1, 3), 0);
  assert.equal(nextIndex("ArrowUp", -1, 3), null);
  assert.equal(nextIndex("Home", -1, 3), null);
  assert.equal(nextIndex("End", -1, 3), null);
});

test("an empty list and other keys move nothing", () => {
  assert.equal(nextIndex("ArrowDown", -1, 0), null);
  assert.equal(nextIndex("Enter", 1, 3), null);
  assert.equal(nextIndex("ArrowLeft", 1, 3), null);
});
