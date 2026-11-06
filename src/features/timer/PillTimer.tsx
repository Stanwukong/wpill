import type { TimerState } from "../../lib/types";
import { StopwatchGlyph, TimerRing } from "./TimerGlyphs";
import { remainingFraction, timerText } from "./timerFormat";
import "./timer.css";

/**
 * The pill's timer (5a): the ring (or the stopwatch mark) and the time, right of the clock. The mark beside the digits
 * keeps `05:00` from reading as a second clock. Paused: dimmed and still. Island shows it only while one runs or is
 * paused (`showsInPill`).
 */
export function PillTimer({ timer }: { timer: TimerState }) {
  const fraction = remainingFraction(timer);
  return (
    <span className="pill-timer" data-phase={timer.phase} aria-label={`Timer ${timerText(timer)}`}>
      {fraction === null ? <StopwatchGlyph size={15} /> : <TimerRing fraction={fraction} />}
      <span className="pill-timer__time">{timerText(timer)}</span>
    </span>
  );
}
