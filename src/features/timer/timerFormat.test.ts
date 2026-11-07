import { test } from "node:test";
import assert from "node:assert/strict";
import {
  clockText,
  doneTitle,
  endedLabel,
  isCountdown,
  kindLabel,
  remainingFraction,
  showsInPill,
  shownMs,
  timerText,
} from "./timerFormat.ts";
import type { TimerState } from "../../lib/types";

const MIN = 60_000;

const state = (extra: Partial<TimerState> = {}): TimerState => ({
  kind: "countdown",
  phase: "running",
  startedMs: 1_000_000,
  durationMs: 5 * MIN,
  elapsedMs: 0,
  deadlineMs: 1_000_000 + 5 * MIN,
  endedAtMs: null,
  nowMs: 1_000_000,
  ...extra,
});

test("clockText: mm:ss under an hour, h:mm:ss from an hour", () => {
  assert.equal(clockText(0), "00:00");
  assert.equal(clockText(9_000), "00:09");
  assert.equal(clockText(5 * MIN), "05:00");
  assert.equal(clockText(59 * MIN + 59_000), "59:59");
  assert.equal(clockText(60 * MIN), "1:00:00");
  assert.equal(clockText(2 * 60 * MIN + 3 * MIN + 4_000), "2:03:04");
});

test("clockText: countdowns round up, stopwatches down, never negative", () => {
  assert.equal(clockText(299_001, "up"), "05:00");
  assert.equal(clockText(298_999, "up"), "04:59");
  assert.equal(clockText(1_999, "down"), "00:01");
  assert.equal(clockText(-500, "up"), "00:00");
  assert.equal(clockText(-500), "00:00");
});

test("a countdown shows what's left", () => {
  assert.equal(timerText(state()), "05:00");
  assert.equal(timerText(state({ elapsedMs: 1_003 })), "04:59");
  assert.equal(timerText(state({ elapsedMs: 5 * MIN })), "00:00");
  assert.equal(shownMs(state({ elapsedMs: 6 * MIN })), 0, "past the end");
  assert.equal(timerText(state({ kind: "focus", durationMs: 25 * MIN, elapsedMs: 10 * MIN })), "15:00");
});

test("a stopwatch shows what has passed", () => {
  const sw = state({ kind: "stopwatch", durationMs: null, deadlineMs: null, elapsedMs: 61_900 });
  assert.equal(timerText(sw), "01:01");
  assert.equal(timerText({ ...sw, elapsedMs: 3 * 60 * MIN + 5_000 }), "3:00:05");
});

test("remainingFraction: 1 → 0 for countdowns, null for a stopwatch", () => {
  assert.equal(remainingFraction(state()), 1);
  assert.equal(remainingFraction(state({ elapsedMs: 2.5 * MIN })), 0.5);
  assert.equal(remainingFraction(state({ elapsedMs: 9 * MIN })), 0);
  assert.equal(remainingFraction(state({ kind: "stopwatch", durationMs: null })), null);
  assert.equal(remainingFraction(state({ durationMs: 0 })), null);
});

test("the pill shows running and paused timers, not ended ones", () => {
  assert.equal(showsInPill(null), false);
  assert.equal(showsInPill(state()), true);
  assert.equal(showsInPill(state({ phase: "paused" })), true);
  assert.equal(showsInPill(state({ phase: "ended", endedAtMs: 1_300_000 })), false);
});

test("labels", () => {
  assert.equal(isCountdown("break"), true);
  assert.equal(isCountdown("stopwatch"), false);
  assert.equal(kindLabel("countdown"), "Timer");
  assert.equal(kindLabel("focus"), "Focus");
  assert.equal(doneTitle("countdown"), "Timer done");
  assert.equal(doneTitle("focus"), "Focus done");
  assert.equal(doneTitle("break"), "Break over");
});

test("endedLabel uses the given local format", () => {
  const utc = new Intl.DateTimeFormat("en-GB", { hour: "2-digit", minute: "2-digit", timeZone: "UTC" });
  assert.equal(endedLabel(Date.UTC(2026, 10, 6, 14, 5), utc), "Ended at 14:05");
  assert.equal(endedLabel(Date.UTC(2026, 10, 6, 9, 0), utc), "Ended at 09:00");
});
