import { commands } from "../../lib/ipc";
import type { TimerDoneActivity, TimerPayload } from "../../lib/types";
import { StopwatchGlyph } from "./TimerGlyphs";
import { doneTitle, endedLabel } from "./timerFormat";
import { timerAction } from "./useTimer";
import "./timer.css";

/**
 * The `timerDone` activity (5a): the mark, "Timer done" / "Focus done" / "Break over" (with "Ended at 14:05" under it
 * when Cay noticed late), then two quiet buttons: Break 5 min after Focus, else +1 min; and Dismiss. Interactive while
 * hovered (Rust); every button makes Rust retract it.
 */
export function TimerDone({ done }: { done: TimerDoneActivity }) {
  const run = (call: () => Promise<TimerPayload>) => void timerAction(call);
  return (
    <div className="activity timer-done">
      <StopwatchGlyph size={22} className="activity__icon" />
      <div className="timer-done__text">
        <span className="activity__title">{doneTitle(done.kind)}</span>
        {done.late && <span className="timer-done__late">{endedLabel(done.endedAtMs)}</span>}
      </div>
      {done.kind === "focus" ? (
        <button className="timer-btn" onClick={() => run(() => commands.timerStart("break"))}>
          Break 5 min
        </button>
      ) : (
        <button className="timer-btn" onClick={() => run(commands.timerAddMinute)}>
          +1 min
        </button>
      )}
      <button className="timer-btn" onClick={() => run(commands.timerCancel)}>
        Dismiss
      </button>
    </div>
  );
}
