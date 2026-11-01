import { useState } from "react";
import type { ClaudeApprovalActivity } from "../../lib/types";
import { decideClaude, focusClaudeSession } from "../claude/useClaudeSessions";
import { ApprovalCard, type ApprovalChoice } from "./AgentApproval";

/**
 * A terminal Claude Code session asks to use a tool (2.8a): the agent card with a quiet "session" after the project,
 * no "for this run" (a terminal session isn't Cay's to widen), and the summary focuses the session's window when Cay
 * knows it. Sticky: Rust withdraws it when answered here or in the terminal, or when it hands back at 55 s.
 */
export function ClaudeApproval({ approval }: { approval: ClaudeApprovalActivity }) {
  const { requestId, sessionId, project, tool, summary, risk } = approval;
  const [chosen, setChosen] = useState<ApprovalChoice | null>(null);

  const answer = (choice: ApprovalChoice) => {
    if (choice === "run") return;
    setChosen(choice);
    decideClaude(requestId, choice).catch((err: unknown) => {
      console.debug("claude decide", err);
      setChosen(null);
    });
  };

  return (
    <ApprovalCard
      title={project}
      origin="session"
      tool={tool}
      risk={risk}
      desktop={false}
      summary={summary}
      summaryHint="Show session"
      onSummary={sessionId === null ? undefined : () => focusClaudeSession(sessionId)}
      chosen={chosen}
      onAnswer={answer}
    />
  );
}
