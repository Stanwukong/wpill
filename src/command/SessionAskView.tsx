import { useState } from "react";
import { commands, errorMessage } from "../lib/ipc";
import type { SessionApprovalActivity } from "../lib/types";
import { RISK_LABEL } from "../features/agent/labels";
import { changePreview, PreviewBlock } from "../features/agent/PreviewBlock";
import { AgentTag } from "../features/sessions/AgentTag";
import { decideSession, focusSession } from "../features/sessions/useSessions";
import { AgentMark } from "../features/sessions/WorkingGlyphs";
import { PlanView, QuestionForm } from "./InteractionViews";
import { StateLine } from "./StateLine";
import { useWholePreview } from "./useSessionAsks";

/**
 * A terminal session's held request (2.8b): who asks, the one line, the whole diff or command, Deny / Allow; or its
 * question or plan (2.8c, `QuestionForm` / `PlanView`). Once it leaves Rust's list (answered here or in the terminal,
 * or handed back at 55 s) the view stays on what was asked and the buttons give way to one quiet line. Keyed by request, so the last-seen copy never crosses requests.
 */
export function SessionAskView({
  ask,
  loaded,
  mixed,
}: {
  ask: SessionApprovalActivity | undefined;
  loaded: boolean;
  /** Some listed ask isn't Claude Code's: the head names this one's agent (2.9a). */
  mixed: boolean;
}) {
  // The last copy Rust listed: kept so the view can still say what was asked after it's gone.
  const [known, setKnown] = useState(ask);
  if (ask && ask !== known) setKnown(ask);
  const waiting = ask !== undefined;

  if (!known) {
    return (
      <div className="run">
        <div className="run__head" />
        {loaded ? (
          <p className="ask__gone ask__gone--alone">No longer waiting: answered, or back in the terminal.</p>
        ) : (
          <StateLine tone="loading">Loading…</StateLine>
        )}
      </div>
    );
  }
  return <AskBody ask={known} waiting={waiting} mixed={mixed} />;
}

function AskBody({ ask, waiting, mixed }: { ask: SessionApprovalActivity; waiting: boolean; mixed: boolean }) {
  const { requestId, sessionId, agent, project, tool, summary, risk } = ask;
  const preview = changePreview(useWholePreview(requestId, ask.preview, commands.sessionAskPreview));
  const [sent, setSent] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const showSession = sessionId !== null && (
    <button className="agent-link" onClick={() => focusSession(sessionId)}>
      Show session
    </button>
  );

  const answer = (choice: "allow" | "deny") => {
    setSent(true);
    setError(null);
    decideSession(requestId, choice).catch((err: unknown) => {
      setSent(false);
      setError(errorMessage(err));
    });
  };

  return (
    <div className="run">
      <div className="run__head ask__head">
        <AgentMark />
        <span className="run__project" title={project}>
          {project}
        </span>
        <AgentTag agent={agent} mixed={mixed} />
        <span className="ask__origin">session</span>
        <span className="ask__tool" title={tool}>
          {tool}
        </span>
        <span className="agent-risk" data-risk={risk}>
          {RISK_LABEL[risk]}
        </span>
      </div>
      {ask.interaction ? (
        <div className="ask">
          {ask.interaction.kind === "question" ? (
            <QuestionForm
              interaction={ask.interaction}
              waiting={waiting}
              lead={showSession}
              onAnswer={(answer) => commands.sessionAnswer(requestId, answer)}
              onSkip={() => decideSession(requestId, "deny")}
            />
          ) : (
            <PlanView
              requestId={requestId}
              fetch={commands.sessionAskPreview}
              cutNote="Cut on the way to Cay; the terminal shows it whole."
              waiting={waiting}
              lead={showSession}
              onPlan={(approve, feedback) => commands.sessionPlan(requestId, approve, feedback)}
              onReject={() => decideSession(requestId, "deny")}
            />
          )}
        </div>
      ) : (
        <div className="ask">
          <p className="ask__summary">{summary}</p>
          {preview && (
            <div className="cmd-preview ask__preview">
              <PreviewBlock preview={preview} />
            </div>
          )}
          <div className="ask__footer">
            {showSession}
            {waiting ? (
              <>
                {error ? (
                  <span className="cmd-error" role="alert" title={error}>
                    {error}
                  </span>
                ) : (
                  <span className="cmd__spacer" />
                )}
                <button className="agent-btn agent-btn--quiet" disabled={sent} onClick={() => answer("deny")}>
                  Deny
                </button>
                <button className="agent-btn agent-btn--allow" disabled={sent} onClick={() => answer("allow")}>
                  Allow
                </button>
              </>
            ) : (
              <span className="ask__gone">No longer waiting: answered, or back in the terminal.</span>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
