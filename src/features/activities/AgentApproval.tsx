import { useState } from "react";
import type { AgentApprovalActivity, AgentRisk } from "../../lib/types";
import { ChevronGlyph, RiskGlyph } from "../agent/AgentGlyphs";
import { RISK_LABEL } from "../agent/labels";
import { changePreview, PreviewBlock, type ChangePreview } from "../agent/PreviewBlock";
import { decide, openRun } from "../agent/useAgentRuns";
import { AgentMark } from "../claude/WorkingGlyphs";
import "../agent/agent.css";

export type ApprovalChoice = "allow" | "deny" | "run";

/**
 * An agent run asks to use a tool: mark · project (or routine) · tool · risk, the command or path on its own line, or
 * its diff / wrapped command (click it for the command window), then Deny / Allow and, where Rust offers it, "Allow … for this run". Sticky: Rust withdraws
 * it once answered, so a click only marks the choice as pending. Interactive while hovered (Rust's hit test).
 */
export function AgentApproval({ approval }: { approval: AgentApprovalActivity }) {
  const { requestId, runId, project, routineName, tool, summary, risk, rememberAs, desktop } = approval;
  const [chosen, setChosen] = useState<ApprovalChoice | null>(null);

  const answer = (choice: ApprovalChoice) => {
    setChosen(choice);
    decide(requestId, choice === "deny" ? "deny" : "allow", choice === "run" ? "run" : "once").catch(
      (err: unknown) => {
        console.debug("agent decide", err);
        setChosen(null);
      },
    );
  };

  return (
    <ApprovalCard
      title={routineName ?? project}
      tool={tool}
      risk={risk}
      desktop={desktop}
      summary={summary}
      preview={changePreview(approval.preview)}
      summaryHint="Details"
      onSummary={() => openRun(runId)}
      rememberAs={rememberAs}
      chosen={chosen}
      onAnswer={answer}
    />
  );
}

/**
 * The approval card both kinds share (agent runs and terminal sessions, 2.8a). Presentational: the caller owns the
 * pending choice and what a click does. `origin` is a quiet word after the title that tells the kinds apart;
 * `onTitle` makes the title a quiet button; without `onSummary` the summary is plain text. With a `preview` (2.8b)
 * the well holds the diff or the wrapped command instead of the one line (the island widens for it, `data-wide`).
 */
export function ApprovalCard({
  title,
  titleHint,
  onTitle,
  origin,
  tool,
  risk,
  desktop,
  summary,
  preview,
  summaryHint,
  onSummary,
  rememberAs,
  chosen,
  onAnswer,
}: {
  title: string;
  titleHint?: string;
  onTitle?: () => void;
  origin?: string;
  tool: string;
  risk: AgentRisk;
  desktop: boolean;
  summary: string;
  preview?: ChangePreview | null;
  summaryHint?: string;
  onSummary?: () => void;
  rememberAs?: string | null;
  chosen: ApprovalChoice | null;
  onAnswer: (choice: ApprovalChoice) => void;
}) {
  const busy = chosen !== null;

  return (
    <div className="activity activity--approval" data-risk={risk}>
      <ApprovalHead
        title={title}
        titleHint={titleHint}
        onTitle={onTitle}
        origin={origin}
        tool={tool}
        risk={risk}
        desktop={desktop}
      />
      {preview && onSummary ? (
        <button
          className="approval__summary approval__preview"
          title={summaryHint ? `${summary}\n${summaryHint}` : summary}
          onClick={onSummary}
        >
          <PreviewBlock preview={preview} />
          <ChevronGlyph />
        </button>
      ) : preview ? (
        <div className="approval__summary approval__preview" data-static="true" title={summary}>
          <PreviewBlock preview={preview} />
        </div>
      ) : onSummary ? (
        <button
          className="approval__summary"
          title={summaryHint ? `${summary}\n${summaryHint}` : summary}
          onClick={onSummary}
        >
          <span className="approval__command">{summary}</span>
          <ChevronGlyph />
        </button>
      ) : (
        <div className="approval__summary" data-static="true" title={summary}>
          <span className="approval__command">{summary}</span>
        </div>
      )}
      <div className="approval__actions">
        {rememberAs && (
          <button className="agent-link" disabled={busy} data-chosen={chosen === "run"} onClick={() => onAnswer("run")}>
            Allow {rememberAs} for this run
          </button>
        )}
        <span className="approval__buttons">
          <button
            className="agent-btn agent-btn--quiet"
            disabled={busy}
            data-chosen={chosen === "deny"}
            onClick={() => onAnswer("deny")}
          >
            Deny
          </button>
          <button
            className="agent-btn agent-btn--allow"
            disabled={busy}
            data-chosen={chosen === "allow"}
            onClick={() => onAnswer("allow")}
          >
            Allow
          </button>
        </span>
      </div>
    </div>
  );
}

/**
 * The approval head (2.8a), shared with the question and plan cards (2.8c): mark · title (a quiet button with
 * `onTitle`) · origin · tool, then `count` ("1/2", questions) and the risk badge (omitted without `risk`).
 */
export function ApprovalHead({
  title,
  titleHint,
  onTitle,
  origin,
  tool,
  count,
  risk,
  desktop = false,
}: {
  title: string;
  titleHint?: string;
  onTitle?: () => void;
  origin?: string;
  tool: string;
  count?: string;
  risk?: AgentRisk;
  desktop?: boolean;
}) {
  return (
    <div className="approval__head">
      <AgentMark />
      <span className="approval__title">
        {onTitle ? (
          <button className="activity__title approval__title-btn" title={titleHint} onClick={onTitle}>
            {title}
          </button>
        ) : (
          <span className="activity__title">{title}</span>
        )}
        {origin && <span className="approval__origin">{origin}</span>}
        <span className="approval__tool">{tool}</span>
        {count && <span className="approval__count">{count}</span>}
      </span>
      {risk && (
        <span className="agent-risk" data-risk={risk}>
          <RiskGlyph desktop={desktop} risk={risk} />
          {RISK_LABEL[risk]}
        </span>
      )}
    </div>
  );
}
