import { useState } from "react";
import { commands } from "../../lib/ipc";
import type { SessionApprovalActivity } from "../../lib/types";
import { changePreview } from "../agent/PreviewBlock";
import { decideSession, focusSession } from "../sessions/useSessions";
import { ApprovalCard, type ApprovalChoice } from "./AgentApproval";

/**
 * A terminal Claude Code session asks to use a tool (2.8a): the agent card with a quiet "session" after the project,
 * no "for this run" (a terminal session isn't Cay's to widen). The well (one line, or the diff / command, 2.8b) opens
 * the request in the command window; the project title focuses the session's window when Cay knows it.
 * Sticky: Rust withdraws it when answered here or in the terminal, or when it hands back at 55 s.
 */
export function SessionApproval({ approval }: { approval: SessionApprovalActivity }) {
  const { requestId, sessionId, project, tool, summary, risk, preview } = approval;
  const [chosen, setChosen] = useState<ApprovalChoice | null>(null);

  const answer = (choice: ApprovalChoice) => {
    if (choice === "run") return;
    setChosen(choice);
    decideSession(requestId, choice).catch((err: unknown) => {
      console.debug("session decide", err);
      setChosen(null);
    });
  };

  return (
    <ApprovalCard
      title={project}
      titleHint="Show session"
      onTitle={sessionId === null ? undefined : () => focusSession(sessionId)}
      origin="session"
      tool={tool}
      risk={risk}
      desktop={false}
      summary={summary}
      preview={changePreview(preview)}
      summaryHint="Details"
      onSummary={() =>
        commands.commandWindowShowAsk(requestId).catch((err: unknown) => console.debug("command window", err))
      }
      chosen={chosen}
      onAnswer={answer}
    />
  );
}
