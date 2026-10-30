import { useCallback, useEffect, useRef, useState } from "react";
import { commands, on, unlistenAll } from "../lib/ipc";
import type { AgentOptions, AgentRun } from "../lib/types";
import { RunDot } from "../features/agent/AgentGlyphs";
import { ago, endLabel } from "../features/agent/labels";
import { isLive, useAgentRuns } from "../features/agent/useAgentRuns";
import { ClaudeMark, ClaudeSpinner } from "../features/claude/ClaudeGlyphs";
import { NewTask } from "./NewTask";
import { RunView } from "./RunView";
import { useTranscripts } from "./useTranscripts";

/**
 * The command window: a normal, focusable window (the island never takes keyboard input). New-task form or one
 * run's transcript, with the runs listed on the left. Esc hides it (Rust keeps it alive for next time).
 */
export function CommandApp() {
  const runs = useAgentRuns();
  const transcripts = useTranscripts();
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
    const subscriptions = [on.commandFocusRun((runId) => setSelected(runId))];
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
          <button className="cmd-new" aria-pressed={selected === null} onClick={() => setSelected(null)}>
            <span className="cmd-new__plus" aria-hidden="true">
              +
            </span>
            New task
          </button>
          <ul className="cmd-runs">
            {runs.map((r) => (
              <li key={r.runId}>
                <RunItem run={r} selected={r.runId === selected} onSelect={() => setSelected(r.runId)} />
              </li>
            ))}
          </ul>
        </nav>
        <main className="cmd__main">
          {/* Kept mounted (hidden) so a draft survives looking at a run. */}
          <div className="cmd__view" hidden={selected !== null}>
            {options && (
              <NewTask
                options={options}
                focusTick={selected === null ? focusTick : -1}
                picking={picking}
                onStarted={(runId) => {
                  setSelected(runId);
                  refreshOptions();
                }}
              />
            )}
          </div>
          {selected && (
            <div className="cmd__view">
              <RunView
                key={selected}
                runId={selected}
                run={run}
                transcript={transcripts.get(selected)}
                focusTick={focusTick}
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
        <span className="cmd-run__project">{run.project}</span>
        <span className="cmd-run__time">{ago(run.updatedAtMs)}</span>
      </span>
      <span className="cmd-run__detail">
        <span className="cmd-run__status">{status}</span>
        <span className="cmd-run__prompt">{run.prompt}</span>
      </span>
    </button>
  );
}
