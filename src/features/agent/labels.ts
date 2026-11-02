import type { AgentEndReason, AgentMode, AgentRisk, AgentRun, AgentStatus } from "../../lib/types";

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

/** Permission modes (2.8e), in the New task picker's order (Auto, the default, first). */
export const MODES: AgentMode[] = ["auto", "default", "acceptEdits", "plan"];

export const MODE_LABEL: Record<AgentMode, string> = {
  auto: "Auto",
  default: "Ask",
  acceptEdits: "Accept edits",
  plan: "Plan",
};

/** The picker's line under the choice. */
export const MODE_HINT: Record<AgentMode, string> = {
  auto: "Claude Code's classifier approves everyday steps and blocks risky ones; questions still come to you.",
  default: "Every step Claude Code would ask about waits for you.",
  acceptEdits: "File edits and simple file commands in the folder go ahead; everything else asks.",
  plan: "Claude plans first and asks you to approve the plan before changing anything.",
};

/**
 * A terminal session's mode as a tag (2.8e), or `null` for plain `default` and anything unknown. Sessions can also
 * be in modes Cay never offers (the user's own terminal): those get their own words.
 */
export function sessionModeLabel(mode: string | null): string | null {
  switch (mode) {
    case "auto":
    case "acceptEdits":
    case "plan":
      return MODE_LABEL[mode];
    case "bypassPermissions":
      return "Bypass";
    case "dontAsk":
      return "Don't ask";
    default:
      return null;
  }
}
