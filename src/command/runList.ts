import type { AgentRun } from "../lib/types";

/** What the run list reads of a run (2.10d): enough to sort, group and filter it. */
export type ListedRun = Pick<AgentRun, "runId" | "backend" | "project" | "prompt" | "model" | "startedAtMs" | "updatedAtMs"> & {
  routine: { name: string } | null;
};

export type RunGroupLabel = "Today" | "Earlier";

export interface RunGroup<R extends ListedRun> {
  label: RunGroupLabel;
  runs: R[];
}

/** Newest first by start; runs started together, the one updated last first. Returns a new array. */
export function sortRuns<R extends ListedRun>(runs: readonly R[]): R[] {
  return [...runs].sort((a, b) => b.startedAtMs - a.startedAtMs || b.updatedAtMs - a.updatedAtMs);
}

/** The query's words, lowercased; none for an empty or blank query. */
export function queryWords(query: string): string[] {
  return query.toLowerCase().split(/\s+/).filter(Boolean);
}

/** Every word appears somewhere in the run's project, prompt, routine name, agent or model. No words: a match. */
export function matchesQuery(run: ListedRun, words: readonly string[]): boolean {
  if (words.length === 0) return true;
  const text = [run.project, run.prompt, run.routine?.name ?? "", run.backend, run.model ?? ""].join("\n").toLowerCase();
  return words.every((w) => text.includes(w));
}

/** Local midnight of the day holding `ms`. */
function startOfDay(ms: number): number {
  const d = new Date(ms);
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

/**
 * The list as shown: filtered by `query`, newest first, in "Today" (started on `nowMs`'s local calendar day, or later)
 * and "Earlier". Empty groups are left out.
 */
export function runGroups<R extends ListedRun>(runs: readonly R[], query: string, nowMs: number): RunGroup<R>[] {
  const words = queryWords(query);
  const today = startOfDay(nowMs);
  const groups: Record<RunGroupLabel, R[]> = { Today: [], Earlier: [] };
  for (const r of sortRuns(runs)) {
    if (matchesQuery(r, words)) groups[r.startedAtMs >= today ? "Today" : "Earlier"].push(r);
  }
  return (["Today", "Earlier"] as const).filter((l) => groups[l].length > 0).map((label) => ({ label, runs: groups[label] }));
}
