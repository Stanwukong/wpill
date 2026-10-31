import type { RoutineDraft, RoutineView, Schedule, SkipReason, Weekday } from "../lib/types";
import { ago, endLabel } from "../features/agent/labels";

// How routines read in the command window. Everything is computed at render (like `ago`); nothing ticks.

/** What a new routine starts from: "Save as routine" fills what it knows. */
export type RoutineSeed = Partial<Omit<RoutineDraft, "maxTurns" | "maxBudgetUsd">> & {
  maxTurns?: number | null;
  maxBudgetUsd?: number | null;
};

export const WEEKDAYS: { day: Weekday; short: string; letter: string }[] = [
  { day: "mon", short: "Mon", letter: "M" },
  { day: "tue", short: "Tue", letter: "T" },
  { day: "wed", short: "Wed", letter: "W" },
  { day: "thu", short: "Thu", letter: "T" },
  { day: "fri", short: "Fri", letter: "F" },
  { day: "sat", short: "Sat", letter: "S" },
  { day: "sun", short: "Sun", letter: "S" },
];

export const WORKDAYS: Weekday[] = ["mon", "tue", "wed", "thu", "fri"];
export const ALL_DAYS: Weekday[] = WEEKDAYS.map((w) => w.day);

/** Days in week order. */
export function sortDays(days: Weekday[]): Weekday[] {
  return ALL_DAYS.filter((d) => days.includes(d));
}

function sameDays(a: Weekday[], b: Weekday[]): boolean {
  return a.length === b.length && b.every((d) => a.includes(d));
}

/** "Weekdays", "Every day", "Weekends", "Mon, Wed". */
export function daysLabel(days: Weekday[]): string {
  if (sameDays(days, ALL_DAYS)) return "Every day";
  if (sameDays(days, WORKDAYS)) return "Weekdays";
  if (sameDays(days, ["sat", "sun"])) return "Weekends";
  if (days.length === 0) return "No days";
  return sortDays(days)
    .map((d) => WEEKDAYS.find((w) => w.day === d)?.short ?? d)
    .join(", ");
}

/** The schedule alone: "Weekdays 09:00", "Every 4 h from 08:00", "At TopBar start", "Off". */
export function scheduleLabel(schedule: Schedule): string {
  switch (schedule.kind) {
    case "off":
      return "Off";
    case "days":
      return `${daysLabel(schedule.days)} ${schedule.time}`;
    case "every":
      return `Every ${schedule.hours} h from ${schedule.from}`;
    case "start":
      return "At TopBar start";
  }
}

/** "in 12 m", "in 3 h", "Mon 09:00". */
export function untilLabel(atMs: number, now = Date.now()): string {
  const minutes = Math.ceil((atMs - now) / 60_000);
  if (minutes <= 1) return "in a minute";
  if (minutes < 60) return `in ${minutes} m`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `in ${hours} h`;
  return dayTime(atMs);
}

function hhmm(d: Date): string {
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

/** "09:00" today, "Mon 09:00" otherwise. */
export function dayTime(atMs: number, now = Date.now()): string {
  const d = new Date(atMs);
  if (d.toDateString() === new Date(now).toDateString()) return hhmm(d);
  return `${WEEKDAYS[(d.getDay() + 6) % 7].short} ${hhmm(d)}`;
}

/** The sidebar's schedule line: the schedule and when it runs next ("Weekdays 09:00 · next in 3 h"). */
export function scheduleLine(routine: RoutineView, paused: boolean): string {
  const base = scheduleLabel(routine.schedule);
  if (routine.schedule.kind === "off") return base;
  if (!routine.enabled) return `${base} · off`;
  if (paused) return `${base} · paused`;
  if (routine.nextRunAtMs != null) {
    const until = untilLabel(routine.nextRunAtMs);
    return `${base} · next ${until}`;
  }
  return base;
}

const SKIP_LABEL: Record<SkipReason, string> = {
  missed: "missed",
  paused: "paused",
  stillRunning: "still running",
  tooMany: "too many runs",
  folderMissing: "folder missing",
  startFailed: "couldn't start",
};

export type ResultTone = "running" | "done" | "failed" | "skipped" | "none";

/** The last thing that happened: its run or a skip, whichever is newer ("Done 2 h ago · $0.02"). */
export function lastLine(routine: RoutineView): { text: string; tone: ResultTone } {
  if (routine.runningRunId) return { text: "Running", tone: "running" };
  const { lastRun, lastSkip } = routine;
  if (lastSkip && (!lastRun || lastSkip.atMs > lastRun.atMs)) {
    const bad = lastSkip.why === "folderMissing" || lastSkip.why === "startFailed";
    return { text: `Skipped ${dayTime(lastSkip.dueMs)}: ${SKIP_LABEL[lastSkip.why]}`, tone: bad ? "failed" : "skipped" };
  }
  if (lastRun) {
    const label = lastRun.status ? endLabel(lastRun.status, lastRun.reason) : "Ran";
    const failed = lastRun.status === "failed";
    return {
      text: [`${label} ${ago(lastRun.atMs)}`, costLabel(lastRun.costUsd)].filter(Boolean).join(" · "),
      tone: failed ? "failed" : "done",
    };
  }
  return { text: "Not run yet", tone: "none" };
}

export function costLabel(usd: number | null): string | null {
  if (usd == null) return null;
  return usd > 0 && usd < 0.01 ? "<$0.01" : `$${usd.toFixed(2)}`;
}

/** A short default name from a prompt: its first few words ("Summarize yesterday's commits"). */
export function nameFromPrompt(prompt: string): string {
  const line = prompt.trim().split(/\r?\n/)[0] ?? "";
  const words = line.split(/\s+/).filter(Boolean);
  let name = "";
  for (const w of words) {
    const next = name ? `${name} ${w}` : w;
    if (next.length > 32 && name) break;
    name = next;
    if (name.split(" ").length >= 5) break;
  }
  name = name.replace(/[.,;:!?…]+$/, "").slice(0, 40);
  return name ? name[0].toUpperCase() + name.slice(1) : "";
}
