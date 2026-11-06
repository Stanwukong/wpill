import type { TimerKind, TimerState } from "../../lib/types";

/** Kinds with a duration (and a ring): everything but the stopwatch. */
export function isCountdown(kind: TimerKind): boolean {
  return kind !== "stopwatch";
}

/** The pill shows a timer while it runs or is paused; an ended one is the `timerDone` activity's (and the panel's). */
export function showsInPill(timer: TimerState | null): timer is TimerState {
  return timer !== null && timer.phase !== "ended";
}

const pad = (n: number) => String(n).padStart(2, "0");

/**
 * `mm:ss`, or `h:mm:ss` from an hour. A countdown rounds up (it reads 05:00 at the start and 00:00 only at the end),
 * a stopwatch rounds down (00:01 after a full second).
 */
export function clockText(ms: number, round: "up" | "down" = "down"): string {
  const seconds = Math.max(0, round === "up" ? Math.ceil(ms / 1000) : Math.floor(ms / 1000));
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = seconds % 60;
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${pad(m)}:${pad(s)}`;
}

/** What a countdown has left (never negative); a stopwatch's elapsed time. From Rust's latest state only. */
export function shownMs(timer: TimerState): number {
  if (isCountdown(timer.kind) && timer.durationMs !== null) {
    return Math.max(0, timer.durationMs - timer.elapsedMs);
  }
  return timer.elapsedMs;
}

/** The time as the island shows it. */
export function timerText(timer: TimerState): string {
  return clockText(shownMs(timer), isCountdown(timer.kind) ? "up" : "down");
}

/** The share of a countdown still left (1 at the start, 0 at the end), for the ring; null for a stopwatch. */
export function remainingFraction(timer: TimerState): number | null {
  if (!isCountdown(timer.kind) || timer.durationMs === null || timer.durationMs <= 0) return null;
  return Math.min(1, Math.max(0, shownMs(timer) / timer.durationMs));
}

/** The panel row's name for what runs. */
export function kindLabel(kind: TimerKind): string {
  switch (kind) {
    case "countdown":
      return "Timer";
    case "stopwatch":
      return "Stopwatch";
    case "focus":
      return "Focus";
    case "break":
      return "Break";
  }
}

/** The `timerDone` title. */
export function doneTitle(kind: TimerKind): string {
  if (kind === "focus") return "Focus done";
  if (kind === "break") return "Break over";
  return "Timer done";
}

const localTime = new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit" });

/** "Ended at 14:05", in local time (the format is a parameter for tests). */
export function endedLabel(endedAtMs: number, format: Intl.DateTimeFormat = localTime): string {
  return `Ended at ${format.format(endedAtMs)}`;
}
