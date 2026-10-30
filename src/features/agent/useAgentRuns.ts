import { useEffect, useState } from "react";
import { commands, on, unlistenAll } from "../../lib/ipc";
import type { AgentRun, AgentStatus } from "../../lib/types";

/**
 * Agent runs as tracked by Rust (most recently started first). Lives at the Island level (like
 * `useClaudeSessions`) so state survives the panel remounting; the command window uses it too.
 */
export function useAgentRuns(): AgentRun[] {
  const [runs, setRuns] = useState<AgentRun[]>([]);

  useEffect(() => {
    let alive = true;
    commands
      .agentRuns()
      .then((r) => alive && setRuns(r.runs))
      .catch(() => {});
    const subscriptions = [on.agentRuns((r) => setRuns(r.runs))];
    return () => {
      alive = false;
      unlistenAll(subscriptions);
    };
  }, []);

  return runs;
}

/** How long an ended run stays in the island's panel. */
const RECENT_MS = 30 * 60_000;

export function isLive(status: AgentStatus): boolean {
  return status === "starting" || status === "working" || status === "waiting";
}

/** Live runs and runs that ended in the last half hour. Computed at render; the panel re-renders on events. */
export function recentRuns(runs: AgentRun[], now = Date.now()): AgentRun[] {
  return runs.filter((r) => isLive(r.status) || now - r.updatedAtMs < RECENT_MS);
}

/** Answers an approval. Rust withdraws the activity and updates the run; failures (already answered) stay quiet. */
export function decide(requestId: number, decision: "allow" | "deny", remember: "once" | "run" = "once") {
  return commands.agentDecide(requestId, decision, remember);
}

/** Opens the command window on a run. */
export function openRun(runId: string) {
  commands.commandWindowShow(runId).catch((err: unknown) => console.debug("command window", err));
}

export function stopRun(runId: string) {
  commands.agentStop(runId).catch((err: unknown) => console.debug("agent stop", err));
}
