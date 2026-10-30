import type { AgentEndReason, AgentRisk, AgentRun, AgentStatus } from "../../lib/types";

/** How an ended run is described: in the finished activity, the panel, the command window. */
export function endLabel(status: AgentStatus, reason: AgentEndReason | null): string {
  switch (reason) {
    case "completed":
      return "Done";
    case "maxTurns":
      return "Turn limit reached";
    case "maxBudget":
      return "Budget reached";
    case "error":
      return "Failed";
    case "stopped":
      return "Stopped";
  }
  return status === "failed" ? "Failed" : status === "stopped" ? "Stopped" : "Done";
}

/** One status line for a run: its current step while working, otherwise its state. */
export function runLabel(run: AgentRun): string {
  switch (run.status) {
    case "starting":
      return "Starting";
    case "working":
      return run.step ?? "Working";
    case "waiting":
      return "Needs approval";
    default:
      return endLabel(run.status, run.reason);
  }
}

export const RISK_LABEL: Record<AgentRisk, string> = {
  read: "reads",
  change: "changes",
  input: "input",
};

/** "just now", "4 m ago", "2 h ago", "3 d ago". Computed at render; nothing ticks. */
export function ago(atMs: number, now = Date.now()): string {
  const minutes = Math.floor((now - atMs) / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes} m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} h ago`;
  return `${Math.floor(hours / 24)} d ago`;
}
