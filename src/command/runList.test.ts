import { test } from "node:test";
import assert from "node:assert/strict";
import { matchesQuery, queryWords, runGroups, sortRuns, type ListedRun } from "./runList.ts";

/** Local times, so "today" doesn't depend on the machine's time zone. */
const at = (day: number, hour: number, minute = 0) => new Date(2026, 10, day, hour, minute).getTime();
const NOW = at(5, 14);

function run(id: string, startedAtMs: number, extra: Partial<ListedRun> = {}): ListedRun {
  return {
    runId: id,
    backend: "claude",
    project: "wpill",
    prompt: "fix the tests",
    model: null,
    startedAtMs,
    updatedAtMs: startedAtMs,
    routine: null,
    ...extra,
  };
}

const ids = (runs: readonly ListedRun[]) => runs.map((r) => r.runId);

test("newest start first; ties by the last update", () => {
  const runs = [run("a", at(5, 9)), run("b", at(5, 12)), run("c", at(5, 9), { updatedAtMs: at(5, 10) }), run("d", at(4, 23))];
  assert.deepEqual(ids(sortRuns(runs)), ["b", "c", "a", "d"]);
  assert.deepEqual(ids(runs), ["a", "b", "c", "d"], "the input is left alone");
});

test("today is the local calendar day of now, from midnight", () => {
  const runs = [run("midnight", at(5, 0)), run("late", at(4, 23, 59)), run("old", at(1, 10)), run("now", NOW)];
  const groups = runGroups(runs, "", NOW);
  assert.deepEqual(
    groups.map((g) => [g.label, ids(g.runs)]),
    [
      ["Today", ["now", "midnight"]],
      ["Earlier", ["late", "old"]],
    ],
  );
});

test("a run that started after now (clock change) counts as today", () => {
  assert.deepEqual(runGroups([run("a", at(5, 15))], "", NOW).map((g) => g.label), ["Today"]);
});

test("empty groups are left out", () => {
  assert.deepEqual(runGroups([run("a", at(2, 9))], "", NOW).map((g) => g.label), ["Earlier"]);
  assert.deepEqual(runGroups([run("a", at(5, 9))], "", NOW).map((g) => g.label), ["Today"]);
  assert.deepEqual(runGroups([], "", NOW), []);
  assert.deepEqual(runGroups([run("a", at(5, 9))], "nothing", NOW), []);
});

test("query words: lowercase, split on any whitespace, blank is none", () => {
  assert.deepEqual(queryWords("  Fix\tTESTS \n"), ["fix", "tests"]);
  assert.deepEqual(queryWords("   "), []);
  assert.deepEqual(queryWords(""), []);
});

test("every word must match, each anywhere, case-insensitively", () => {
  const r = run("a", NOW, {
    project: "Cay-Docs",
    prompt: "Update the README",
    backend: "codex",
    model: "gpt-6-luna",
    routine: { name: "Morning sweep" },
  });
  for (const q of ["docs", "readme", "MORNING", "codex", "luna", "cay readme codex", "sweep gpt"]) {
    assert.ok(matchesQuery(r, queryWords(q)), q);
  }
  for (const q of ["claude", "readme claude", "evening"]) {
    assert.ok(!matchesQuery(r, queryWords(q)), q);
  }
});

test("a word doesn't match across two fields", () => {
  const r = run("a", NOW, { project: "alpha", prompt: "beta" });
  assert.ok(!matchesQuery(r, queryWords("alphabeta")));
  assert.ok(!matchesQuery(r, queryWords("alpha beta gamma")));
});

test("no model or routine is fine", () => {
  const r = run("a", NOW);
  assert.ok(matchesQuery(r, []));
  assert.ok(!matchesQuery(r, queryWords("null")));
});

test("the filter keeps the order and the grouping", () => {
  const runs = [
    run("a", at(5, 9), { project: "web" }),
    run("b", at(5, 11), { project: "api" }),
    run("c", at(3, 9), { project: "web", prompt: "old" }),
    run("d", at(5, 12), { project: "web" }),
  ];
  assert.deepEqual(
    runGroups(runs, "WEB", NOW).map((g) => [g.label, ids(g.runs)]),
    [
      ["Today", ["d", "a"]],
      ["Earlier", ["c"]],
    ],
  );
});
