import { useCallback, useEffect, useRef, useState } from "react";
import { commands, on, unlistenAll } from "../lib/ipc";
import type { AgentBackendsPayload, AgentOptions, AgentRun, SessionApprovalActivity, RoutineDraft, RoutineView } from "../lib/types";
import { ClockGlyph, RunDot } from "../features/agent/AgentGlyphs";
import { ago, endLabel } from "../features/agent/labels";
import { isLive, useAgentRuns } from "../features/agent/useAgentRuns";
import { AgentTag, mixedAgents } from "../features/sessions/AgentTag";
import { AgentMark, WorkingIndicator } from "../features/sessions/WorkingGlyphs";
import { DropOverlay, type DropInto } from "./Attachments";
import { NewTask } from "./NewTask";
import { RoutineEditor } from "./RoutineEditor";
import { RoutineList } from "./RoutineList";
import { nameFromPrompt, WORKDAYS, type RoutineSeed } from "./routineLabels";
import { runGroups } from "./runList";
import { RunView } from "./RunView";
import { SessionAskView } from "./SessionAskView";
import { useSessionAsks } from "./useSessionAsks";
import { useRoutines } from "./useRoutines";
import { useTranscripts } from "./useTranscripts";

/** What the main area shows besides the new-task form (`null`) and the routine editor. */
type Selection = { kind: "run"; runId: string } | { kind: "ask"; requestId: number };

/** The routine editor's form: `key` remounts it for each routine opened; `id` follows a new one once saved. */
interface Editing {
  key: number;
  id: string | null;
  initial: RoutineDraft;
  /** New, with no agent chosen: it follows Rust's default agent. */
  followDefault: boolean;
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
    googleTools: r.googleTools,
    mode: r.mode,
    backend: r.backend,
  };
}

/** A new routine: what the task knew, then the defaults (weekdays at 09:00, a budget, desktop tools off, the default mode). */
function newDraft(seed: RoutineSeed, options: AgentOptions, backends: AgentBackendsPayload | null): RoutineDraft {
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
    googleTools: seed.googleTools ?? false,
    mode: seed.mode ?? options.mode,
    backend: seed.backend ?? backends?.defaultBackend ?? "claude",
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
  const asks = useSessionAsks();
  /** Set while the routine editor shows (it takes the main area over a run, an ask or the new-task form). */
  const [editing, setEditing] = useState<Editing | null>(null);
  const [options, setOptions] = useState<AgentOptions | null>(null);
  /** The agents the forms offer (2.9c); the first answer can take a few seconds, so nothing waits for it. */
  const [backends, setBackends] = useState<AgentBackendsPayload | null>(null);
  /** `null`: the new-task form. */
  const [selected, setSelected] = useState<Selection | null>(null);
  /** Bumped when the window gains focus, so the visible view focuses its input. */
  const [focusTick, setFocusTick] = useState(0);
  /** A native folder picker is open: Esc belongs to it. */
  const picking = useRef(false);
  /** The visible composer's drop handler (2.10b.4); `null` while neither the new-task form nor a run shows. */
  const dropInto: DropInto = useRef(null);
  /** Files hover the window and a composer would take them. */
  const [dragging, setDragging] = useState(false);
  /** Native maximize (2.10c): Rust reports every change, including double-clicks on the bar and Snap. */
  const [maximized, setMaximized] = useState(false);
  /** The run list's filter (2.10d); the selected run stays selected while filtered out of view. */
  const [runQuery, setRunQuery] = useState("");
  /** The filter box, for Ctrl+F (2.10d.3). */
  const runFilter = useRef<HTMLInputElement>(null);

  const refreshOptions = useCallback(() => {
    commands
      .agentOptions()
      .then(setOptions)
      .catch((err: unknown) => console.debug("agent options", err));
    commands
      .agentBackends()
      .then(setBackends)
      .catch((err: unknown) => console.debug("agent backends", err));
  }, []);

  const toggleMaximize = useCallback(() => {
    commands
      .commandWindowToggleMaximize()
      .then(setMaximized)
      .catch((err: unknown) => console.debug("command maximize", err));
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
      on.commandMaximized(setMaximized),
      on.filesDrag((drag) => {
        if (drag.type === "over") setDragging(dropInto.current !== null);
        else {
          setDragging(false);
          if (drag.type === "drop") dropInto.current?.(drag.paths);
        }
      }),
    ];
    // A window created for a run or an ask missed that focus event; Rust hands it over once. No `alive` guard:
    // under StrictMode's double mount only the first call gets it.
    commands
      .commandTakeFocus()
      .then((focus) => focus && select(focus))
      .catch((err: unknown) => console.debug("command focus", err));
    commands
      .commandWindowMaximized()
      .then(setMaximized)
      .catch((err: unknown) => console.debug("command maximized", err));
    const onFocus = () => setFocusTick((n) => n + 1);
    const onVisible = () => document.visibilityState === "visible" && onFocus();
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || picking.current) return;
      if (e.key === "Escape") {
        e.preventDefault();
        void commands.commandWindowEscape().catch(() => {});
      } else if (e.key === "F11") {
        e.preventDefault();
        toggleMaximize();
      }
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
  }, [refreshOptions, toggleMaximize]);

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
  const ended = runs.filter((r) => !isLive(r.status));
  /** Rust drops the run and sends `agent:runs`; the list follows that, never ahead of it. */
  const remove = (id: string) => {
    if (id === runId) setSelected(null);
    commands.agentRemove(id).catch((err: unknown) => console.debug("agent remove", err));
  };
  const editRoutine = (initial: RoutineDraft, followDefault = false) =>
    setEditing((e) => ({ key: (e?.key ?? 0) + 1, id: initial.id, initial, followDefault }));
  const newRoutine = (seed: RoutineSeed = {}) =>
    options && editRoutine(newDraft(seed, options, backends), seed.backend == null);
  /** Any run on another agent: then every row names its agent (2.9c, as sessions do). */
  const runsMixed = runs.some((r) => r.backend !== "claude");
  /** Worked out per render, like `ago`: nothing ticks to move a run from Today to Earlier. */
  const groups = runGroups(runs, runQuery, Date.now());

  return (
    <div className="cmd" data-maximized={maximized || undefined}>
      <header className="cmd__bar" data-tauri-drag-region>
        <AgentMark />
        <span className="cmd__title" data-tauri-drag-region>
          Cay
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
        <div className="cmd__btns">
          <button
            className="cmd__btn"
            aria-label={maximized ? "Exit fullscreen (F11)" : "Fullscreen (F11)"}
            title={maximized ? "Exit fullscreen (F11)" : "Fullscreen (F11)"}
            onClick={toggleMaximize}
          >
            <svg
              viewBox="0 0 16 16"
              fill="none"
              stroke="currentColor"
              strokeWidth={1.4}
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              {maximized ? (
                <path d="M4 6.5h5.5V12H4zM6.5 4H12v5.5" />
              ) : (
                <rect x="4" y="4" width="8" height="8" rx="0.5" />
              )}
            </svg>
          </button>
          <button
            className="cmd__btn"
            aria-label="Close"
            title="Close (Esc)"
            onClick={() => void commands.commandWindowHide().catch(() => {})}
          >
            <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.4} strokeLinecap="round">
              <path d="M4 4l8 8M12 4l-8 8" />
            </svg>
          </button>
        </div>
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
            <svg className="cmd-new__plus" viewBox="0 0 16 16" aria-hidden="true">
              <path d="M8 3.5v9M3.5 8h9" fill="none" stroke="currentColor" strokeWidth={1.6} strokeLinecap="round" />
            </svg>
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
          {runs.length > 0 && (
            <section className="cmd-runs" aria-label="Runs">
              <div className="cmd-routines__head">
                <span className="cmd-routines__label">Runs</span>
                <span className="cmd__spacer" />
                {ended.length > 0 && (
                  <button
                    className="cmd-routines__pause"
                    title="Remove every finished run from this list"
                    onClick={() => ended.forEach((r) => remove(r.runId))}
                  >
                    Clear ended
                  </button>
                )}
              </div>
              <div className="cmd-filter">
                <svg className="cmd-filter__glyph" viewBox="0 0 16 16" aria-hidden="true">
                  <circle cx="7" cy="7" r="4.25" fill="none" stroke="currentColor" strokeWidth={1.5} />
                  <path d="M10.25 10.25L13 13" stroke="currentColor" strokeWidth={1.5} strokeLinecap="round" />
                </svg>
                <input
                  ref={runFilter}
                  className="cmd-filter__input"
                  type="text"
                  placeholder="Filter runs"
                  aria-label="Filter runs"
                  spellCheck={false}
                  value={runQuery}
                  onChange={(e) => setRunQuery(e.target.value)}
                  onKeyDown={(e) => {
                    // Esc clears the text first; only an empty box lets it hide the window.
                    if (e.key === "Escape" && runQuery !== "") {
                      e.preventDefault();
                      setRunQuery("");
                    }
                  }}
                />
                {runQuery !== "" && (
                  <button
                    className="cmd-filter__clear"
                    aria-label="Clear filter"
                    title="Clear (Esc)"
                    onClick={() => {
                      setRunQuery("");
                      runFilter.current?.focus();
                    }}
                  >
                    <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.5} strokeLinecap="round" aria-hidden="true">
                      <path d="M5 5l6 6M11 5l-6 6" />
                    </svg>
                  </button>
                )}
              </div>
              <ul className="cmd-runs__list">
                {groups.map((g) => (
                  <li key={g.label} className="cmd-runs__group">
                    <span className="cmd-routines__label">{g.label}</span>
                    <ul className="cmd-runs__grouplist">
                      {g.runs.map((r) => (
                        <li key={r.runId}>
                          <RunItem
                            run={r}
                            mixed={runsMixed}
                            selected={editing === null && r.runId === runId}
                            onSelect={() => showRun(r.runId)}
                            onRemove={() => remove(r.runId)}
                          />
                        </li>
                      ))}
                    </ul>
                  </li>
                ))}
                {groups.length === 0 && <li className="cmd-runs__none">No runs match</li>}
              </ul>
            </section>
          )}
        </nav>
        <main className="cmd__main">
          {dragging && <DropOverlay />}
          {/* Kept mounted (hidden) so a draft survives looking at a run. */}
          <div className="cmd__view" hidden={selected !== null || editing !== null}>
            {options && (
              <NewTask
                options={options}
                backends={backends}
                focusTick={selected === null ? focusTick : -1}
                active={selected === null && editing === null}
                dropInto={dropInto}
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
                dropInto={dropInto}
                picking={picking}
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
                mixed={mixedAgents(asks.asks)}
              />
            </div>
          )}
          {editing && options && (
            <div className="cmd__view">
              <RoutineEditor
                key={editing.key}
                options={options}
                backends={backends}
                initial={editing.initial}
                followDefault={editing.followDefault}
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
function AskItem({ ask, selected, onSelect }: { ask: SessionApprovalActivity; selected: boolean; onSelect: () => void }) {
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

/** A run's row: the row selects it; an ended run also gets a remove button (a sibling, shown on hover or focus). */
function RunItem({
  run,
  mixed,
  selected,
  onSelect,
  onRemove,
}: {
  run: AgentRun;
  mixed: boolean;
  selected: boolean;
  onSelect: () => void;
  onRemove: () => void;
}) {
  const live = isLive(run.status);
  const status = run.status === "waiting" ? "Needs approval" : live ? "Working" : endLabel(run.status, run.reason);
  const name = run.routine?.name ?? run.project;
  return (
    <div className="cmd-runitem" data-removable={!live}>
      <button className="cmd-run" data-status={run.status} aria-pressed={selected} onClick={onSelect} title={run.cwd}>
        <span className="cmd-run__line">
          <span className="cmd-run__mark">
            {run.status === "working" || run.status === "starting" ? <WorkingIndicator small /> : <RunDot status={run.status} />}
          </span>
          <span className="cmd-run__project">
            {run.routine?.trigger === "schedule" && <ClockGlyph />}
            {name}
          </span>
          <AgentTag agent={run.backend} mixed={mixed} />
          <span className="cmd-run__time">{ago(run.updatedAtMs)}</span>
        </span>
        <span className="cmd-run__detail">
          <span className="cmd-run__status">{status}</span>
          <span className="cmd-run__prompt">{run.prompt}</span>
        </span>
      </button>
      {!live && (
        <button className="cmd-run__remove" aria-label={`Remove ${name}`} title="Remove from list" onClick={onRemove}>
          <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.5} strokeLinecap="round" aria-hidden="true">
            <path d="M5 5l6 6M11 5l-6 6" />
          </svg>
        </button>
      )}
    </div>
  );
}
