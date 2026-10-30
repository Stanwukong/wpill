import { useEffect, useRef, useState, type FormEvent, type KeyboardEvent, type MutableRefObject } from "react";
import { commands, errorMessage } from "../lib/ipc";
import type { AgentOptions } from "../lib/types";

const MODELS = [
  { value: "", label: "Default" },
  { value: "haiku", label: "Haiku" },
  { value: "sonnet", label: "Sonnet" },
  { value: "opus", label: "Opus" },
];

/** Last path segment ("C:\\code\\wpill" → "wpill"). */
function folderName(path: string): string {
  const parts = path.split(/[\\/]+/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

/**
 * The new-task form: the prompt, the folder it runs in (recent folders, or Browse…), and the run's limits behind
 * an Options disclosure. Ctrl+Enter or Run starts it.
 */
export function NewTask({
  options,
  focusTick,
  picking,
  onStarted,
}: {
  options: AgentOptions;
  /** Changes when the window gains focus while this view shows; -1 while hidden. */
  focusTick: number;
  picking: MutableRefObject<boolean>;
  onStarted: (runId: string) => void;
}) {
  const [prompt, setPrompt] = useState("");
  const [folders, setFolders] = useState(options.recentFolders);
  const [cwd, setCwd] = useState<string | null>(options.recentFolders[0] ?? null);
  const [showOptions, setShowOptions] = useState(false);
  const [model, setModel] = useState(options.model ?? "");
  const [maxTurns, setMaxTurns] = useState(String(options.maxTurns));
  const [budget, setBudget] = useState(options.maxBudgetUsd != null ? String(options.maxBudgetUsd) : "");
  const [claudePath, setClaudePath] = useState(options.defaultInstall ?? options.installs[0]?.path ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const promptRef = useRef<HTMLTextAreaElement>(null);

  // Recent folders change after a run starts; keep a browsed folder on top until Rust reports it too.
  useEffect(() => {
    setFolders((current) => {
      const extra = current.filter((f) => !options.recentFolders.includes(f));
      return [...extra, ...options.recentFolders];
    });
    setCwd((current) => current ?? options.recentFolders[0] ?? null);
  }, [options.recentFolders]);

  useEffect(() => {
    if (focusTick >= 0) promptRef.current?.focus();
  }, [focusTick]);

  const models = MODELS.some((m) => m.value === model) ? MODELS : [...MODELS, { value: model, label: model }];
  const canRun = prompt.trim() !== "" && cwd !== null && !busy;

  const browse = async () => {
    picking.current = true;
    try {
      const path = await commands.agentPickFolder();
      if (path) {
        setFolders((current) => [path, ...current.filter((f) => f !== path)]);
        setCwd(path);
      }
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      picking.current = false;
      promptRef.current?.focus();
    }
  };

  const submit = async (e?: FormEvent) => {
    e?.preventDefault();
    if (!canRun || cwd === null) return;
    const turns = Number.parseInt(maxTurns, 10);
    const usd = Number.parseFloat(budget);
    setBusy(true);
    setError(null);
    try {
      const { runId } = await commands.agentStart({
        prompt: prompt.trim(),
        cwd,
        model: model || null,
        maxTurns: Number.isFinite(turns) && turns > 0 ? turns : null,
        maxBudgetUsd: Number.isFinite(usd) && usd > 0 ? usd : null,
        claudePath: claudePath || null,
      });
      setPrompt("");
      onStarted(runId);
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  };

  const onPromptKey = (e: KeyboardEvent) => {
    if (e.key === "Enter" && e.ctrlKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      void submit();
    }
  };

  const summary = [
    MODELS.find((m) => m.value === model)?.label.toLowerCase() ?? model,
    maxTurns ? `${maxTurns} turns` : "no turn limit",
    budget ? `$${budget}` : "no budget",
  ].join(" · ");

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

      <div className="task__section">
        <span className="task__label">Folder</span>
        <ul className="task__folders" role="listbox" aria-label="Folder">
          {folders.map((f) => (
            <li key={f}>
              <button
                type="button"
                role="option"
                aria-selected={f === cwd}
                className="task-folder"
                onClick={() => setCwd(f)}
              >
                <span className="task-folder__name">{folderName(f)}</span>
                <span className="task-folder__path">{f}</span>
              </button>
            </li>
          ))}
          {folders.length === 0 && <li className="task__empty">No recent folders</li>}
        </ul>
        <button type="button" className="cmd-btn cmd-btn--quiet task__browse" onClick={() => void browse()}>
          Browse…
        </button>
      </div>

      <div className="task__section">
        <button
          type="button"
          className="task__disclosure"
          aria-expanded={showOptions}
          onClick={() => setShowOptions((v) => !v)}
        >
          <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.6} strokeLinecap="round">
            <path d="M6.5 4l4 4-4 4" />
          </svg>
          Options
          {!showOptions && <span className="task__summary">{summary}</span>}
        </button>
        {showOptions && (
          <div className="task__options">
            <label className="task-field">
              <span>Model</span>
              <select value={model} onChange={(e) => setModel(e.target.value)}>
                {models.map((m) => (
                  <option key={m.value} value={m.value}>
                    {m.label}
                  </option>
                ))}
              </select>
            </label>
            <label className="task-field">
              <span>Max turns</span>
              <input
                type="number"
                min={1}
                value={maxTurns}
                placeholder="None"
                onChange={(e) => setMaxTurns(e.target.value)}
              />
            </label>
            <label className="task-field">
              <span>Budget (USD)</span>
              <input
                type="number"
                min={0}
                step={0.5}
                value={budget}
                placeholder="None"
                onChange={(e) => setBudget(e.target.value)}
              />
            </label>
            <label className="task-field task-field--wide">
              <span>Claude Code</span>
              <select
                value={claudePath}
                disabled={options.installs.length === 0}
                onChange={(e) => setClaudePath(e.target.value)}
              >
                {options.installs.length === 0 && <option value="">Not found</option>}
                {options.installs.map((i) => (
                  <option key={i.path} value={i.path} title={i.path}>
                    {`${i.version ?? "unknown version"} · ${i.source}`}
                  </option>
                ))}
              </select>
            </label>
          </div>
        )}
      </div>

      <footer className="task__footer">
        {error && (
          <span className="cmd-error" role="alert">
            {error}
          </span>
        )}
        <span className="task__hint">Ctrl+Enter</span>
        <button type="submit" className="cmd-btn cmd-btn--primary" disabled={!canRun}>
          {busy ? "Starting…" : "Run"}
        </button>
      </footer>
    </form>
  );
}
