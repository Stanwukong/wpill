import { useEffect, useState } from "react";
import { commands, errorMessage } from "../lib/ipc";
import type { Routines, RoutineView } from "../lib/types";
import { ClockGlyph } from "../features/agent/AgentGlyphs";
import { ClaudeSpinner } from "../features/claude/ClaudeGlyphs";
import { lastLine, scheduleLine } from "./routineLabels";
import { folderName } from "./TaskFields";

/**
 * The sidebar's Routines: a header with Pause/Resume and +, then one row per routine (name; schedule and next run;
 * last result and folder) with a Run button. Selecting a row opens its editor.
 */
export function RoutineList({
  routines: { routines, paused },
  selectedId,
  onSelect,
  onNew,
  onStarted,
}: {
  routines: Routines;
  selectedId: string | null;
  onSelect: (routine: RoutineView) => void;
  onNew: () => void;
  onStarted: (runId: string) => void;
}) {
  // A failed Run click, shown on its row until the next change from Rust.
  const [runError, setRunError] = useState<{ id: string; message: string } | null>(null);
  useEffect(() => setRunError(null), [routines]);

  const run = (id: string) => {
    setRunError(null);
    commands
      .routineRun(id)
      .then(({ runId }) => onStarted(runId))
      .catch((err: unknown) => setRunError({ id, message: errorMessage(err) }));
  };

  return (
    <section className="cmd-routines" aria-label="Routines">
      <div className="cmd-routines__head">
        <span className="cmd-routines__label">Routines</span>
        {paused && <span className="cmd-routines__paused">Paused</span>}
        <span className="cmd__spacer" />
        {(paused || routines.length > 0) && (
          <button
            className="cmd-routines__pause"
            data-paused={paused}
            title={paused ? "Let schedules run again" : "Hold every schedule (Run still works)"}
            onClick={() => void commands.routinesPause(!paused).catch(() => {})}
          >
            {paused ? "Resume" : "Pause"}
          </button>
        )}
        <button className="cmd-routines__add" aria-label="New routine" title="New routine" onClick={onNew}>
          <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.6} strokeLinecap="round">
            <path d="M8 3.5v9M3.5 8h9" />
          </svg>
        </button>
      </div>
      {routines.length === 0 ? (
        <button className="cmd-routines__empty" onClick={onNew}>
          Save a task to run on a schedule
        </button>
      ) : (
        <ul className="cmd-routines__list">
          {routines.map((r) => (
            <li key={r.id}>
              <RoutineItem
                routine={r}
                paused={paused}
                selected={r.id === selectedId}
                error={runError?.id === r.id ? runError.message : null}
                onSelect={() => onSelect(r)}
                onRun={() => run(r.id)}
              />
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function RoutineItem({
  routine,
  paused,
  selected,
  error,
  onSelect,
  onRun,
}: {
  routine: RoutineView;
  paused: boolean;
  selected: boolean;
  error: string | null;
  onSelect: () => void;
  onRun: () => void;
}) {
  const running = routine.runningRunId !== null;
  const last = error ? { text: error, tone: "failed" as const } : lastLine(routine);
  const schedule = scheduleLine(routine, paused);
  return (
    <div className="cmd-routine" data-scheduled={routine.nextRunAtMs !== null}>
      <button className="cmd-run cmd-routine__body" aria-pressed={selected} onClick={onSelect} title={routine.cwd}>
        <span className="cmd-run__line">
          <span className="cmd-run__mark cmd-routine__mark">{running ? <ClaudeSpinner /> : <ClockGlyph />}</span>
          <span className="cmd-run__project">{routine.name}</span>
        </span>
        <span className="cmd-run__detail" title={schedule}>
          <span className="cmd-routine__when">{schedule}</span>
        </span>
        <span className="cmd-run__detail">
          <span className="cmd-routine__last" data-tone={last.tone} title={last.text}>
            {last.text}
          </span>
          <span className="cmd-run__prompt">{folderName(routine.cwd)}</span>
        </span>
      </button>
      <button
        className="cmd-routine__run"
        aria-label={`Run ${routine.name} now`}
        title={running ? "Running" : "Run now"}
        disabled={running}
        onClick={onRun}
      >
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path d="M5.5 3.9v8.2a.6.6 0 0 0 .9.5l6.4-4.1a.6.6 0 0 0 0-1L6.4 3.4a.6.6 0 0 0-.9.5z" fill="currentColor" />
        </svg>
      </button>
    </div>
  );
}
