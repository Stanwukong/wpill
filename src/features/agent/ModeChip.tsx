import { useState } from "react";
import type { AgentRun } from "../../lib/types";
import { MODE_LABEL, sessionModeLabel } from "./labels";
import { cycleMode } from "./useAgentRuns";
import "./agent.css";

/**
 * A run's permission mode (2.8e) as a small chip; a click asks Rust for the next one (Ask → Accept edits → Plan →
 * Auto). Rust owns the mode: the chip only stays disabled until the next `agent:runs` (a new `run` object) or a
 * failed call. A mode chosen while no process runs (`nextMode`) shows dashed: it applies with your next message.
 */
export function ModeChip({ run }: { run: AgentRun }) {
  /** The run as it was when clicked: pending until Rust sends it again. */
  const [clickedOn, setClickedOn] = useState<AgentRun | null>(null);
  const pending = clickedOn === run;
  const next = run.nextMode !== null && run.nextMode !== run.mode;
  const shown = next && run.nextMode ? run.nextMode : run.mode;
  const label = MODE_LABEL[shown];
  const title = next
    ? `${label} from your next message (now ${MODE_LABEL[run.mode]})\nClick for the next mode`
    : `Permission mode: ${label}\nClick for the next mode`;

  return (
    <button
      type="button"
      className="mode-chip"
      data-mode={shown}
      data-next={next}
      disabled={pending}
      title={title}
      aria-label={next ? `Mode ${label}, from your next message` : `Mode ${label}`}
      onClick={(e) => {
        e.stopPropagation();
        setClickedOn(run);
        cycleMode(run.runId).catch((err: unknown) => {
          console.debug("agent mode", err);
          setClickedOn(null);
        });
      }}
    >
      {label}
    </button>
  );
}

/** A terminal session's mode, read-only (Cay never changes it); nothing for plain `default` or unknown modes. */
export function SessionModeTag({ mode }: { mode: string | null }) {
  const label = sessionModeLabel(mode);
  if (!label) return null;
  return (
    <span className="mode-chip" data-mode={mode} data-static="true" title={`Permission mode: ${label}`}>
      {label}
    </span>
  );
}
