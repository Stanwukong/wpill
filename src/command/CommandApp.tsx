import { useCallback, useEffect, useRef, useState } from "react";
import { commands, on, unlistenAll } from "../lib/ipc";
import type { AgentOptions, AgentRun, ClaudeApprovalActivity, RoutineDraft, RoutineView } from "../lib/types";
import { ClockGlyph, RunDot } from "../features/agent/AgentGlyphs";
import { ago, endLabel } from "../features/agent/labels";
import { isLive, useAgentRuns } from "../features/agent/useAgentRuns";
import { AgentMark, WorkingIndicator } from "../features/claude/WorkingGlyphs";
import { NewTask } from "./NewTask";
import { RoutineEditor } from "./RoutineEditor";
import { RoutineList } from "./RoutineList";
import { nameFromPrompt, WORKDAYS, type RoutineSeed } from "./routineLabels";
import { RunView } from "./RunView";
import { SessionAskView } from "./SessionAskView";
import { useClaudeAsks } from "./useClaudeAsks";
import { useRoutines } from "./useRoutines";
import { useTranscripts } from "./useTranscripts";

/** What the main area shows besides the new-task form (`null`) and the routine editor. */
type Selection = { kind: "run"; runId: string } | { kind: "ask"; requestId: number };

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
    mode: r.mode,
  };
}

/** A new routine: what the task knew, then the defaults (weekdays at 09:00, a budget, desktop tools off, the default mode). */
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
    mode: seed.mode ?? options.mode,
  };
}

/**
 * The command window: a normal, focusable window (the island never takes keyboard input). New-task form, one
 * run's transcript or a terminal session's held request (2.8b), with those listed on the left. Esc hides it (Rust keeps it alive for next time).
 */
export function CommandApp() {
  const runs = useAgentRuns();
  const transcripts = useTranscripts();
  const routines = useRoutines();
  const asks = useClaudeAsks();
  /** Set while the routine editor shows (it takes the main area over a run, an ask or the new-task form). */
  const [editing, setEditing] = useState<Editing | null>(null);
  const [options, setOptions] = useState<AgentOptions | null>(null);
  /** `null`: the new-task form. */
  const [selected, setSelected] = useState<Selection | null>(null);
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
    const select = (s: Selection) => {
      setEditing(null);
      setSelected(s);
    };
    const subscriptions = [
      on.commandFocusRun((runId) => select({ kind: "run", runId })),
      on.commandFocusAsk((requestId) => select({ kind: "ask", requestId })),
    ];
    // A window created for a run or an ask missed that focus event; Rust hands it over once. No `alive` guard:
    // under StrictMode's double mount only the first call gets it.
    commands
      .commandTakeFocus()
      .then((focus) => focus && select(focus))
      .catch((err: unknown) => console.debug("command focus", err));
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

  const runId = selected?.kind === "run" ? selected.runId : null;
  const askId = selected?.kind === "ask" ? selected.requestId : null;

  useEffect(() => {
    if (runId) transcripts.load(runId);
  }, [runId, transcripts]);

  const run = runId ? runs.find((r) => r.runId === runId) : undefined;

  const showRun = (runId: string) => {
    setEditing(null);
    setSelected({ kind: "run", runId });
  };
  const showAsk = (requestId: number) => {
    setEditing(null);
    setSelected({ kind: "ask", requestId });
  };
  const editRoutine = (initial: RoutineDraft) => setEditing((e) => ({ key: (e?.key ?? 0) + 1, id: initial.id, initial }));
  const newRoutine = (seed: RoutineSeed = {}) => options && editRoutine(newDraft(seed, options));

  return (
    <div className="cmd">
      <header className="cmd__bar" data-tauri-drag-region>
        <AgentMark />
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
          {asks.asks.length > 0 && (
            <section className="cmd-asks" aria-label="Waiting in sessions">
              <div className="cmd-routines__head">
                <span className="cmd-routines__label">Waiting in sessions</span>
              </div>
              <ul className="cmd-asks__list">
                {asks.asks.map((a) => (
                  <li key={a.requestId}>
                    <AskItem
                      ask={a}
                      selected={editing === null && a.requestId === askId}
                      onSelect={() => showAsk(a.requestId)}
                    />
                  </li>
                ))}
              </ul>
            </section>
          )}
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
                <RunItem run={r} selected={editing === null && r.runId === runId} onSelect={() => showRun(r.runId)} />
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
                  setSelected({ kind: "run", runId });
                  refreshOptions();
                }}
                onSaveAsRoutine={newRoutine}
              />
            )}
          </div>
          {runId && editing === null && (
            <div className="cmd__view">
              <RunView
                key={runId}
                runId={runId}
                run={run}
                transcript={transcripts.get(runId)}
                focusTick={focusTick}
                onSaveAsRoutine={newRoutine}
              />
            </div>
          )}
          {askId !== null && editing === null && (
            <div className="cmd__view">
              <SessionAskView
                key={askId}
                ask={asks.asks.find((a) => a.requestId === askId)}
                loaded={asks.loaded}
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

/** A session's held request: a breathing amber dot (it waits on you), the project, and the tool. */
function AskItem({ ask, selected, onSelect }: { ask: ClaudeApprovalActivity; selected: boolean; onSelect: () => void }) {
  return (
    <button className="cmd-run" data-status="waiting" aria-pressed={selected} onClick={onSelect} title={ask.summary}>
      <span className="cmd-run__line">
        <span className="cmd-run__mark">
          <RunDot status="waiting" />
        </span>
        <span className="cmd-run__project">{ask.project}</span>
        <span className="cmd-ask__tool">{ask.tool}</span>
      </span>
    </button>
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
          {run.status === "working" || run.status === "starting" ? <WorkingIndicator small /> : <RunDot status={run.status} />}
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
