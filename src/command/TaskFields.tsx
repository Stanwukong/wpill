import { useEffect, useRef, useState, type MutableRefObject } from "react";
import { commands, errorMessage } from "../lib/ipc";
import type { AgentBackend, AgentBackendInfo, AgentBackendsPayload, AgentMode, AgentOptions } from "../lib/types";
import { BACKEND_LABEL, BACKENDS, MODE_LABEL, MODES, modeHint } from "../features/agent/labels";

// Form pieces shared by the new-task form and the routine editor: the agent, the folder list and the run's limits.

export const MODELS = [
  { value: "", label: "Default" },
  { value: "haiku", label: "Haiku" },
  { value: "sonnet", label: "Sonnet" },
  { value: "opus", label: "Opus" },
];

export function backendInfo(
  backends: AgentBackendsPayload | null,
  backend: AgentBackend,
): AgentBackendInfo | undefined {
  return backends?.backends.find((b) => b.backend === backend);
}

/**
 * The chosen agent (2.9c). `initial` is a saved choice; `null` follows Rust's default once it arrives (if installed),
 * until the user picks one. `onSwitch` runs whenever the agent changes after the first render (the model resets).
 */
export function useBackend(
  backends: AgentBackendsPayload | null,
  initial: AgentBackend | null,
  onSwitch: (backend: AgentBackend) => void,
) {
  const [picked, setPicked] = useState<AgentBackend | null>(initial);
  const fallback = backends && backendInfo(backends, backends.defaultBackend)?.available ? backends.defaultBackend : "claude";
  const backend = picked ?? fallback;
  const shown = useRef(backend);
  useEffect(() => {
    if (shown.current === backend) return;
    shown.current = backend;
    onSwitch(backend);
  }, [backend]);
  return [backend, setPicked] as const;
}

/**
 * Agent: Claude Code, OpenCode or Codex as a segmented choice, like Mode. An agent that isn't installed is disabled,
 * and how to install it sits on the label's line (quiet, cut to one line), so nothing moves when the list arrives.
 */
export function AgentField({
  backend,
  backends,
  onChange,
}: {
  backend: AgentBackend;
  /** `null` until Rust lists the agents: every choice is offered meanwhile. */
  backends: AgentBackendsPayload | null;
  onChange: (backend: AgentBackend) => void;
}) {
  const missing = backends?.backends.filter((b) => !b.available && b.install) ?? [];
  const hint = missing.map((b) => `${BACKEND_LABEL[b.backend]}: ${b.install}`).join(" · ");
  return (
    <div className="task__section">
      <div className="task__label-row">
        <span className="task__label" id="task-agent">
          Agent
        </span>
        {hint && (
          <span className="task__install" title={hint}>
            {hint}
          </span>
        )}
      </div>
      <div className="seg seg--fit" role="radiogroup" aria-labelledby="task-agent">
        {BACKENDS.map((b) => {
          const info = backendInfo(backends, b);
          const off = info !== undefined && !info.available;
          return (
            <button
              key={b}
              type="button"
              role="radio"
              aria-checked={backend === b}
              className="seg__item"
              disabled={off && backend !== b}
              data-missing={off || undefined}
              title={off ? `Not installed${info?.install ? `: ${info.install}` : ""}` : undefined}
              onClick={() => onChange(b)}
            >
              {BACKEND_LABEL[b]}
            </button>
          );
        })}
      </div>
    </div>
  );
}

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

/**
 * Mode (2.8e): Claude Code's permission modes as a segmented choice, with what the chosen one means under it. The
 * hint keeps two lines' room so switching doesn't move the form. Never Bypass.
 */
export function ModeField({
  mode,
  backend,
  onChange,
}: {
  mode: AgentMode;
  backend: AgentBackend;
  onChange: (mode: AgentMode) => void;
}) {
  return (
    <div className="task__section">
      <span className="task__label" id="task-mode">
        Mode
      </span>
      <div className="seg seg--fit" role="radiogroup" aria-labelledby="task-mode">
        {MODES.map((m) => (
          <button
            key={m}
            type="button"
            role="radio"
            aria-checked={mode === m}
            className="seg__item"
            onClick={() => onChange(m)}
          >
            {MODE_LABEL[m]}
          </button>
        ))}
      </div>
      <p className="task__mode-hint">{modeHint(mode, backend)}</p>
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

/** The model a Claude Code task starts with when switching back to it (other agents start at their default). */
export function defaultModel(options: AgentOptions, backend: AgentBackend): string {
  return backend === "claude" ? (options.model ?? "") : "";
}

/** The model choices for an agent: Claude Code's aliases, or the agent's own list after "Default (its model)". */
function modelChoices(backend: AgentBackend, info: AgentBackendInfo | undefined, model: string) {
  const list =
    backend === "claude"
      ? MODELS
      : [
          { value: "", label: info?.defaultModel ? `Default (${info.defaultModel})` : "Default" },
          ...(info?.models ?? []).map((m) => ({ value: m, label: m })),
        ];
  return list.some((m) => m.value === model) ? list : [...list, { value: model, label: model }];
}

/**
 * Options: model, turns, budget and install behind a disclosure that shows a one-line summary while closed. The model
 * follows the agent; turns, budget and the install only apply to Claude Code, so only it shows them (what was typed is
 * kept for switching back).
 */
export function LimitsField({
  options,
  backend,
  info,
  limits,
  onChange,
  budgetRequired = false,
}: {
  options: AgentOptions;
  backend: AgentBackend;
  /** The agent as Rust lists it (its models); missing until the list arrives. */
  info: AgentBackendInfo | undefined;
  limits: Limits;
  onChange: (limits: Limits) => void;
  /** Routines always have a budget ($0.05–$20). */
  budgetRequired?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const { model, maxTurns, budget, claudePath } = limits;
  const set = (patch: Partial<Limits>) => onChange({ ...limits, ...patch });
  const models = modelChoices(backend, info, model);
  const claude = backend === "claude";
  const missingBudget = claude && budgetRequired && !(Number.parseFloat(budget) > 0);

  const summary = claude
    ? [
        MODELS.find((m) => m.value === model)?.label.toLowerCase() ?? model,
        maxTurns ? `${maxTurns} turns` : "no turn limit",
        budget ? `$${budget}` : budgetRequired ? "budget needed" : "no budget",
      ].join(" · ")
    : model || "default";

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
          <label className={claude ? "task-field" : "task-field task-field--model"}>
            <span>Model</span>
            <select value={model} onChange={(e) => set({ model: e.target.value })}>
              {models.map((m) => (
                <option key={m.value} value={m.value}>
                  {m.label}
                </option>
              ))}
            </select>
          </label>
          {claude && (
            <>
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
            </>
          )}
        </div>
      )}
    </div>
  );
}
