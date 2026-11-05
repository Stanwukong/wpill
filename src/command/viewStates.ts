import type { AgentEntry, AgentRun } from "../lib/types";

/** What these checks read of a run. */
export type KnownRun = Pick<AgentRun, "error">;

/**
 * Which state a run's view is in (2.10d.4): `gone` once Rust no longer lists it (seen before, or its transcript says
 * so), `loading` until its snapshot arrives, `failed` if that load failed, `empty` with nothing to show, else `ready`.
 */
export type RunViewState = "gone" | "loading" | "failed" | "empty" | "ready";

export interface TranscriptLoad {
  entries: AgentEntry[];
  loading: boolean;
  error: string | null;
  draft: string | null;
}

export function runViewState(
  run: KnownRun | undefined,
  runsLoaded: boolean,
  seen: boolean,
  transcript: TranscriptLoad | undefined,
): RunViewState {
  // Right after a start the run can be missing until `agent:runs` arrives; its transcript loads fine meanwhile.
  if (!run && runsLoaded && (seen || (transcript && !transcript.loading && transcript.error !== null))) return "gone";
  if (!transcript || (transcript.loading && transcript.entries.length === 0)) return "loading";
  if (transcript.error !== null && transcript.entries.length === 0) return "failed";
  if (transcript.entries.length === 0 && transcript.draft === null) return "empty";
  return "ready";
}

/** The run's error, when the transcript doesn't already show it as an error entry (cut, or not loaded). */
export function unshownRunError(run: KnownRun | undefined, entries: AgentEntry[]): string | null {
  const error = run?.error ?? null;
  if (error === null) return null;
  return entries.some((e) => e.kind === "error" && e.message === error) ? null : error;
}

/** The sidebar's one-line hint: only once both lists have arrived and both are empty. */
export function showsStartHint(runsLoaded: boolean, routinesLoaded: boolean, runs: number, routines: number): boolean {
  return runsLoaded && routinesLoaded && runs === 0 && routines === 0;
}
