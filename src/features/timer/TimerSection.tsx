import { useState } from "react";
import { commands } from "../../lib/ipc";
import type { TimerPayload, TimerState } from "../../lib/types";
import { StopwatchGlyph, TimerRing } from "./TimerGlyphs";
import { doneTitle, endedLabel, isCountdown, kindLabel, remainingFraction, timerText } from "./timerFormat";
import { timerAction } from "./useTimer";
import "./timer.css";

/** The presets (D56): clicks only, no custom time in the island (typing would need focus). */
const PRESETS: { label: string; start: () => Promise<TimerPayload> }[] = [
  { label: "5 min", start: () => commands.timerStart("countdown", 5) },
  { label: "10 min", start: () => commands.timerStart("countdown", 10) },
  { label: "25 min focus", start: () => commands.timerStart("focus") },
  { label: "Stopwatch", start: () => commands.timerStart("stopwatch") },
];

/**
 * Panel section (5a): the preset chips while nothing runs; otherwise one row with the mark, the time, what it is and
 * its controls (Pause / Resume, +1 min for countdowns, Cancel; a quiet Do not disturb link during Focus). An ended
 * timer offers what its activity does. Rendered from Rust's latest `timer:state`.
 */
export function TimerSection({ timer }: { timer: TimerState | null }) {
  const [error, setError] = useState<string | null>(null);
  const run = (call: () => Promise<TimerPayload>) => void timerAction(call).then(setError);

  return (
    <section className="timer" aria-label="Timer">
      <h2 className="timer__heading">Timer</h2>
      {timer === null ? (
        <div className="timer__presets">
          {PRESETS.map((p) => (
            <button key={p.label} className="timer-btn" onClick={() => run(p.start)}>
              {p.label}
            </button>
          ))}
        </div>
      ) : (
        <TimerRow timer={timer} run={run} onError={setError} />
      )}
      {error && <p className="timer__error">{error}</p>}
    </section>
  );
}

function TimerRow({
  timer,
  run,
  onError,
}: {
  timer: TimerState;
  run: (call: () => Promise<TimerPayload>) => void;
  onError: (message: string | null) => void;
}) {
  const fraction = remainingFraction(timer);
  const ended = timer.phase === "ended";
  const label = ended
    ? timer.endedAtMs !== null
      ? `${doneTitle(timer.kind)} · ${endedLabel(timer.endedAtMs)}`
      : doneTitle(timer.kind)
    : timer.phase === "paused"
      ? `${kindLabel(timer.kind)} · Paused`
      : kindLabel(timer.kind);

  return (
    <div className="timer-row" data-phase={timer.phase}>
      <span className="timer-row__mark">
        {fraction === null ? <StopwatchGlyph size={16} /> : <TimerRing fraction={fraction} />}
      </span>
      <span className="timer-row__time">{timerText(timer)}</span>
      <span className="timer-row__label">{label}</span>
      {timer.kind === "focus" && !ended && (
        <button
          className="timer-link"
          onClick={() =>
            void commands.timerOpenDnd().then(
              () => onError(null),
              () => onError("Couldn't open Do not disturb"),
            )
          }
        >
          Do not disturb
        </button>
      )}
      {ended ? (
        timer.kind === "focus" ? (
          <button className="timer-btn" onClick={() => run(() => commands.timerStart("break"))}>
            Break 5 min
          </button>
        ) : (
          isCountdown(timer.kind) && (
            <button className="timer-btn" onClick={() => run(commands.timerAddMinute)}>
              +1 min
            </button>
          )
        )
      ) : (
        <>
          {timer.phase === "paused" ? (
            <button className="timer-btn" onClick={() => run(commands.timerResume)}>
              Resume
            </button>
          ) : (
            <button className="timer-btn" onClick={() => run(commands.timerPause)}>
              Pause
            </button>
          )}
          {isCountdown(timer.kind) && (
            <button className="timer-btn" onClick={() => run(commands.timerAddMinute)}>
              +1 min
            </button>
          )}
        </>
      )}
      <button className="timer-link" onClick={() => run(commands.timerCancel)}>
        {ended ? "Dismiss" : "Cancel"}
      </button>
    </div>
  );
}
