import { useEffect, useRef, useState, type FormEvent, type KeyboardEvent, type MutableRefObject } from "react";
import { commands, errorMessage } from "../lib/ipc";
import type { AgentOptions } from "../lib/types";
import type { RoutineSeed } from "./routineLabels";
import { defaultLimits, FolderField, LimitsField, useFolders, type Limits } from "./TaskFields";

/**
 * The new-task form: the prompt, the folder it runs in (recent folders, or Browse…), and the run's limits behind
 * an Options disclosure. Ctrl+Enter or Run starts it; "Save as routine" opens the routine editor with it.
 */
export function NewTask({
  options,
  focusTick,
  picking,
  onStarted,
  onSaveAsRoutine,
}: {
  options: AgentOptions;
  /** Changes when the window gains focus while this view shows; -1 while hidden. */
  focusTick: number;
  picking: MutableRefObject<boolean>;
  onStarted: (runId: string) => void;
  onSaveAsRoutine: (seed: RoutineSeed) => void;
}) {
  const [prompt, setPrompt] = useState("");
  const { folders, cwd, setCwd, add } = useFolders(options);
  const [limits, setLimits] = useState<Limits>(() => defaultLimits(options));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const promptRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    if (focusTick >= 0) promptRef.current?.focus();
  }, [focusTick]);

  const canRun = prompt.trim() !== "" && cwd !== null && !busy;
  const turns = Number.parseInt(limits.maxTurns, 10);
  const usd = Number.parseFloat(limits.budget);

  const submit = async (e?: FormEvent) => {
    e?.preventDefault();
    if (!canRun || cwd === null) return;
    setBusy(true);
    setError(null);
    try {
      const { runId } = await commands.agentStart({
        prompt: prompt.trim(),
        cwd,
        model: limits.model || null,
        maxTurns: Number.isFinite(turns) && turns > 0 ? turns : null,
        maxBudgetUsd: Number.isFinite(usd) && usd > 0 ? usd : null,
        claudePath: limits.claudePath || null,
      });
      setPrompt("");
      onStarted(runId);
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  };

  const saveAsRoutine = () =>
    onSaveAsRoutine({
      prompt: prompt.trim(),
      cwd: cwd ?? undefined,
      model: limits.model || null,
      maxTurns: Number.isFinite(turns) && turns > 0 ? turns : null,
      maxBudgetUsd: Number.isFinite(usd) && usd > 0 ? usd : null,
      claudePath: limits.claudePath || null,
    });

  const onPromptKey = (e: KeyboardEvent) => {
    if (e.key === "Enter" && e.ctrlKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      void submit();
    }
  };

  return (
    <form className="task" onSubmit={(e) => void submit(e)}>
      <textarea
        ref={promptRef}
        className="task__prompt"
        placeholder="What should Claude do?"
        value={prompt}
        autoFocus
        spellCheck={false}
        onChange={(e) => setPrompt(e.target.value)}
        onKeyDown={onPromptKey}
      />

      <FolderField
        folders={folders}
        cwd={cwd}
        picking={picking}
        onSelect={setCwd}
        onBrowsed={(path) => {
          if (path) add(path);
          promptRef.current?.focus();
        }}
        onError={setError}
      />

      <LimitsField options={options} limits={limits} onChange={setLimits} />

      <footer className="task__footer">
        {error && (
          <span className="cmd-error" role="alert">
            {error}
          </span>
        )}
        <span className="task__hint">Ctrl+Enter</span>
        <button
          type="button"
          className="cmd-btn cmd-btn--quiet"
          disabled={prompt.trim() === ""}
          title="Save this task with a schedule"
          onClick={saveAsRoutine}
        >
          Save as routine
        </button>
        <button type="submit" className="cmd-btn cmd-btn--primary" disabled={!canRun}>
          {busy ? "Starting…" : "Run"}
        </button>
      </footer>
    </form>
  );
}
