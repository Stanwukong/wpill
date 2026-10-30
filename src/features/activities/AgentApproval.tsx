import { useState } from "react";
import type { AgentApprovalActivity } from "../../lib/types";
import { ChevronGlyph, EyeGlyph } from "../agent/AgentGlyphs";
import { RISK_LABEL } from "../agent/labels";
import { decide, openRun } from "../agent/useAgentRuns";
import { ClaudeMark } from "../claude/ClaudeGlyphs";
import "../agent/agent.css";

type Choice = "allow" | "deny" | "run";

/**
 * An agent run asks to use a tool: ✻ · project · tool · risk, the command or path on its own line (click it for
 * the command window), then Deny / Allow and, where Rust offers it, "Allow … for this run". Sticky: Rust withdraws
 * it once answered, so a click only marks the choice as pending. Interactive while hovered (Rust's hit test).
 */
export function AgentApproval({ approval }: { approval: AgentApprovalActivity }) {
  const { requestId, runId, project, tool, summary, risk, rememberAs, desktop } = approval;
  const [chosen, setChosen] = useState<Choice | null>(null);

  const answer = (choice: Choice) => {
    setChosen(choice);
    decide(requestId, choice === "deny" ? "deny" : "allow", choice === "run" ? "run" : "once").catch(
      (err: unknown) => {
        console.debug("agent decide", err);
        setChosen(null);
      },
    );
  };
  const busy = chosen !== null;

  return (
    <div className="activity activity--approval" data-risk={risk}>
      <div className="approval__head">
        <ClaudeMark />
        <span className="approval__title">
          <span className="activity__title">{project}</span>
          <span className="approval__tool">{tool}</span>
        </span>
        <span className="agent-risk" data-risk={risk}>
          {desktop && <EyeGlyph />}
          {RISK_LABEL[risk]}
        </span>
      </div>
      <button className="approval__summary" title={`${summary}\nDetails`} onClick={() => openRun(runId)}>
        <span className="approval__command">{summary}</span>
        <ChevronGlyph />
      </button>
      <div className="approval__actions">
        {rememberAs && (
          <button className="agent-link" disabled={busy} data-chosen={chosen === "run"} onClick={() => answer("run")}>
            Allow {rememberAs} for this run
          </button>
        )}
        <span className="approval__buttons">
          <button
            className="agent-btn agent-btn--quiet"
            disabled={busy}
            data-chosen={chosen === "deny"}
            onClick={() => answer("deny")}
          >
            Deny
          </button>
          <button
            className="agent-btn agent-btn--allow"
            disabled={busy}
            data-chosen={chosen === "allow"}
            onClick={() => answer("allow")}
          >
            Allow
          </button>
        </span>
      </div>
    </div>
  );
}
