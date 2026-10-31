import { useEffect, useState, type MutableRefObject } from "react";
import { commands, errorMessage } from "../lib/ipc";
import type { AgentOptions } from "../lib/types";

// Form pieces shared by the new-task form and the routine editor: the folder list and the run's limits.

export const MODELS = [
  { value: "", label: "Default" },
  { value: "haiku", label: "Haiku" },
  { value: "sonnet", label: "Sonnet" },
  { value: "opus", label: "Opus" },
];

/** Last path segment ("C:\\code\\wpill" → "wpill"). */
export function folderName(path: string): string {
  const parts = path.split(/[\\/]+/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

/**
 * Recent folders plus any browsed one (kept on top until Rust reports it too), and the chosen folder.
 * `initial` (a routine's folder) is listed even if it isn't recent.
 */
export function useFolders(options: AgentOptions, initial: string | null = null) {
  const [folders, setFolders] = useState(() =>
    initial && !options.recentFolders.includes(initial) ? [initial, ...options.recentFolders] : options.recentFolders,
  );
  const [cwd, setCwd] = useState<string | null>(initial ?? options.recentFolders[0] ?? null);

  useEffect(() => {
    setFolders((current) => {
      const extra = current.filter((f) => !options.recentFolders.includes(f));
      return [...extra, ...options.recentFolders];
    });
    setCwd((current) => current ?? options.recentFolders[0] ?? null);
  }, [options.recentFolders]);

  const add = (path: string) => {
    setFolders((current) => [path, ...current.filter((f) => f !== path)]);
    setCwd(path);
  };

  return { folders, cwd, setCwd, add };
}

/** Folder: recent folders as a list, and Browse… (the shell's picker; Esc belongs to it while it's open). */
export function FolderField({
  folders,
  cwd,
  picking,
  onSelect,
  onBrowsed,
  onError,
}: {
  folders: string[];
  cwd: string | null;
  picking: MutableRefObject<boolean>;
  onSelect: (path: string) => void;
  /** A folder was picked (`null`: cancelled); also where focus goes back. */
  onBrowsed: (path: string | null) => void;
  onError: (message: string) => void;
}) {
  const browse = async () => {
    picking.current = true;
    let path: string | null = null;
    try {
      path = await commands.agentPickFolder();
    } catch (err) {
      onError(errorMessage(err));
    } finally {
      picking.current = false;
      onBrowsed(path);
    }
  };

  return (
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
              onClick={() => onSelect(f)}
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
  );
}

/** A run's limits as the form holds them (strings, so a field can be empty while typing). */
export interface Limits {
  model: string;
  maxTurns: string;
  budget: string;
  claudePath: string;
}

export function defaultLimits(options: AgentOptions): Limits {
  return {
    model: options.model ?? "",
    maxTurns: String(options.maxTurns),
    budget: options.maxBudgetUsd != null ? String(options.maxBudgetUsd) : "",
    claudePath: options.defaultInstall ?? options.installs[0]?.path ?? "",
  };
}

/** Options: model, turns, budget and install behind a disclosure that shows a one-line summary while closed. */
export function LimitsField({
  options,
  limits,
  onChange,
  budgetRequired = false,
}: {
  options: AgentOptions;
  limits: Limits;
  onChange: (limits: Limits) => void;
  /** Routines always have a budget ($0.05–$20). */
  budgetRequired?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const { model, maxTurns, budget, claudePath } = limits;
  const set = (patch: Partial<Limits>) => onChange({ ...limits, ...patch });
  const models = MODELS.some((m) => m.value === model) ? MODELS : [...MODELS, { value: model, label: model }];
  const missingBudget = budgetRequired && !(Number.parseFloat(budget) > 0);

  const summary = [
    MODELS.find((m) => m.value === model)?.label.toLowerCase() ?? model,
    maxTurns ? `${maxTurns} turns` : "no turn limit",
    budget ? `$${budget}` : budgetRequired ? "budget needed" : "no budget",
  ].join(" · ");

  return (
    <div className="task__section">
      <button type="button" className="task__disclosure" aria-expanded={open} onClick={() => setOpen((v) => !v)}>
        <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.6} strokeLinecap="round">
          <path d="M6.5 4l4 4-4 4" />
        </svg>
        Options
        {!open && (
          <span className="task__summary" data-missing={missingBudget}>
            {summary}
          </span>
        )}
      </button>
      {open && (
        <div className="task__options">
          <label className="task-field">
            <span>Model</span>
            <select value={model} onChange={(e) => set({ model: e.target.value })}>
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
              placeholder={budgetRequired ? "Default" : "None"}
              onChange={(e) => set({ maxTurns: e.target.value })}
            />
          </label>
          <label className="task-field">
            <span>Budget (USD)</span>
            <input
              type="number"
              min={budgetRequired ? 0.05 : 0}
              max={budgetRequired ? 20 : undefined}
              step={budgetRequired ? 0.05 : 0.5}
              value={budget}
              required={budgetRequired}
              aria-invalid={missingBudget || undefined}
              placeholder={budgetRequired ? "Required" : "None"}
              onChange={(e) => set({ budget: e.target.value })}
            />
          </label>
          <label className="task-field task-field--wide">
            <span>Claude Code</span>
            <select
              value={claudePath}
              disabled={options.installs.length === 0}
              onChange={(e) => set({ claudePath: e.target.value })}
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
  );
}
