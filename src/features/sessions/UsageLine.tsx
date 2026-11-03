import type { LimitWindow, UsageLimits } from "../../lib/types";

/** The bar turns amber from here, red from `DANGER_AT` (percent used). */
const WARN_AT = 80;
const DANGER_AT = 95;

// The user's locale decides 24 h vs 12 h. Built once; formatting happens at render (nothing ticks).
const TIME = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });
const DAY_TIME = new Intl.DateTimeFormat(undefined, { weekday: "short", hour: "numeric", minute: "2-digit" });

type Level = "ok" | "warn" | "danger";

function level(percent: number): Level {
  return percent >= DANGER_AT ? "danger" : percent >= WARN_AT ? "warn" : "ok";
}

function Limit({ label, window, resets }: { label: string; window: LimitWindow; resets: string }) {
  if (window.stale) {
    return (
      <span className="usage__item" data-stale="true">
        <span className="usage__label">{label}</span>
        <span className="usage__reset">reset · no new reading yet</span>
      </span>
    );
  }
  const percent = Math.round(Math.min(100, Math.max(0, window.usedPercent)));
  return (
    <span className="usage__item" data-level={level(percent)}>
      <span className="usage__label">{label}</span>
      <span
        className="usage__meter"
        role="meter"
        aria-label={`${label} limit used`}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent}
      >
        <span className="usage__fill" style={{ transform: `scaleX(${percent / 100})` }} />
      </span>
      <span className="usage__percent">{percent}%</span>
      <span className="usage__reset">resets {resets}</span>
    </span>
  );
}

function describe(name: string, w: LimitWindow, resets: string): string {
  return w.stale
    ? `${name}: reset at ${resets}, no newer reading yet`
    : `${name}: ${Math.round(w.usedPercent)}% used, resets ${resets}`;
}

/**
 * Claude's usage limits (2.8f) beside the section heading: the 5-hour and 7-day windows, each a percentage on a thin
 * meter and its reset as a local time. A window whose reset has passed reads dimmed until a new reading arrives.
 */
export function UsageLine({ limits }: { limits: UsageLimits }) {
  const { fiveHour, sevenDay } = limits;
  const five = fiveHour && TIME.format(fiveHour.resetsAtMs);
  const seven = sevenDay && DAY_TIME.format(sevenDay.resetsAtMs);
  const title = [
    `Usage limits from Claude Code's status line, read ${DAY_TIME.format(limits.atMs)}`,
    fiveHour && five && describe("5-hour window", fiveHour, five),
    sevenDay && seven && describe("7-day window", sevenDay, seven),
  ]
    .filter(Boolean)
    .join("\n");

  return (
    <div className="usage" title={title}>
      {fiveHour && five && <Limit label="5h" window={fiveHour} resets={five} />}
      {sevenDay && seven && <Limit label="7d" window={sevenDay} resets={seven} />}
    </div>
  );
}

/** Whether there's anything to show; with no sessions, only a live (not stale) reading earns the section. */
export function hasLimits(limits: UsageLimits | null, liveOnly = false): limits is UsageLimits {
  const windows = [limits?.fiveHour, limits?.sevenDay].filter((w): w is LimitWindow => !!w);
  return liveOnly ? windows.some((w) => !w.stale) : windows.length > 0;
}
