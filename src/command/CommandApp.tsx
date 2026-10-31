import { useCallback, useEffect, useRef, useState } from "react";
import { commands, on, unlistenAll } from "../lib/ipc";
import type { AgentOptions, AgentRun, RoutineDraft, RoutineView } from "../lib/types";
import { ClockGlyph, RunDot } from "../features/agent/AgentGlyphs";
import { ago, endLabel } from "../features/agent/labels";
import { isLive, useAgentRuns } from "../features/agent/useAgentRuns";
import { ClaudeMark, ClaudeSpinner } from "../features/claude/ClaudeGlyphs";
import { NewTask } from "./NewTask";
import { RoutineEditor } from "./RoutineEditor";
import { RoutineList } from "./RoutineList";
import { nameFromPrompt, WORKDAYS, type RoutineSeed } from "./routineLabels";
import { RunView } from "./RunView";
import { useRoutines } from "./useRoutines";
import { useTranscripts } from "./useTranscripts";

/** The routine editor's form: `key` remounts it for each routine opened; `id` follows a new one once saved. */
interface Editing {
  key: number;
  id: string | null;
  initial: RoutineDraft;
}

function routineDraft(r: RoutineView): RoutineDraft {
  return {
    id: r.id,
    name: r.name,
    prompt: r.prompt,
    cwd: r.cwd,
    model: r.model,
    maxTurns: r.maxTurns,
    maxBudgetUsd: r.maxBudgetUsd,
    claudePath: r.claudePath,
    schedule: r.schedule,
    enabled: r.enabled,
    ifMissed: r.ifMissed,
    waitForMe: r.waitForMe,
    desktopTools: r.desktopTools,
  };
}

/** A new routine: what the task knew, then the defaults (weekdays at 09:00, a budget, desktop tools off). */
function newDraft(seed: RoutineSeed, options: AgentOptions): RoutineDraft {
  const prompt = seed.prompt ?? "";
  return {
    id: null,
    name: seed.name ?? nameFromPrompt(prompt),
    prompt,
    cwd: seed.cwd ?? options.recentFolders[0] ?? "",
    model: seed.model ?? options.model,
    maxTurns: seed.maxTurns ?? options.maxTurns,
    maxBudgetUsd: seed.maxBudgetUsd ?? options.maxBudgetUsd ?? 1,
    claudePath: seed.claudePath ?? options.defaultInstall,
    schedule: seed.schedule ?? { kind: "days", time: "09:00", days: WORKDAYS },
    enabled: seed.enabled ?? true,
    ifMissed: seed.ifMissed ?? "once",
    waitForMe: seed.waitForMe ?? true,
    desktopTools: seed.desktopTools ?? false,
  };
}

/**
 * The command window: a normal, focusable window (the island never takes keyboard input). New-task form or one
 * run's transcript, with the runs listed on the left. Esc hides it (Rust keeps it alive for next time).
 */
export function CommandApp() {
  const runs = useAgentRuns();
  const transcripts = useTranscripts();
  const routines = useRoutines();
  /** Set while the routine editor shows (it takes the main area over a run or the new-task form). */
  const [editing, setEditing] = useState<Editing | null>(null);
  const [options, setOptions] = useState<AgentOptions | null>(null);
  /** `null`: the new-task form. */
  const [selected, setSelected] = useState<string | null>(null);
  /** Bumped when the window gains focus, so the visible view focuses its input. */
  const [focusTick, setFocusTick] = useState(0);
  /** A native folder picker is open: Esc belongs to it. */
  const picking = useRef(false);

  const refreshOptions = useCallback(() => {
    commands
      .agentOptions()
      .then(setOptions)
      .catch((err: unknown) => console.debug("agent options", err));
  }, []);

  useEffect(() => {
    refreshOptions();
    const subscriptions = [
      on.commandFocusRun((runId) => {
        setEditing(null);
        setSelected(runId);
      }),
    ];
    const onFocus = () => setFocusTick((n) => n + 1);
    const onVisible = () => document.visibilityState === "visible" && onFocus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented || picking.current) return;
      e.preventDefault();
      void commands.commandWindowHide().catch(() => {});
    };
    window.addEventListener("focus", onFocus);
    document.addEventListener("visibilitychange", onVisible);
    document.addEventListener("keydown", onKey);
    return () => {
      unlistenAll(subscriptions);
      window.removeEventListener("focus", onFocus);
      document.removeEventListener("visibilitychange", onVisible);
      document.removeEventListener("keydown", onKey);
    };
  }, [refreshOptions]);

  useEffect(() => {
    if (selected) transcripts.load(selected);
  }, [selected, transcripts]);

  const run = selected ? runs.find((r) => r.runId === selected) : undefined;

  const showRun = (runId: string) => {
    setEditing(null);
    setSelected(runId);
  };
  const editRoutine = (initial: RoutineDraft) => setEditing((e) => ({ key: (e?.key ?? 0) + 1, id: initial.id, initial }));
  const newRoutine = (seed: RoutineSeed = {}) => options && editRoutine(newDraft(seed, options));

  return (
    <div className="cmd">
      <header className="cmd__bar" data-tauri-drag-region>
        <ClaudeMark />
        <span className="cmd__title" data-tauri-drag-region>
          Claude agent
        </span>
        <span className="cmd__spacer" data-tauri-drag-region />
        {options?.hotkeyError ? (
          <span className="cmd__hotkey cmd__hotkey--error" title={options.hotkeyError} data-tauri-drag-region>
            Hotkey unavailable
          </span>
        ) : (
          options?.hotkey && (
            <kbd className="cmd__hotkey" title="Opens this window from anywhere" data-tauri-drag-region>
              {options.hotkey}
            </kbd>
          )
        )}
        <button
          className="cmd__close"
          aria-label="Close"
          title="Close (Esc)"
          onClick={() => void commands.commandWindowHide().catch(() => {})}
        >
          <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.4} strokeLinecap="round">
            <path d="M4 4l8 8M12 4l-8 8" />
          </svg>
        </button>
      </header>
      <div className="cmd__body">
        <nav className="cmd__runs" aria-label="Runs">
          <button
            className="cmd-new"
            aria-pressed={selected === null && editing === null}
            onClick={() => {
              setEditing(null);
              setSelected(null);
            }}
          >
            <span className="cmd-new__plus" aria-hidden="true">
              +
            </span>
            New task
          </button>
          <RoutineList
            routines={routines}
            selectedId={editing?.id ?? null}
            onSelect={(r) => (editing?.id === r.id ? undefined : editRoutine(routineDraft(r)))}
            onNew={() => newRoutine()}
            onStarted={showRun}
          />
          <ul className="cmd-runs">
            {runs.map((r) => (
              <li key={r.runId}>
                <RunItem run={r} selected={editing === null && r.runId === selected} onSelect={() => showRun(r.runId)} />
              </li>
            ))}
          </ul>
        </nav>
        <main className="cmd__main">
          {/* Kept mounted (hidden) so a draft survives looking at a run. */}
          <div className="cmd__view" hidden={selected !== null || editing !== null}>
            {options && (
              <NewTask
                options={options}
                focusTick={selected === null ? focusTick : -1}
                picking={picking}
                onStarted={(runId) => {
                  setSelected(runId);
                  refreshOptions();
                }}
                onSaveAsRoutine={newRoutine}
              />
            )}
          </div>
          {selected && editing === null && (
            <div className="cmd__view">
              <RunView
                key={selected}
                runId={selected}
                run={run}
                transcript={transcripts.get(selected)}
                focusTick={focusTick}
                onSaveAsRoutine={newRoutine}
              />
            </div>
          )}
          {editing && options && (
            <div className="cmd__view">
              <RoutineEditor
                key={editing.key}
                options={options}
                initial={editing.initial}
                view={routines.routines.find((r) => r.id === editing.id)}
                paused={routines.paused}
                picking={picking}
                onSaved={(id) => setEditing((e) => e && { ...e, id })}
                onDeleted={() => setEditing(null)}
                onClose={() => setEditing(null)}
                onShowRun={showRun}
              />
            </div>
          )}
        </main>
      </div>
    </div>
  );
}

function RunItem({ run, selected, onSelect }: { run: AgentRun; selected: boolean; onSelect: () => void }) {
  const status =
    run.status === "waiting"
      ? "Needs approval"
      : isLive(run.status)
        ? "Working"
        : endLabel(run.status, run.reason);
  return (
    <button className="cmd-run" data-status={run.status} aria-pressed={selected} onClick={onSelect} title={run.cwd}>
      <span className="cmd-run__line">
        <span className="cmd-run__mark">
          {run.status === "working" || run.status === "starting" ? <ClaudeSpinner /> : <RunDot status={run.status} />}
        </span>
        <span className="cmd-run__project">
          {run.routine?.trigger === "schedule" && <ClockGlyph />}
          {run.routine?.name ?? run.project}
        </span>
        <span className="cmd-run__time">{ago(run.updatedAtMs)}</span>
      </span>
      <span className="cmd-run__detail">
        <span className="cmd-run__status">{status}</span>
        <span className="cmd-run__prompt">{run.prompt}</span>
      </span>
    </button>
  );
}
