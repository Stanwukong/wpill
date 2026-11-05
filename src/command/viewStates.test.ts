import { test } from "node:test";
import assert from "node:assert/strict";
import { runViewState, showsStartHint, unshownRunError, type TranscriptLoad } from "./viewStates.ts";
import type { AgentEntry } from "../lib/types";

const user: AgentEntry = { kind: "user", text: "fix the tests", atMs: 1 };
const failed: AgentEntry = { kind: "error", message: "Claude Code exited", atMs: 2 };

const loading: TranscriptLoad = { entries: [], loading: true, error: null, draft: null };
const loaded = (entries: AgentEntry[], extra: Partial<TranscriptLoad> = {}): TranscriptLoad => ({
  entries,
  loading: false,
  error: null,
  draft: null,
  ...extra,
});
const run = { error: null };

test("loading until the snapshot arrives, unless live entries already show", () => {
  assert.equal(runViewState(run, true, true, undefined), "loading");
  assert.equal(runViewState(run, true, true, loading), "loading");
  assert.equal(runViewState(run, true, true, { ...loading, entries: [user] }), "ready");
});

test("a failed load with nothing to show, and an empty one", () => {
  assert.equal(runViewState(run, true, true, loaded([], { error: "boom" })), "failed");
  assert.equal(runViewState(run, true, true, loaded([user], { error: "boom" })), "ready");
  assert.equal(runViewState(run, true, true, loaded([])), "empty");
  assert.equal(runViewState(run, true, true, loaded([], { draft: "Look" })), "ready");
});

test("gone: a run seen before, or one whose transcript Rust refused", () => {
  assert.equal(runViewState(undefined, true, true, loaded([user])), "gone");
  assert.equal(runViewState(undefined, true, false, loaded([], { error: "That agent run is gone." })), "gone");
});

test("not gone right after a start (not listed yet) or before the list has arrived", () => {
  assert.equal(runViewState(undefined, true, false, loaded([user])), "ready");
  assert.equal(runViewState(undefined, true, false, loading), "loading");
  assert.equal(runViewState(undefined, false, true, loaded([], { error: "x" })), "failed");
});

test("the run's error shows only when the transcript doesn't hold it", () => {
  assert.equal(unshownRunError({ error: null }, []), null);
  assert.equal(unshownRunError(undefined, []), null);
  assert.equal(unshownRunError({ error: "Claude Code exited" }, [user, failed]), null);
  assert.equal(unshownRunError({ error: "Claude Code exited" }, [user]), "Claude Code exited");
});

test("the sidebar hint waits for both lists and needs both empty", () => {
  assert.equal(showsStartHint(true, true, 0, 0), true);
  assert.equal(showsStartHint(false, true, 0, 0), false);
  assert.equal(showsStartHint(true, false, 0, 0), false);
  assert.equal(showsStartHint(true, true, 1, 0), false);
  assert.equal(showsStartHint(true, true, 0, 2), false);
});
