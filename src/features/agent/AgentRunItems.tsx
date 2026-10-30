import { useState } from "react";
import type { AgentApprovalActivity, AgentRun } from "../../lib/types";
import { ClaudeSpinner } from "../claude/ClaudeGlyphs";
import { RunDot, StopGlyph } from "./AgentGlyphs";
import { ago, runLabel } from "./labels";
import { decide, isLive, openRun, stopRun } from "./useAgentRuns";
import "./agent.css";

/**
 * Agent runs as rows of the panel's Claude Code list (`<li>`s; `ClaudeSection` owns the list). A row opens the
 * command window on its run; Stop ends a live run. Pending approvals sit under their run: the panel is where they
 * are answered while the island is expanded.
 */
export function AgentRunItems({ runs }: { runs: AgentRun[] }) {
  return (
    <>
      {runs.map((run) => (
        <AgentRunItem key={run.runId} run={run} />
      ))}
    </>
  );
}

function AgentRunItem({ run }: { run: AgentRun }) {
  const live = isLive(run.status);
  return (
    <>
      <li>
        <div
          className="claude-row agent-row"
          data-status={run.status}
          role="button"
          title={run.prompt}
          onClick={() => openRun(run.runId)}
        >
          <span className="claude-row__mark">
            {run.status === "working" || run.status === "starting" ? <ClaudeSpinner /> : <RunDot status={run.status} />}
          </span>
          <span className="agent-row__project">{run.project}</span>
          <span className="agent-badge">agent</span>
          <span className="agent-row__status">{runLabel(run)}</span>
          {live ? (
            <button
              className="agent-row__stop"
              aria-label="Stop"
              title="Stop"
              onClick={(e) => {
                e.stopPropagation();
                stopRun(run.runId);
              }}
            >
              <StopGlyph />
            </button>
          ) : (
            <span className="claude-row__time">{ago(run.updatedAtMs)}</span>
          )}
        </div>
      </li>
      {run.pending.map((a) => (
        <li key={a.requestId}>
          <PendingApproval approval={a} />
        </li>
      ))}
    </>
  );
}

function PendingApproval({ approval }: { approval: AgentApprovalActivity }) {
  // Pressed until Rust answers with new runs (this item unmounts); reset if the call fails.
  const [sent, setSent] = useState(false);
  const answer = (decision: "allow" | "deny") => {
    setSent(true);
    decide(approval.requestId, decision).catch(() => setSent(false));
  };
  return (
    <div className="agent-pending" data-risk={approval.risk}>
      <span className="agent-pending__tool">{approval.tool}</span>
      <span className="agent-pending__summary" title={approval.summary}>
        {approval.summary}
      </span>
      <button className="agent-btn agent-btn--quiet" disabled={sent} onClick={() => answer("deny")}>
        Deny
      </button>
      <button className="agent-btn agent-btn--allow" disabled={sent} onClick={() => answer("allow")}>
        Allow
      </button>
    </div>
  );
}
